use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
};

use portable_pty::{Child, ChildKiller, MasterPty, PtySize};
use tauri::{AppHandle, Emitter};

use super::{
    backend::PtyError,
    flow::FlowControl,
    model::{PtyErrorEvent, PtyExitEvent, PtyOutputEvent, MAX_CHUNK_BYTES, MAX_UNACKED_BYTES},
};

pub struct PtySession {
    id: String,
    pid: u32,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    flow: Arc<FlowControl>,
    sequence: AtomicU64,
    exited: AtomicBool,
}

impl PtySession {
    pub fn new(
        id: String,
        pid: u32,
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        killer: Box<dyn ChildKiller + Send + Sync>,
    ) -> Self {
        Self {
            id,
            pid,
            master: Mutex::new(master),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            flow: Arc::new(FlowControl::new(MAX_UNACKED_BYTES)),
            sequence: AtomicU64::new(0),
            exited: AtomicBool::new(false),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn is_alive(&self) -> bool {
        !self.exited.load(Ordering::Acquire)
    }

    pub fn attach(&self) {
        self.flow.attach();
    }

    pub fn detach(&self) {
        self.flow.detach();
    }

    pub fn ack(&self, bytes: usize) {
        self.flow.ack(bytes);
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
        });
    }

    fn start_waiter(
        self: &Arc<Self>,
        mut child: Box<dyn Child + Send + Sync>,
        app: AppHandle,
    ) {
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
            session.flow.close();
            let _ = app.emit("pty.exit", event);
        });
    }
}
