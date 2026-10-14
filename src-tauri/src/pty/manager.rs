use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use rand::{rngs::OsRng, RngCore};
use tauri::AppHandle;

use crate::error::AppError;

use super::{
    backend::{PortablePtyBackend, PtyBackend, PtyError},
    model::{ChildProcess, PtySpawnRequest, PtySpawnResponse, SpawnSpec},
    process::ProcessInspector,
    session::PtySession,
};

type SessionMap = Arc<RwLock<HashMap<String, Arc<PtySession>>>>;

pub struct PtyManager {
    backend: Arc<dyn PtyBackend>,
    sessions: SessionMap,
    processes: ProcessInspector,
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new(Arc::new(PortablePtyBackend))
    }
}

impl PtyManager {
    pub fn new(backend: Arc<dyn PtyBackend>) -> Self {
        Self {
            backend,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            processes: ProcessInspector::new(),
        }
    }

    pub fn spawn<R: tauri::Runtime>(
        &self,
        app: AppHandle<R>,
        request: PtySpawnRequest,
    ) -> Result<PtySpawnResponse, AppError> {
        let spec = SpawnSpec::try_from(request)?;
        let sudo = spec.sudo.clone();
        let spawned = self.backend.spawn(&spec).map_err(AppError::from)?;
        let id = new_session_id();
        let response = PtySpawnResponse {
            id: id.clone(),
            pid: spawned.pid,
        };
        let started_at = self.processes.start_time(spawned.pid);
        let sessions = Arc::downgrade(&self.sessions);
        let on_completed = Arc::new(move |completed_id: &str| {
            if let Some(sessions) = sessions.upgrade() {
                sessions
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(completed_id);
            }
        });
        let session = Arc::new(PtySession::new(
            id.clone(),
            spawned.pid,
            started_at,
            spawned.master,
            spawned.writer,
            spawned.killer,
            sudo,
            on_completed,
        ));

        self.sessions
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, Arc::clone(&session));
        session.start(spawned.reader, spawned.child, app);
        Ok(response)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.session(id)
            .map(|session| session.can_restore())
            .unwrap_or(false)
    }

    pub fn is_alive(&self, id: &str) -> bool {
        self.session(id)
            .map(|session| session.is_alive())
            .unwrap_or(false)
    }

    pub fn attach<R: tauri::Runtime>(&self, app: &AppHandle<R>, id: &str) -> Result<(), AppError> {
        self.session_or_error(id)?.attach(app);
        Ok(())
    }

    pub fn detach(&self, id: &str) -> Result<(), AppError> {
        self.session_or_error(id)?.detach();
        Ok(())
    }

    pub fn write(&self, id: &str, data: &[u8]) -> Result<(), AppError> {
        self.live_session(id)?.write(data).map_err(AppError::from)
    }

    pub fn resize(&self, id: &str, columns: u16, rows: u16) -> Result<(), AppError> {
        self.live_session(id)?
            .resize(columns, rows)
            .map_err(AppError::from)
    }

    pub fn kill(&self, id: &str, signal: Option<&str>) -> Result<(), AppError> {
        self.live_session(id)?.kill(signal).map_err(AppError::from)
    }

    pub fn discard(&self, id: &str) -> Result<(), AppError> {
        if let Some(session) = self.session(id) {
            session.discard().map_err(AppError::from)?;
        }
        Ok(())
    }

    pub fn ack(&self, id: &str, bytes: usize) -> Result<(), AppError> {
        self.session_or_error(id)?.ack(bytes);
        Ok(())
    }

    pub fn claim_sudo(&self, prompt_id: &str) -> Result<(Arc<PtySession>, String), AppError> {
        let sessions = self
            .sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            match session.claim_sudo(prompt_id) {
                Ok(Some(secret_ref)) => return Ok((session, secret_ref)),
                Ok(None) => continue,
                Err(error) => return Err(AppError::from(error)),
            }
        }
        Err(AppError::NotFound(
            "sudo prompt is unknown, expired, or already handled".into(),
        ))
    }

    pub fn pid(&self, id: &str) -> Result<u32, AppError> {
        Ok(self.session_or_error(id)?.pid())
    }

    pub fn true_pid(&self, id: &str) -> Result<u32, AppError> {
        let session = self.live_session(id)?;
        self.processes
            .true_pid(session.pid(), session.started_at())
            .ok_or_else(|| AppError::NotFound("PTY process identity no longer matches".into()))
    }

    pub fn children(&self, id: &str) -> Result<Vec<ChildProcess>, AppError> {
        let pid = self.true_pid(id)?;
        Ok(self.processes.children(pid))
    }

    pub fn cwd(&self, id: &str) -> Result<Option<String>, AppError> {
        let pid = self.true_pid(id)?;
        Ok(self.processes.cwd(pid))
    }

    fn live_session(&self, id: &str) -> Result<Arc<PtySession>, AppError> {
        let session = self.session_or_error(id)?;
        if !session.is_alive() {
            return Err(AppError::NotFound("PTY session has exited".into()));
        }
        Ok(session)
    }

    fn session_or_error(&self, id: &str) -> Result<Arc<PtySession>, AppError> {
        self.session(id)
            .ok_or_else(|| AppError::NotFound("PTY session does not exist".into()))
    }

    fn session(&self, id: &str) -> Option<Arc<PtySession>> {
        self.sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
    }
}

