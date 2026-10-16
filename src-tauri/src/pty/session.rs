use std::{
    io::{Read, Write},
    sync::mpsc,
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
    process::ProcessIdentity,
};

type CompletionHandler = Arc<dyn Fn(&str) + Send + Sync>;

pub struct PtySession {
    id: String,
    pid: u32,
    started_at: Option<u64>,
    pub(super) shell_identity: Mutex<Option<ProcessIdentity>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
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
    stopping: AtomicBool,
    native_handles_closing: AtomicBool,
    #[cfg(unix)]
    stop: Mutex<Option<super::io::UnixStopHandle>>,
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
            shell_identity: Mutex::new(None),
            master: Mutex::new(Some(master)),
            writer: Mutex::new(Some(writer)),
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
            stopping: AtomicBool::new(false),
            native_handles_closing: AtomicBool::new(false),
            #[cfg(unix)]
            stop: Mutex::new(None),
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
        !self.completed.load(Ordering::Acquire) && !self.exited.load(Ordering::Acquire)
    }

    pub fn can_restore(&self) -> bool {
        !self.completed.load(Ordering::Acquire) && !self.exit_emitted.load(Ordering::Acquire)
    }

    pub fn attach<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
        self.attached.store(true, Ordering::Release);
        let dropped = self
            .dropped_bytes
            .swap(0, Ordering::AcqRel)
            .saturating_add(self.flow.attach() as u64);
        if dropped > 0 {
            let _ = app.emit(
                "pty:error",
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
        let writer = writer
            .as_mut()
            .ok_or_else(|| PtyError::Io("PTY writer is closed".into()))?;
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
        let master = master
            .as_ref()
            .ok_or_else(|| PtyError::Io("PTY master is closed".into()))?;
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

    pub fn discard(&self) -> Result<(), PtyError> {
        let _killer = self
            .killer
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.completed.load(Ordering::Acquire) {
            return Ok(());
        }
        if !self.exited.load(Ordering::Acquire) {
            #[cfg(unix)]
            {
                let result = unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGKILL) };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(PtyError::Io(error.to_string()));
                    }
                }
            }

            #[cfg(windows)]
            terminate_process(self.pid)?;
        }

        self.attached.store(false, Ordering::Release);
        let completed = self
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        self.begin_stop_io();
        self.flow.close();
        self.finish_stop_io();
        if completed {
            (self.on_completed)(&self.id);
        }
        Ok(())
    }

    pub fn start<R: tauri::Runtime>(
        self: &Arc<Self>,
        reader: Box<dyn Read + Send>,
        child: Box<dyn Child + Send + Sync>,
        app: AppHandle<R>,
    ) {
        self.start_reader(reader, app.clone());
        self.start_waiter(child, app);
    }

    #[cfg(unix)]
    pub fn install_stop(&self, stop: super::io::UnixStopHandle) {
        *self.stop.lock().unwrap_or_else(|error| error.into_inner()) = Some(stop);
    }

    fn begin_stop_io(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    fn finish_stop_io(&self) {
        #[cfg(unix)]
        {
            if let Some(stop) = self
                .stop
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
            {
                stop.cancel();
            }
        }
        if self
            .native_handles_closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        #[cfg(windows)]
        {
            use std::sync::TryLockError;

            let (writer, busy) = match self.writer.try_lock() {
                Ok(mut guard) => (guard.take(), false),
                Err(TryLockError::Poisoned(error)) => (error.into_inner().take(), false),
                Err(TryLockError::WouldBlock) => (None, true),
            };
            let master = self
                .master
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            drop(writer);
            drop(master);
            if busy {
                let writer = self
                    .writer
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                drop(writer);
            }
        }
        #[cfg(not(windows))]
        {
            let writer = self
                .writer
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            let master = self
                .master
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            drop(writer);
            drop(master);
        }
    }

    fn start_reader<R: tauri::Runtime>(
        self: &Arc<Self>,
        mut reader: Box<dyn Read + Send>,
        app: AppHandle<R>,
    ) {
        let (sender, receiver) = mpsc::sync_channel::<std::io::Result<Vec<u8>>>(1);
        thread::spawn(move || {
            let mut buffer = vec![0u8; MAX_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => {
                        if sender.send(Ok(buffer[..length].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });

        let session = Arc::clone(self);
        thread::spawn(move || {
            loop {
                let message = receiver.recv();
                if session.completed.load(Ordering::Acquire) {
                    if message.is_err() {
                        break;
                    }
                    continue;
                }

                match message {
                    Ok(Ok(data)) => {
                        let length = data.len();
                        if !session.flow.reserve(length) {
                            if session.completed.load(Ordering::Acquire) {
                                continue;
                            }
                            break;
                        }
                        if !session.emit_reserved_output(&app, &data) {
                            drop(receiver);
                            session.begin_stop_io();
                            session.flow.close();
                            session.finish_stop_io();
                            break;
                        }
                    }
                    Ok(Err(error)) => {
                        if session.is_alive() && !session.stopping.load(Ordering::Acquire) {
                            let _ = app.emit(
                                "pty:error",
                                PtyErrorEvent {
                                    id: session.id.clone(),
                                    code: "readFailed".into(),
                                    details: error.to_string(),
                                },
                            );
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
            session.reader_done.store(true, Ordering::Release);
            session.maybe_emit_exit(&app);
        });
    }

    fn emit_reserved_output<R: tauri::Runtime>(&self, app: &AppHandle<R>, data: &[u8]) -> bool {
        let length = data.len();
        let sudo_prompt = self
            .sudo
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .inspect(&self.id, data);
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel);
        let payload = PtyOutputEvent {
            id: self.id.clone(),
            sequence,
            data: data.to_vec(),
        };
        if let Err(error) = app.emit("pty:output", payload) {
            self.flow.ack(length);
            let _ = app.emit(
                "pty:error",
                PtyErrorEvent {
                    id: self.id.clone(),
                    code: "eventEmissionFailed".into(),
                    details: error.to_string(),
                },
            );
            return false;
        }
        if let Some(prompt) = sudo_prompt {
            if let Err(error) = app.emit("sudo:prompt", prompt) {
                let _ = app.emit(
                    "pty:error",
                    PtyErrorEvent {
                        id: self.id.clone(),
                        code: "sudoPromptEmissionFailed".into(),
                        details: error.to_string(),
                    },
                );
            }
        }
        true
    }

    fn start_waiter<R: tauri::Runtime>(
        self: &Arc<Self>,
        mut child: Box<dyn Child + Send + Sync>,
        app: AppHandle<R>,
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
                        "pty:error",
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
            thread::sleep(std::time::Duration::from_millis(200));
            session.begin_stop_io();
            session.finish_stop_io();
            session.maybe_emit_exit(&app);
        });
    }

    fn maybe_emit_exit<R: tauri::Runtime>(&self, app: &AppHandle<R>) {
        if self.completed.load(Ordering::Acquire)
            || !self.attached.load(Ordering::Acquire)
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

        if let Err(error) = app.emit("pty:exit", event) {
            self.exit_emitted.store(false, Ordering::Release);
            let _ = app.emit(
                "pty:error",
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

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<(), PtyError> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE},
        System::Threading::{
            GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            PROCESS_TERMINATE,
        },
    };

    let process = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(());
        }
        return Err(PtyError::Io(error.to_string()));
    }

    let terminated = unsafe { TerminateProcess(process, 1) };
    let terminate_error = if terminated == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    let result = if let Some(error) = terminate_error {
        let mut exit_code = 0;
        let has_exited = unsafe { GetExitCodeProcess(process, &mut exit_code) } != 0
            && exit_code != STILL_ACTIVE as u32;
        if has_exited {
            Ok(())
        } else {
            Err(PtyError::Io(error.to_string()))
        }
    } else {
        Ok(())
    };

    let closed = unsafe { CloseHandle(process) };
    let close_error = if closed == 0 {
        Some(std::io::Error::last_os_error())
    } else {
        None
    };
    match (result, close_error) {
        (Err(error), _) => Err(error),
        (Ok(()), Some(error)) => Err(PtyError::Io(error.to_string())),
        (Ok(()), None) => Ok(()),
    }
}

#[cfg(test)]
mod discard_tests {
    use std::{
        collections::BTreeMap,
        io::{self, Read},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    use portable_pty::Child;

    use super::PtySession;
    use crate::pty::{
        backend::{PortablePtyBackend, PtyBackend},
        model::SpawnSpec,
    };

    struct OutputCleanup {
        worker: Option<JoinHandle<io::Result<u64>>>,
    }

    impl OutputCleanup {
        fn start(reader: Box<dyn Read + Send>, drained: Arc<AtomicBool>) -> Self {
            let worker = thread::spawn(move || {
                let mut reader = crate::pty::test_support::TraceReader::new(reader);
                let result = io::copy(&mut reader, &mut io::sink());
                if result.is_ok() {
                    drained.store(true, Ordering::Release);
                }
                result
            });
            Self {
                worker: Some(worker),
            }
        }

        fn join(mut self) {
            self.worker
                .take()
                .expect("output drain thread is present")
                .join()
                .expect("output drain thread does not panic")
                .expect("PTY output drain succeeds");
        }
    }

    impl Drop for OutputCleanup {
        fn drop(&mut self) {
            if let Some(drain) = self.worker.take() {
                let _ = drain.join();
            }
        }
    }

    struct ChildCleanup {
        child: Option<Box<dyn Child + Send + Sync>>,
        exited: Arc<AtomicBool>,
    }

    impl ChildCleanup {
        fn new(child: Box<dyn Child + Send + Sync>, exited: Arc<AtomicBool>) -> Self {
            Self {
                child: Some(child),
                exited,
            }
        }

        fn wait_for_exit(&mut self) -> bool {
            for _ in 0..300 {
                if self
                    .child
                    .as_mut()
                    .expect("child cleanup handle is present")
                    .try_wait()
                    .expect("PTY child status can be queried")
                    .is_some()
                {
                    self.exited.store(true, Ordering::Release);
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        }
    }

    impl Drop for ChildCleanup {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                let still_running = match child.try_wait() {
                    Ok(Some(_)) => false,
                    Ok(None) => true,
                    Err(_) => {
                        let _ = child.kill();
                        false
                    }
                };
                if still_running {
                    #[cfg(unix)]
                    if let Some(pid) = child.process_id() {
                        unsafe {
                            libc::kill(pid as libc::pid_t, libc::SIGKILL);
                        }
                    }
                    #[cfg(windows)]
                    {
                        let _ = child.kill();
                    }
                }
                if child.wait().is_ok() {
                    self.exited.store(true, Ordering::Release);
                }
            }
        }
    }

    struct ReserveCleanup {
        flow: Arc<super::super::flow::FlowControl>,
        worker: Option<JoinHandle<()>>,
    }

    impl ReserveCleanup {
        fn join(mut self) {
            self.worker
                .take()
                .expect("reserve worker is present")
                .join()
                .expect("reserve worker exits after flow closes");
        }
    }

    impl Drop for ReserveCleanup {
        fn drop(&mut self) {
            self.flow.close();
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn fixture_spec() -> SpawnSpec {
        let mut environment = BTreeMap::new();
        if let Some(path) = std::env::var_os("PATH") {
            environment.insert("PATH".into(), path.to_string_lossy().into_owned());
        }

        #[cfg(unix)]
        let (executable, arguments) = (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), "exec sleep 60".to_owned()],
        );

        #[cfg(windows)]
        let (executable, arguments) = {
            let command_processor = std::env::var_os("COMSPEC")
                .or_else(|| std::env::var_os("ComSpec"))
                .expect("Windows PTY test requires COMSPEC");
            for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
                if let Some(value) = std::env::var_os(name) {
                    environment.insert(name.to_owned(), value.to_string_lossy().into_owned());
                }
            }
            (
                command_processor.to_string_lossy().into_owned(),
                vec!["/c".to_owned(), "pause".to_owned()],
            )
        };

        SpawnSpec {
            executable,
            arguments,
            cwd: None,
            environment,
            columns: 80,
            rows: 24,
            shell_type: None,
            sudo: None,
        }
    }

    #[test]
    fn discard_closes_detached_flow_wakes_blocked_reserve_and_completes_once() {
        let test_name =
            "pty::session::discard_tests::discard_closes_detached_flow_wakes_blocked_reserve_and_completes_once";
        if !crate::pty::test_support::run_bounded(test_name) {
            return;
        }

        let spawned = PortablePtyBackend
            .spawn(&fixture_spec())
            .expect("real PTY child spawns");
        let crate::pty::backend::SpawnedPty {
            master,
            reader,
            writer,
            killer,
            child,
            pid,
            ..
        } = spawned;
        let output_cleanup = OutputCleanup::start(reader, Arc::new(AtomicBool::new(false)));
        let mut child_cleanup = ChildCleanup::new(child, Arc::new(AtomicBool::new(false)));
        let completed = Arc::new(AtomicUsize::new(0));
        let completion_counter = Arc::clone(&completed);
        let session = Arc::new(PtySession::new(
            "discard-flow-fixture".into(),
            pid,
            None,
            master,
            writer,
            killer,
            None,
            Arc::new(move |_| {
                completion_counter.fetch_add(1, Ordering::SeqCst);
            }),
        ));

        let flow = Arc::clone(&session.flow);
        let (started, waiting) = mpsc::channel();
        let (reserved, result) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started.send(()).unwrap();
            reserved.send(flow.reserve(1)).unwrap();
        });
        let waiter_cleanup = ReserveCleanup {
            flow: Arc::clone(&session.flow),
            worker: Some(waiter),
        };
        waiting.recv_timeout(Duration::from_secs(1)).unwrap();
        thread::sleep(Duration::from_millis(25));

        let first_discard = session.discard();
        let second_discard = session.discard();
        if first_discard.is_err() || second_discard.is_err() {
            session.flow.close();
        }
        let reservation_succeeded = result
            .recv_timeout(Duration::from_secs(1))
            .expect("discard must wake a blocked detached reserve");
        waiter_cleanup.join();
        first_discard.unwrap();
        second_discard.unwrap();

        assert!(
            !reservation_succeeded,
            "closed flow must release reserve with false"
        );
        assert!(
            child_cleanup.wait_for_exit(),
            "discard must terminate the real PTY child"
        );
        assert!(
            !session.is_alive(),
            "discarded session must no longer report alive"
        );
        assert!(
            !session.can_restore(),
            "discarded session must no longer be restorable"
        );
        assert_eq!(completed.load(Ordering::SeqCst), 1);

        eprintln!("PTY_DISCARD_STAGE before session drop");
        drop(session);
        eprintln!("PTY_DISCARD_STAGE after session drop");
        output_cleanup.join();
        eprintln!("PTY_DISCARD_STAGE after output drain joined");
    }

    #[test]
    fn panic_unwinds_session_child_and_output_cleanup_within_bound() {
        let test_name =
            "pty::session::discard_tests::panic_unwinds_session_child_and_output_cleanup_within_bound";
        if !crate::pty::test_support::run_bounded(test_name) {
            return;
        }

        let started_at = Instant::now();
        let drained = Arc::new(AtomicBool::new(false));
        let child_exited = Arc::new(AtomicBool::new(false));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let spawned = PortablePtyBackend
                .spawn(&fixture_spec())
                .expect("real PTY child spawns for panic cleanup control");
            let crate::pty::backend::SpawnedPty {
                master,
                reader,
                writer,
                killer,
                child,
                pid,
                ..
            } = spawned;

            let _output_cleanup = OutputCleanup::start(reader, Arc::clone(&drained));
            let _child_cleanup = ChildCleanup::new(child, Arc::clone(&child_exited));
            let _session = Arc::new(PtySession::new(
                "discard-panic-cleanup-control".into(),
                pid,
                None,
                master,
                writer,
                killer,
                None,
                Arc::new(|_| {}),
            ));

            panic!("intentional panic to verify PTY fixture cleanup order");
        }));

        assert!(result.is_err(), "the cleanup control must catch its panic");
        assert!(
            child_exited.load(Ordering::Acquire),
            "panic cleanup must terminate and reap the real PTY child"
        );
        assert!(
            drained.load(Ordering::Acquire),
            "panic cleanup must finish draining the real PTY output"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(10),
            "PTY fixture panic cleanup must finish within 10 seconds"
        );
    }
}
