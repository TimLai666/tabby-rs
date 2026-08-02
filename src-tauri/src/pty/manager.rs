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

pub struct PtyManager {
    backend: Arc<dyn PtyBackend>,
    sessions: RwLock<HashMap<String, Arc<PtySession>>>,
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
            sessions: RwLock::new(HashMap::new()),
            processes: ProcessInspector::new(),
        }
    }

    pub fn spawn(
        &self,
        app: AppHandle,
        request: PtySpawnRequest,
    ) -> Result<PtySpawnResponse, AppError> {
        let spec = SpawnSpec::try_from(request)?;
        let spawned = self.backend.spawn(&spec).map_err(AppError::from)?;
        let id = new_session_id();
        let response = PtySpawnResponse {
            id: id.clone(),
            pid: spawned.pid,
        };
        let session = Arc::new(PtySession::new(
            id.clone(),
            spawned.pid,
            spawned.master,
            spawned.writer,
            spawned.killer,
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

    pub fn ack(&self, id: &str, bytes: usize) -> Result<(), AppError> {
        self.session_or_error(id)?.ack(bytes);
        Ok(())
    }

    pub fn pid(&self, id: &str) -> Result<u32, AppError> {
        Ok(self.session_or_error(id)?.pid())
    }

    pub fn true_pid(&self, id: &str) -> Result<u32, AppError> {
        let root = self.live_session(id)?.pid();
        Ok(self.processes.true_pid(root))
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
    use super::new_session_id;

    #[test]
    fn session_ids_are_random_uuid_v4_values() {
        let first = new_session_id();
        let second = new_session_id();
        assert_ne!(first, second);
        assert_eq!(first.len(), 36);
        assert_eq!(&first[14..15], "4");
        assert!(matches!(&first[19..20], "8" | "9" | "a" | "b"));
    }
}
