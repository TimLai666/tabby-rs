use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use tempfile::NamedTempFile;

use crate::error::AppError;

use super::safe_path::{resolve_inside, safe_file_name};

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferDescriptor {
    pub id: String,
    pub direction: String,
    pub name: String,
    pub size: Option<u64>,
    pub transferred: u64,
    pub state: String,
    pub error: Option<TransferError>,
}

#[derive(Debug)]
enum TransferSession {
    Upload {
        file: File,
        descriptor: TransferDescriptor,
    },
    Download {
        temp: NamedTempFile,
        destination: PathBuf,
        mode: u32,
        descriptor: TransferDescriptor,
    },
}

#[derive(Default)]
pub struct TransferManager {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<String, TransferSession>>,
}

impl TransferManager {
    fn next_id(&self) -> String {
        format!(
            "transfer-{}",
            self.next_id.fetch_add(1, Ordering::Relaxed) + 1
        )
    }

    pub fn open_upload(&self, paths: &[String]) -> Result<Vec<TransferDescriptor>, AppError> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("transfer manager lock poisoned");
        let mut descriptors = Vec::with_capacity(paths.len());
        let mut opened_ids = Vec::with_capacity(paths.len());
        for value in paths {
            let path = PathBuf::from(value);
            let result = (|| {
                let metadata = fs::metadata(&path)?;
                if !metadata.is_file() {
                    return Err(AppError::InvalidArgument(format!(
                        "not a regular file: {value}"
                    )));
                }
                let id = self.next_id();
                let descriptor = TransferDescriptor {
                    id: id.clone(),
                    direction: "upload".into(),
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "upload".into()),
                    size: Some(metadata.len()),
                    transferred: 0,
                    state: "pending".into(),
                    error: None,
                };
                let file = File::open(path)?;
                sessions.insert(
                    id.clone(),
                    TransferSession::Upload {
                        file,
                        descriptor: descriptor.clone(),
                    },
                );
                opened_ids.push(id);
                Ok(descriptor)
            })();
            match result {
                Ok(descriptor) => descriptors.push(descriptor),
                Err(error) => {
                    opened_ids.iter().for_each(|id| {
                        sessions.remove(id);
                    });
                    return Err(error);
                }
            }
        }
        Ok(descriptors)
    }

    pub fn open_download(
        &self,
        name: &str,
        mode: u32,
        size: Option<u64>,
        destination: &str,
        base_directory: Option<&str>,
        relative_path: Option<&str>,
    ) -> Result<TransferDescriptor, AppError> {
        let destination = match (base_directory, relative_path) {
            (Some(base), Some(relative)) => resolve_inside(Path::new(base), relative)?,
            (None, None) => PathBuf::from(destination),
            _ => {
                return Err(AppError::InvalidArgument(
                    "download base and relative paths must be supplied together".into(),
                ));
            }
        };
        if destination.as_os_str().is_empty() {
            return Err(AppError::InvalidArgument(
                "download destination is empty".into(),
            ));
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let parent = destination.parent().ok_or_else(|| {
            AppError::InvalidArgument("download destination has no parent".into())
        })?;
        let temp = NamedTempFile::new_in(parent)?;
        let id = self.next_id();
        let descriptor = TransferDescriptor {
            id: id.clone(),
            direction: "download".into(),
            name: safe_file_name(name),
            size,
            transferred: 0,
            state: "pending".into(),
            error: None,
        };
        self.sessions
            .lock()
            .expect("transfer manager lock poisoned")
            .insert(
                id,
                TransferSession::Download {
                    temp,
                    destination,
                    mode,
                    descriptor: descriptor.clone(),
                },
            );
        Ok(descriptor)
    }

    pub fn read(
        &self,
        id: &str,
        max_bytes: usize,
    ) -> Result<(Vec<u8>, TransferDescriptor), AppError> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("transfer manager lock poisoned");
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(format!("transfer {id}")))?;
        let TransferSession::Upload { file, descriptor } = session else {
            return Err(AppError::InvalidArgument(
                "download transfers cannot be read".into(),
            ));
        };
        let limit = max_bytes.clamp(1, 1024 * 1024);
        let mut bytes = vec![0; limit];
        let count = file.read(&mut bytes)?;
        bytes.truncate(count);
        descriptor.state = if count == 0 { "completed" } else { "running" }.into();
        descriptor.transferred = descriptor.transferred.saturating_add(count as u64);
        Ok((bytes, descriptor.clone()))
    }

    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<TransferDescriptor, AppError> {
        let mut sessions = self
            .sessions
            .lock()
            .expect("transfer manager lock poisoned");
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| AppError::NotFound(format!("transfer {id}")))?;
        let TransferSession::Download {
            temp, descriptor, ..
        } = session
        else {
            return Err(AppError::InvalidArgument(
                "upload transfers cannot be written".into(),
            ));
        };
        if let Some(size) = descriptor.size {
            if descriptor.transferred.saturating_add(bytes.len() as u64) > size {
                return Err(AppError::InvalidArgument(
                    "transfer exceeds advertised size".into(),
                ));
            }
        }
        temp.as_file_mut().write_all(bytes)?;
        descriptor.transferred = descriptor.transferred.saturating_add(bytes.len() as u64);
        descriptor.state = "running".into();
        Ok(descriptor.clone())
    }

    pub fn close(&self, id: &str) -> Result<TransferDescriptor, AppError> {
        let session = self
            .sessions
            .lock()
            .expect("transfer manager lock poisoned")
            .remove(id)
            .ok_or_else(|| AppError::NotFound(format!("transfer {id}")))?;
        match session {
            TransferSession::Upload {
                mut file,
                mut descriptor,
            } => {
                if descriptor.size != Some(descriptor.transferred) {
                    return Err(AppError::InvalidData(
                        "upload closed before the advertised size was read".into(),
                    ));
                }
                file.flush()?;
                descriptor.state = "completed".into();
                Ok(descriptor)
            }
            TransferSession::Download {
                mut temp,
                destination,
                mode,
                mut descriptor,
            } => {
                if let Some(size) = descriptor.size {
                    if descriptor.transferred != size {
                        return Err(AppError::InvalidData(
                            "download size does not match the advertised size".into(),
                        ));
                    }
                }
                temp.as_file_mut().flush()?;
                temp.persist(&destination)
                    .map_err(|error| AppError::Io(error.error.to_string()))?;
                #[cfg(unix)]
                fs::set_permissions(&destination, fs::Permissions::from_mode(mode))?;
                descriptor.state = "completed".into();
                Ok(descriptor)
            }
        }
    }

    pub fn cancel(&self, id: &str) -> Result<TransferDescriptor, AppError> {
        let mut session = self
            .sessions
            .lock()
            .expect("transfer manager lock poisoned")
            .remove(id)
            .ok_or_else(|| AppError::NotFound(format!("transfer {id}")))?;
        let descriptor = match &mut session {
            TransferSession::Upload { descriptor, .. }
            | TransferSession::Download { descriptor, .. } => descriptor,
        };
        descriptor.state = "cancelled".into();
        Ok(descriptor.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::TransferManager;

    #[test]
    fn cancelled_download_keeps_existing_destination() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("report.txt");
        fs::write(&destination, b"old").unwrap();
        let manager = TransferManager::default();

        let descriptor = manager
            .open_download(
                "report.txt",
                0o644,
                Some(3),
                destination.to_str().unwrap(),
                None,
                None,
            )
            .unwrap();
        manager.write(&descriptor.id, b"new").unwrap();
        manager.cancel(&descriptor.id).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"old");
    }

    #[test]
    fn completed_download_is_persisted_only_on_close() {
        let directory = tempdir().unwrap();
        let destination = directory.path().join("report.txt");
        let manager = TransferManager::default();

        let descriptor = manager
            .open_download(
                "report.txt",
                0o644,
                Some(3),
                destination.to_str().unwrap(),
                None,
                None,
            )
            .unwrap();
        manager.write(&descriptor.id, b"new").unwrap();
        assert!(!destination.exists());
        manager.close(&descriptor.id).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"new");
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
