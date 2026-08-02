use std::io::{Read, Write};

use portable_pty::{
    native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize, PtySystem,
};

use super::model::SpawnSpec;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("PTY spawn failed: {0}")]
    Spawn(String),
    #[error("PTY I/O failed: {0}")]
    Io(String),
    #[error("PTY process identifier is unavailable")]
    MissingPid,
}

impl From<std::io::Error> for PtyError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

pub struct SpawnedPty {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
    pub pid: u32,
}

pub trait PtyBackend: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedPty, PtyError>;
}

#[derive(Debug, Default)]
pub struct PortablePtyBackend;

impl PtyBackend for PortablePtyBackend {
    fn spawn(&self, spec: &SpawnSpec) -> Result<SpawnedPty, PtyError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| PtyError::Spawn(error.to_string()))?;

        let mut command = CommandBuilder::new(&spec.executable);
        command.args(&spec.arguments);
        command.env_clear();
        for (key, value) in &spec.environment {
            command.env(key, value);
        }
        if let Some(cwd) = &spec.cwd {
            command.cwd(cwd);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| PtyError::Spawn(error.to_string()))?;
        let pid = child.process_id().ok_or(PtyError::MissingPid)?;
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| PtyError::Io(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| PtyError::Io(error.to_string()))?;
        drop(pair.slave);

        Ok(SpawnedPty {
            master: pair.master,
            reader,
            writer,
            child,
            killer,
            pid,
        })
    }
}
