use std::{collections::HashMap, sync::Arc};

use russh_sftp::{
    client::{fs::File, SftpSession},
    protocol::OpenFlags,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    backend::{RusshSftpBackend, SftpBackend},
    model::{RemoteFileEntry, SftpOverwritePolicy, SftpTransferDescriptor},
    path,
};
use crate::ssh::model::SshError;

enum SftpTransfer {
    Upload {
        file: File,
        temp_path: String,
        final_path: String,
        descriptor: SftpTransferDescriptor,
    },
    Download {
        file: File,
        descriptor: SftpTransferDescriptor,
    },
}

pub struct SftpManager {
    pub backend: RusshSftpBackend,
    transfers: HashMap<String, SftpTransfer>,
    next_transfer_id: u64,
}

impl SftpManager {
    pub fn new(session: SftpSession) -> Self {
        Self {
            backend: RusshSftpBackend::new(session),
            transfers: HashMap::new(),
            next_transfer_id: 0,
        }
    }

    pub fn session(&self) -> Arc<SftpSession> {
        Arc::clone(&self.backend.session)
    }

    pub async fn shutdown(mut self) {
        let ids: Vec<String> = self.transfers.keys().cloned().collect();
        for id in ids {
            let _ = self.cancel(&id).await;
        }
        let _ = self.backend.session.close().await;
    }

    pub async fn list(&self, path: &str) -> Result<Vec<RemoteFileEntry>, SshError> {
        self.backend.list(path).await
    }

    pub async fn stat(&self, path: &str, follow: bool) -> Result<RemoteFileEntry, SshError> {
        self.backend.stat(path, follow).await
    }

    pub async fn mkdir(&self, path: &str) -> Result<(), SshError> {
        self.backend.mkdir(path).await
    }

    pub async fn rename(&self, from: &str, to: &str) -> Result<(), SshError> {
        self.backend.rename(from, to).await
    }

    pub async fn remove(&self, path: &str, recursive: bool) -> Result<(), SshError> {
        self.backend.remove(path, recursive).await
    }

    fn next_transfer_id(&mut self) -> String {
        self.next_transfer_id = self.next_transfer_id.saturating_add(1);
        format!("sftp-transfer-{}", self.next_transfer_id)
    }

    async fn available_upload_path(
        &self,
        path: &str,
        policy: SftpOverwritePolicy,
    ) -> Result<String, SshError> {
        let path = super::model::checked_path(path)?;
        let exists = self.backend.stat(&path, false).await.is_ok();
        if !exists {
            return Ok(path);
        }
        match policy {
            SftpOverwritePolicy::Skip => {
                Err(SshError::Sftp("remote destination already exists".into()))
            }
            SftpOverwritePolicy::Overwrite => Ok(path),
            SftpOverwritePolicy::Rename => {
                let parent = path
                    .rsplit_once('/')
                    .map(|(parent, _)| parent)
                    .unwrap_or(".");
                let name = path::basename(&path);
                for index in 1..=10_000 {
                    let candidate = format!("{parent}/{name} ({index})");
                    if self.backend.stat(&candidate, false).await.is_err() {
                        return Ok(path::normalize(&candidate)?);
                    }
                }
                Err(SshError::Sftp(
                    "could not find a free remote destination name".into(),
                ))
            }
        }
    }

    pub async fn open_upload(
        &mut self,
        path: &str,
        size: Option<u64>,
        policy: SftpOverwritePolicy,
    ) -> Result<SftpTransferDescriptor, SshError> {
        let final_path = self.available_upload_path(path, policy).await?;
        if final_path == "/" || final_path == "." {
            return Err(SshError::InvalidRequest(
                "SFTP upload requires a file path".into(),
            ));
        }
        let id = self.next_transfer_id();
        let temp_path = format!("{final_path}.tabby-upload-{id}");
        let file = self
            .backend
            .session
            .open_with_flags(
                temp_path.clone(),
                OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
            )
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        let descriptor = SftpTransferDescriptor {
            id: id.clone(),
            direction: "upload".into(),
            name: path::basename(&final_path),
            size,
            transferred: 0,
            state: "pending".into(),
        };
        self.transfers.insert(
            id,
            SftpTransfer::Upload {
                file,
                temp_path,
                final_path,
                descriptor: descriptor.clone(),
            },
        );
        Ok(descriptor)
    }

