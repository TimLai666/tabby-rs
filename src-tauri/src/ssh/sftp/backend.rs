use std::sync::Arc;

use async_trait::async_trait;
use russh_sftp::client::SftpSession;

use super::{model::RemoteFileEntry, path};
use crate::ssh::model::SshError;

pub type SftpResult<T> = Result<T, SshError>;

#[async_trait]
pub trait SftpBackend: Send + Sync {
    async fn list(&self, path: &str) -> SftpResult<Vec<RemoteFileEntry>>;
    async fn stat(&self, path: &str, follow: bool) -> SftpResult<RemoteFileEntry>;
    async fn mkdir(&self, path: &str) -> SftpResult<()>;
    async fn rename(&self, from: &str, to: &str) -> SftpResult<()>;
    async fn remove(&self, path: &str, recursive: bool) -> SftpResult<()>;
}

#[derive(Clone)]
pub struct RusshSftpBackend {
    pub session: Arc<SftpSession>,
}

impl RusshSftpBackend {
    pub fn new(session: SftpSession) -> Self {
        Self {
            session: Arc::new(session),
        }
    }

    fn entry(path: String, metadata: &russh_sftp::client::fs::Metadata) -> RemoteFileEntry {
        let name = path::basename(&path);
        let is_operable = !name.contains('\u{FFFD}');
        RemoteFileEntry {
            name,
            full_path: path,
            is_directory: metadata.file_type().is_dir(),
            is_symlink: metadata.file_type().is_symlink(),
            mode: metadata.permissions.unwrap_or_default(),
            size: metadata.size.unwrap_or_default(),
            modified: metadata.mtime.map(u64::from),
            is_operable,
            unoperable_reason: (!is_operable).then(|| {
                "remote filename was decoded from invalid UTF-8 and is display-only".into()
            }),
        }
    }
}

#[async_trait]
impl SftpBackend for RusshSftpBackend {
    async fn list(&self, path: &str) -> SftpResult<Vec<RemoteFileEntry>> {
        let path = super::model::checked_path(path)?;
        let entries = self
            .session
            .read_dir(path.clone())
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        entries
            .map(|entry| {
                let full_path = path::join(&path, &entry.file_name())?;
                let metadata = entry.metadata();
                Ok(Self::entry(full_path, &metadata))
            })
            .collect()
    }

    async fn stat(&self, path: &str, follow: bool) -> SftpResult<RemoteFileEntry> {
        let path = super::model::checked_path(path)?;
        let metadata = if follow {
            self.session.metadata(path.clone()).await
        } else {
            self.session.symlink_metadata(path.clone()).await
        }
        .map_err(|error| SshError::Sftp(error.to_string()))?;
        Ok(Self::entry(path, &metadata))
    }

    async fn mkdir(&self, path: &str) -> SftpResult<()> {
        let path = super::model::checked_path(path)?;
        self.session
            .create_dir(path)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))
    }

    async fn rename(&self, from: &str, to: &str) -> SftpResult<()> {
        let from = super::model::checked_path(from)?;
        let to = super::model::checked_path(to)?;
        self.session
            .rename(from, to)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))
    }

    async fn remove(&self, path: &str, recursive: bool) -> SftpResult<()> {
        let path = super::model::checked_path(path)?;
        if path == "/" || path == "." {
            return Err(SshError::InvalidRequest(
                "remote root cannot be deleted".into(),
            ));
        }
        let mut pending = vec![path];
        let mut seen: usize = 0;
        while let Some(current) = pending.pop() {
            seen = seen.saturating_add(1);
            if seen > 100_000 {
                return Err(SshError::InvalidRequest(
                    "remote delete contains too many entries".into(),
                ));
            }
            let entry = self.stat(&current, false).await?;
            if entry.is_directory {
                if !recursive {
                    return Err(SshError::InvalidRequest(
                        "recursive delete must be explicitly confirmed".into(),
                    ));
                }
                let children = self.list(&current).await?;
                if children.is_empty() {
                    self.session
                        .remove_dir(current)
                        .await
                        .map_err(|error| SshError::Sftp(error.to_string()))?;
                } else {
                    pending.push(current);
                    pending.extend(children.into_iter().map(|child| child.full_path));
                }
            } else {
                self.session
                    .remove_file(current)
                    .await
                    .map_err(|error| SshError::Sftp(error.to_string()))?;
            }
        }
        Ok(())
    }
}
