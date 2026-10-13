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

    pub fn spawn(
        &self,
        app: AppHandle,
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

    pub fn attach(&self, app: &AppHandle, id: &str) -> Result<(), AppError> {
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
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    use portable_pty::Child;

    use super::{new_session_id, PtyManager};
    use crate::pty::{
        backend::{PortablePtyBackend, PtyBackend, SpawnedPty},
        model::SpawnSpec,
        session::PtySession,
    };

    struct ChildCleanup(Option<Box<dyn Child + Send + Sync>>);

    impl ChildCleanup {
        fn is_running(&mut self) -> bool {
            self.0
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
    }

    impl Drop for ChildCleanup {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
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
                let _ = child.wait();
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

    fn register_fixture(manager: &PtyManager, id: &str) -> ChildCleanup {
        let spawned = spawn_fixture();
        let SpawnedPty {
            master,
            reader: _reader,
            writer,
            killer,
            child,
            pid,
        } = spawned;
        let cleanup = ChildCleanup(Some(child));
        let started_at = manager.processes.start_time(pid);
        let sessions = Arc::downgrade(&manager.sessions);
        let on_completed = Arc::new(move |completed_id: &str| {
            if let Some(sessions) = sessions.upgrade() {
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
        manager
            .sessions
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
    }
}