    pub async fn open_download(&mut self, path: &str) -> Result<SftpTransferDescriptor, SshError> {
        let path = super::model::checked_path(path)?;
        let metadata = self
            .backend
            .session
            .symlink_metadata(path.clone())
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        if metadata.is_dir() || metadata.is_symlink() {
            return Err(SshError::InvalidRequest(
                "SFTP download requires a regular remote file".into(),
            ));
        }
        let id = self.next_transfer_id();
        let descriptor = SftpTransferDescriptor {
            id: id.clone(),
            direction: "download".into(),
            name: path::basename(&path),
            size: metadata.size,
            transferred: 0,
            state: "pending".into(),
        };
        let file = self
            .backend
            .session
            .open(path)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        self.transfers.insert(
            id,
            SftpTransfer::Download {
                file,
                descriptor: descriptor.clone(),
            },
        );
        Ok(descriptor)
    }

    pub async fn read(
        &mut self,
        id: &str,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, SftpTransferDescriptor), SshError> {
        let transfer = self
            .transfers
            .get_mut(id)
            .ok_or_else(|| SshError::InvalidRequest("SFTP transfer is unknown".into()))?;
        let SftpTransfer::Download { file, descriptor } = transfer else {
            return Err(SshError::InvalidRequest(
                "SFTP upload transfers cannot be read".into(),
            ));
        };
        let mut bytes = vec![0; max_bytes.clamp(1, 1024 * 1024)];
        let count = file
            .read(&mut bytes)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        bytes.truncate(count);
        descriptor.transferred = descriptor.transferred.saturating_add(count as u64);
        descriptor.state = if count == 0 { "completed" } else { "running" }.into();
        Ok((bytes, descriptor.clone()))
    }

    pub async fn write(
        &mut self,
        id: &str,
        bytes: &[u8],
    ) -> Result<SftpTransferDescriptor, SshError> {
        if bytes.len() > 1024 * 1024 {
            return Err(SshError::InvalidRequest("SFTP chunk is too large".into()));
        }
        let transfer = self
            .transfers
            .get_mut(id)
            .ok_or_else(|| SshError::InvalidRequest("SFTP transfer is unknown".into()))?;
        let SftpTransfer::Upload {
            file, descriptor, ..
        } = transfer
        else {
            return Err(SshError::InvalidRequest(
                "SFTP download transfers cannot be written".into(),
            ));
        };
        if descriptor
            .size
            .is_some_and(|size| descriptor.transferred.saturating_add(bytes.len() as u64) > size)
        {
            return Err(SshError::InvalidRequest(
                "SFTP upload exceeds the advertised size".into(),
            ));
        }
        file.write_all(bytes)
            .await
            .map_err(|error| SshError::Sftp(error.to_string()))?;
        descriptor.transferred = descriptor.transferred.saturating_add(bytes.len() as u64);
        descriptor.state = "running".into();
        Ok(descriptor.clone())
    }

    pub async fn close(&mut self, id: &str) -> Result<SftpTransferDescriptor, SshError> {
        let transfer = self
            .transfers
            .remove(id)
            .ok_or_else(|| SshError::InvalidRequest("SFTP transfer is unknown".into()))?;
        match transfer {
            SftpTransfer::Upload {
                mut file,
                temp_path,
                final_path,
                mut descriptor,
            } => {
                if descriptor
                    .size
                    .is_some_and(|size| descriptor.transferred != size)
                {
                    let _ = self.backend.session.remove_file(temp_path).await;
                    return Err(SshError::InvalidRequest(
                        "SFTP upload closed before all bytes were sent".into(),
                    ));
                }
                if let Err(error) = file.shutdown().await {
                    let _ = self.backend.session.remove_file(&temp_path).await;
                    return Err(SshError::Sftp(error.to_string()));
                }
                if let Err(error) = self.backend.rename(&temp_path, &final_path).await {
                    let _ = self.backend.session.remove_file(&temp_path).await;
                    return Err(error);
                }
                descriptor.state = "completed".into();
                Ok(descriptor)
            }
            SftpTransfer::Download {
                mut file,
                mut descriptor,
            } => {
                file.shutdown()
                    .await
                    .map_err(|error| SshError::Sftp(error.to_string()))?;
                descriptor.state = "completed".into();
                Ok(descriptor)
            }
        }
    }

    pub async fn cancel(&mut self, id: &str) -> Result<SftpTransferDescriptor, SshError> {
        let transfer = self
            .transfers
            .remove(id)
            .ok_or_else(|| SshError::InvalidRequest("SFTP transfer is unknown".into()))?;
        let (temp_path, descriptor) = match transfer {
            SftpTransfer::Upload {
                temp_path,
                descriptor,
                ..
            } => (Some(temp_path), descriptor),
            SftpTransfer::Download { descriptor, .. } => (None, descriptor),
        };
        if let Some(temp_path) = temp_path {
            let _ = self.backend.session.remove_file(temp_path).await;
        }
        Ok(SftpTransferDescriptor {
            state: "cancelled".into(),
            ..descriptor
        })
    }
}
