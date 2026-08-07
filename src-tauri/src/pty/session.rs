use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use secrecy::{ExposeSecret, SecretString};
use tauri::{AppHandle, Emitter};
use zeroize::Zeroize;

use crate::sudo::{SudoConfig, SudoPromptBroker};

use super::{
    backend::PtyError,
    flow::FlowControl,
    model::{PtyErrorEvent, PtyExitEvent, PtyOutputEvent, MAX_CHUNK_BYTES, MAX_UNACKED_BYTES},
};

type CompletionHandler = Arc<dyn Fn(&str) + Send + Sync>;

pub struct PtySession {
    id: String,
    pid: u32,
    started_at: Option<u64>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    flow: Arc<FlowControl>,
    sudo: Mutex<SudoPromptBroker>,
    sequence: AtomicU64,
    dropped_bytes: AtomicU64,
    attached: AtomicBool,
    exited: AtomicBool,
    reader_done: AtomicBool,
    exit_emitted: AtomicBool,
    completed: AtomicBool,
    exit_event: Mutex<Option<PtyExitEvent>>,
    on_completed: CompletionHandler,
}

impl PtySession {
    pub fn new(
        id: String,
        pid: u32,
        started_at: Option<u64>,
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        sudo: Option<SudoConfig>,
        on_completed: CompletionHandler,
    ) -> Self {
        Self {
            id,
            pid,
            started_at,
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            flow: Arc::new(FlowControl::new(MAX_UNACKED_BYTES)),
            sudo: Mutex::new(SudoPromptBroker::new(sudo)),
            sequence: AtomicU64::new(0),
            dropped_bytes: AtomicU64::new(0),
            attached: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            reader_done: AtomicBool::new(false),
            exit_emitted: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            exit_event: Mutex::new(None),
            on_completed,
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn started_at(&self) -> Option<u64> {
        self.started_at
    }

    pub fn is_alive(&self) -> bool {
        !self.exited.load(Ordering::Acquire)
    }

    pub fn can_restore(&self) -> bool {
        !self.exit_emitted.load(Ordering::Acquire)
    }

    pub fn attach(&self, app: &AppHandle) {
        self.attached.store(true, Ordering::Release);
        let dropped = self
            .dropped_bytes
            .swap(0, Ordering::AcqRel)
            .saturating_add(self.flow.attach() as u64);
        if dropped > 0 {
            let _ = app.emit(
                "pty.error",
                PtyErrorEvent {
                    id: self.id.clone(),
                    code: "outputDropped".into(),
                    details: format!(
                        "{dropped} byte(s) of unacknowledged PTY output were discarded while the renderer was detached"
                    ),
                },
            );
        }
        self.maybe_emit_exit(app);
    }

    pub fn detach(&self) {
        self.attached.store(false, Ordering::Release);
        let dropped = self.flow.detach() as u64;
        if dropped > 0 {
            self.dropped_bytes.fetch_add(dropped, Ordering::AcqRel);
        }
        self.complete_if_delivered();
    }

    pub fn ack(&self, bytes: usize) {
        self.flow.ack(bytes);
        self.complete_if_delivered();
    }

    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        if !self.is_alive() {
            return Err(PtyError::Io("PTY session has exited".into()));
        }
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    pub fn claim_sudo(&self, prompt_id: &str) -> Result<Option<String>, PtyError> {
        self.sudo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .claim(prompt_id)
            .map_err(|error| PtyError::Io(error.to_string()))
    }

    pub fn write_sudo_secret(&self, secret: &SecretString) -> Result<(), PtyError> {
        let mut bytes = secret.expose_secret().as_bytes().to_vec();
        bytes.push(b'\n');
        let result = self.write(&bytes);
        bytes.zeroize();
        result
    }

    pub fn resize(&self, columns: u16, rows: u16) -> Result<(), PtyError> {
        if columns == 0 || rows == 0 {
            return Err(PtyError::Io(
                "PTY dimensions must be greater than zero".into(),
            ));
        }
        let master = self
            .master
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        master
            .resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::Io(error.to_string()))
    }

    pub fn kill(&self, signal: Option<&str>) -> Result<(), PtyError> {
        #[cfg(unix)]
        if let Some(signal) = signal {
            let signal = match signal {
                "SIGTERM" => libc::SIGTERM,
                "SIGKILL" => libc::SIGKILL,
                "SIGHUP" => libc::SIGHUP,
                "SIGINT" => libc::SIGINT,
                other => return Err(PtyError::Io(format!("unsupported PTY signal: {other}"))),
            };
            let result = unsafe { libc::kill(self.pid as i32, signal) };
            if result != 0 {
                return Err(PtyError::Io(std::io::Error::last_os_error().to_string()));
            }
            return Ok(());
        }

        #[cfg(not(unix))]
        let _ = signal;

        self.killer
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .kill()
            .map_err(PtyError::from)
    }

    pub fn start(
        self: &Arc<Self>,
        reader: Box<dyn Read + Send>,
        child: Box<dyn Child + Send + Sync>,
        app: AppHandle,
    ) {
        self.start_reader(reader, app.clone());
        self.start_waiter(child, app);
    }

    fn start_reader(self: &Arc<Self>, mut reader: Box<dyn Read + Send>, app: AppHandle) {
        let session = Arc::clone(self);
        thread::spawn(move || {
            let mut buffer = vec![0u8; MAX_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => {
                        if !session.flow.reserve(length) {
                            break;
                        }
                        let sudo_prompt = session
                            .sudo
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .inspect(&session.id, &buffer[..length]);
                        let sequence = session.sequence.fetch_add(1, Ordering::AcqRel);
                        let payload = PtyOutputEvent {
                            id: session.id.clone(),
                            sequence,
                            data: buffer[..length].to_vec(),
                        };
                        if let Err(error) = app.emit("pty.output", payload) {
                            session.flow.ack(length);
                            let _ = app.emit(
                                "pty.error",
                                PtyErrorEvent {
                                    id: session.id.clone(),
                                    code: "eventEmissionFailed".into(),
                                    details: error.to_string(),
                                },
                            );
                            break;
                        }
                        if let Some(prompt) = sudo_prompt {
                            if let Err(error) = app.emit("sudo.prompt", prompt) {
                                let _ = app.emit(
                                    "pty.error",
                                    PtyErrorEvent {
                                        id: session.id.clone(),
                                        code: "sudoPromptEmissionFailed".into(),
                                        details: error.to_string(),
                                    },
                                );
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        if session.is_alive() {
                            let _ = app.emit(
                                "pty.error",
                                PtyErrorEvent {
                                    id: session.id.clone(),
                                    code: "readFailed".into(),
                                    details: error.to_string(),
                                },
                            );
                        }
                        break;
                    }
                }
            }
            session.reader_done.store(true, Ordering::Release);
            session.maybe_emit_exit(&app);
        });
    }

    fn start_waiter(self: &Arc<Self>, mut child: Box<dyn Child + Send + Sync>, app: AppHandle) {
        let session = Arc::clone(self);
        thread::spawn(move || {
            let event = match child.wait() {
                Ok(status) => PtyExitEvent {
                    id: session.id.clone(),
                    exit_code: Some(status.exit_code()),
                    signal: status.signal().map(str::to_owned),
                },
                Err(error) => {
                    let _ = app.emit(
                        "pty.error",
                        PtyErrorEvent {
                            id: session.id.clone(),
                            code: "waitFailed".into(),
                            details: error.to_string(),
                        },
                    );
                    PtyExitEvent {
                        id: session.id.clone(),
                        exit_code: None,
                        signal: None,
                    }
                }
            };
            session.exited.store(true, Ordering::Release);
            *session
                .exit_event
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(event);
            session.maybe_emit_exit(&app);
        });
    }

    fn maybe_emit_exit(&self, app: &AppHandle) {
        if !self.attached.load(Ordering::Acquire)
            || !self.reader_done.load(Ordering::Acquire)
            || self.exit_emitted.load(Ordering::Acquire)
        {
            return;
        }

        let event = self
            .exit_event
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(event) = event else {
            return;
        };

        if self
            .exit_emitted
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        if let Err(error) = app.emit("pty.exit", event) {
            self.exit_emitted.store(false, Ordering::Release);
            let _ = app.emit(
                "pty.error",
                PtyErrorEvent {
                    id: self.id.clone(),
                    code: "exitEmissionFailed".into(),
                    details: error.to_string(),
                },
            );
            return;
        }

        self.flow.close();
        self.complete_if_delivered();
    }

    fn complete_if_delivered(&self) {
        if self.attached.load(Ordering::Acquire)
            || !self.exit_emitted.load(Ordering::Acquire)
            || !self.flow.is_drained()
        {
            return;
        }
        if self
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            (self.on_completed)(&self.id);
        }
    }
}