impl From<PtyError> for AppError {
    fn from(error: PtyError) -> Self {
        match error {
            PtyError::MissingPid => AppError::Io("PTY process identifier is unavailable".into()),
            PtyError::Spawn(details) => AppError::Io(format!("PTY spawn failed: {details}")),
            PtyError::Io(details) => AppError::Io(format!("PTY I/O failed: {details}")),
        }
    }
}

fn new_session_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    use portable_pty::Child;

    use super::{new_session_id, PtyManager, SessionMap};
    use crate::pty::{
        backend::{PortablePtyBackend, PtyBackend, SpawnedPty},
        model::SpawnSpec,
        session::PtySession,
    };

    struct ChildCleanup {
        child: Option<Box<dyn Child + Send + Sync>>,
        reaped: Arc<AtomicBool>,
    }

    impl ChildCleanup {
        fn new(child: Box<dyn Child + Send + Sync>) -> (Self, Arc<AtomicBool>) {
            let reaped = Arc::new(AtomicBool::new(false));
            (
                Self {
                    child: Some(child),
                    reaped: Arc::clone(&reaped),
                },
                reaped,
            )
        }

        fn is_running(&mut self) -> bool {
            self.child
                .as_mut()
                .expect("child cleanup handle is present")
                .try_wait()
                .expect("PTY child status can be queried")
                .is_none()
        }

        fn wait_for_exit(&mut self) -> bool {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if !self.is_running() {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            !self.is_running()
        }

        fn finish(&mut self) -> Result<(), String> {
            if let Some(mut child) = self.child.take() {
                let still_running = child
                    .try_wait()
                    .map_err(|error| format!("could not inspect PTY child: {error}"))?
                    .is_none();
                if still_running {
                    #[cfg(unix)]
                    if let Some(pid) = child.process_id() {
                        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
                        if result != 0 {
                            let error = io::Error::last_os_error();
                            if error.raw_os_error() != Some(libc::ESRCH) {
                                return Err(format!("could not terminate PTY child: {error}"));
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    child
                        .kill()
                        .map_err(|error| format!("could not terminate PTY child: {error}"))?;
                }
                child
                    .wait()
                    .map_err(|error| format!("could not reap PTY child: {error}"))?;
                self.reaped.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    impl Drop for ChildCleanup {
        fn drop(&mut self) {
            let _ = self.finish();
        }
    }

    #[derive(Clone)]
    struct CleanupSignals {
        child_reaped: Arc<AtomicBool>,
        reader_drained: Arc<AtomicBool>,
    }

    struct RegisteredFixtureCleanup {
        sessions: SessionMap,
        id: String,
        child: ChildCleanup,
        reader: Option<JoinHandle<io::Result<u64>>>,
        signals: CleanupSignals,
    }

    impl RegisteredFixtureCleanup {
        fn signals(&self) -> CleanupSignals {
            self.signals.clone()
        }

        fn is_running(&mut self) -> bool {
            self.child.is_running()
        }

        fn wait_for_exit(&mut self) -> bool {
            self.child.wait_for_exit()
        }

        fn finish(&mut self) -> Result<(), String> {
            let record = {
                self.sessions
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&self.id)
            };
            drop(record);

            let child_result = self.child.finish();
            let reader_result = self.reader.take().map(|reader| {
                reader
                    .join()
                    .map_err(|_| "PTY reader drain thread panicked".to_owned())?
                    .map(|_| ())
                    .map_err(|error| format!("PTY reader drain failed: {error}"))
            });
            match (child_result, reader_result) {
                (Ok(()), Some(Ok(())) | None) => Ok(()),
                (Err(child_error), Some(Err(reader_error))) => {
                    Err(format!("{child_error}; {reader_error}"))
                }
                (Err(error), _) | (Ok(()), Some(Err(error))) => Err(error),
            }
        }
    }

    impl Drop for RegisteredFixtureCleanup {
        fn drop(&mut self) {
            if let Err(error) = self.finish() {
                eprintln!("PTY fixture cleanup failed for {}: {error}", self.id);
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

    fn spawn_fixture() -> SpawnedPty {
        PortablePtyBackend
            .spawn(&fixture_spec())
            .expect("real PTY child spawns")
    }

    fn register_fixture(manager: &PtyManager, id: &str) -> RegisteredFixtureCleanup {
        let spawned = spawn_fixture();
        let SpawnedPty {
            master,
            reader,
            writer,
            killer,
            child,
            pid,
        } = spawned;
        let (child, child_reaped) = ChildCleanup::new(child);
        let reader_drained = Arc::new(AtomicBool::new(false));
        let drain_completed = Arc::clone(&reader_drained);
        let reader = thread::spawn(move || {
            let mut reader = crate::pty::test_support::TraceReader::new(reader);
            let result = io::copy(&mut reader, &mut io::sink());
            if result.is_ok() {
                drain_completed.store(true, Ordering::Release);
            }
            result
        });
        let sessions = Arc::clone(&manager.sessions);
        let cleanup = RegisteredFixtureCleanup {
            sessions: Arc::clone(&sessions),
            id: id.to_owned(),
            child,
            reader: Some(reader),
            signals: CleanupSignals {
                child_reaped,
                reader_drained,
            },
        };
        let started_at = manager.processes.start_time(pid);
        let sessions_weak = Arc::downgrade(&sessions);
        let on_completed = Arc::new(move |completed_id: &str| {
            if let Some(sessions) = sessions_weak.upgrade() {
                sessions
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(completed_id);
            }
        });
        let session = Arc::new(PtySession::new(
            id.to_owned(),
            pid,
            started_at,
            master,
            writer,
            killer,
            None,
            on_completed,
        ));
        sessions
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.to_owned(), session);
        cleanup
    }

    #[test]
    fn session_ids_are_random_uuid_v4_values() {
        let first = new_session_id();
        let second = new_session_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        assert_eq!(&first[14..15], "4");
        assert!(matches!(&first[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn discard_reaps_real_child_removes_only_its_record_and_missing_ids_are_idempotent() {
        let test_name = "pty::manager::tests::discard_reaps_real_child_removes_only_its_record_and_missing_ids_are_idempotent";
        if !crate::pty::test_support::run_bounded(test_name) {
            return;
        }

        let manager = PtyManager::new(Arc::new(PortablePtyBackend));
        let mut discarded_child = register_fixture(&manager, "discard-fixture");
        let mut retained_child = register_fixture(&manager, "retained-fixture");

        assert!(manager.exists("discard-fixture"));
        assert!(manager.exists("retained-fixture"));
        assert!(retained_child.is_running());

        manager.discard("discard-fixture").unwrap();
        assert!(
            discarded_child.wait_for_exit(),
            "discard must terminate and reap the real child"
        );
        assert!(!manager.exists("discard-fixture"));
        assert!(!manager
            .sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key("discard-fixture"));

        assert!(manager.discard("discard-fixture").is_ok());
        assert!(manager.discard("unknown-fixture").is_ok());
        assert!(manager.exists("retained-fixture"));
        assert!(
            retained_child.is_running(),
            "discard must leave another child alive"
        );

        discarded_child
            .finish()
            .expect("discarded PTY fixture cleanup succeeds");
        retained_child
            .finish()
            .expect("retained PTY fixture cleanup succeeds");
    }

    #[test]
    fn panic_after_fixture_registration_cleans_its_record_child_and_reader() {
        let test_name = "pty::manager::tests::panic_after_fixture_registration_cleans_its_record_child_and_reader";
        if !crate::pty::test_support::run_bounded(test_name) {
            return;
        }

        let started_at = Instant::now();
        let manager = PtyManager::new(Arc::new(PortablePtyBackend));
        let signal_slot = Arc::new(Mutex::new(None));
        let worker_signal_slot = Arc::clone(&signal_slot);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let fixture = register_fixture(&manager, "panic-registration-fixture");
            *worker_signal_slot
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(fixture.signals());
            assert!(manager.exists("panic-registration-fixture"));
            panic!("intentional panic after PTY fixture registration");
        }));

        assert!(result.is_err(), "the cleanup control must catch its panic");
        assert!(
            !manager.exists("panic-registration-fixture"),
            "panic cleanup must remove the registered PTY session"
        );
        assert!(!manager
            .sessions
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key("panic-registration-fixture"));
        let signals = signal_slot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .expect("panic cleanup signals were captured");
        assert!(
            signals.child_reaped.load(Ordering::Acquire),
            "panic cleanup must terminate and reap the real PTY child"
        );
        assert!(
            signals.reader_drained.load(Ordering::Acquire),
            "panic cleanup must finish draining the real PTY output"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(10),
            "PTY fixture panic cleanup must finish within 10 seconds"
        );
    }
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod reader_tests;
