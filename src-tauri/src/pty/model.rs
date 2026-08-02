use std::collections::BTreeMap;

use crate::shell::{PreparedSpawnRequest, ShellType};

pub const MAX_CHUNK_BYTES: usize = 100 * 1024;
pub const MAX_UNACKED_BYTES: usize = MAX_CHUNK_BYTES * 5;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtySpawnRequest {
    pub prepared: PreparedSpawnRequest,
    #[serde(default = "default_columns")]
    pub columns: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
}

fn default_columns() -> u16 {
    80
}

fn default_rows() -> u16 {
    30
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtySpawnResponse {
    pub id: String,
    pub pid: u32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PtyIdRequest {
    pub id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PtyWriteRequest {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PtyResizeRequest {
    pub id: String,
    pub columns: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PtyKillRequest {
    pub id: String,
    #[serde(default)]
    pub signal: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PtyAckRequest {
    pub id: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputEvent {
    pub id: String,
    pub sequence: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitEvent {
    pub id: String,
    pub exit_code: Option<u32>,
    pub signal: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyErrorEvent {
    pub id: String,
    pub code: String,
    pub details: String,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ChildProcess {
    pub pid: u32,
    pub ppid: u32,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub executable: String,
    pub arguments: Vec<String>,
    pub cwd: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub columns: u16,
    pub rows: u16,
    pub shell_type: Option<ShellType>,
}

impl TryFrom<PtySpawnRequest> for SpawnSpec {
    type Error = crate::error::AppError;

    fn try_from(request: PtySpawnRequest) -> Result<Self, Self::Error> {
        if request.columns == 0 || request.rows == 0 {
            return Err(crate::error::AppError::InvalidArgument(
                "PTY dimensions must be greater than zero".into(),
            ));
        }
        let prepared = request.prepared;
        Ok(Self {
            executable: prepared.executable,
            arguments: prepared.arguments,
            cwd: prepared.cwd,
            environment: prepared.environment,
            columns: request.columns,
            rows: request.rows,
            shell_type: prepared.shell_type,
        })
    }
}
