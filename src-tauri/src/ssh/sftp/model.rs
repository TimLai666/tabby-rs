use super::path;
use crate::ssh::model::SshError;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpSessionInfo {
    pub id: String,
    pub ssh_session_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEntry {
    pub name: String,
    pub full_path: String,
    pub is_directory: bool,
    pub is_symlink: bool,
    pub mode: u32,
    pub size: u64,
    pub modified: Option<u64>,
    pub is_operable: bool,
    pub unoperable_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SftpOverwritePolicy {
    Skip,
    Overwrite,
    Rename,
}

impl Default for SftpOverwritePolicy {
    fn default() -> Self {
        Self::Skip
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpPathRequest {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpStatRequest {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub follow: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpRenameRequest {
    pub id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpRemoveRequest {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpUploadOpenRequest {
    pub id: String,
    pub path: String,
    pub size: Option<u64>,
    #[serde(default)]
    pub overwrite_policy: SftpOverwritePolicy,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpDownloadOpenRequest {
    pub id: String,
    pub path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpTransferIdRequest {
    pub id: String,
    pub transfer_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpReadRequest {
    pub id: String,
    pub transfer_id: String,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpWriteRequest {
    pub id: String,
    pub transfer_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpTransferDescriptor {
    pub id: String,
    pub direction: String,
    pub name: String,
    pub size: Option<u64>,
    pub transferred: u64,
    pub state: String,
}

pub fn checked_path(value: &str) -> Result<String, SshError> {
    if value.contains('\u{FFFD}') {
        return Err(SshError::InvalidRequest(
            "remote filename is not valid UTF-8 and cannot be operated on by this client".into(),
        ));
    }
    path::normalize(value)
}
