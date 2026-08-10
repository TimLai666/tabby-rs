use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use russh::keys::{known_hosts::known_host_keys_path, HashAlg, PublicKey};

use super::model::{HostKeyStatus, SshError};

#[derive(Clone)]
pub struct KnownHostsStore {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl KnownHostsStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn classify(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Result<Option<(HostKeyStatus, Vec<String>)>, SshError> {
        validate_host(host)?;
        let recorded =
            known_host_keys_path(host, port, &self.path).map_err(|_| SshError::Internal)?;
        if recorded.is_empty() {
            return Ok(Some((HostKeyStatus::Unknown, Vec::new())));
        }

        let previous_fingerprints = recorded
            .iter()
            .map(|(_, key)| fingerprint(key))
            .collect::<Vec<_>>();
        if recorded.iter().any(|(_, recorded)| recorded == key) {
            Ok(None)
        } else {
            Ok(Some((HostKeyStatus::Changed, previous_fingerprints)))
        }
    }

    pub fn save(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        validate_host(host)?;
        let line = known_host_line(host, port, key)?;
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        let parent = self
            .path
            .parent()
            .ok_or_else(|| SshError::InvalidRequest("known_hosts path has no parent".into()))?;
        fs::create_dir_all(parent).map_err(|_| SshError::Internal)?;
        let _file_lock = FileLock::acquire(&self.path)?;
        let existing = fs::read_to_string(&self.path).unwrap_or_default();
        if existing.lines().any(|existing| existing == line) {
            return Ok(());
        }
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');

        let temporary = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|_| SshError::Internal)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|_| SshError::Internal)?;
            }
            if let Err(error) = file.write_all(content.as_bytes()) {
                let _ = fs::remove_file(&temporary);
                return Err(if error.kind() == std::io::ErrorKind::WriteZero {
                    SshError::Internal
                } else {
                    SshError::Internal
                });
            }
            file.sync_all().map_err(|_| SshError::Internal)?;
        }
        if let Err(error) = fs::rename(&temporary, &self.path) {
            let _ = fs::remove_file(&temporary);
            return Err(if error.kind() == std::io::ErrorKind::PermissionDenied {
                SshError::Internal
            } else {
                SshError::Internal
            });
        }
        Ok(())
    }
}

struct FileLock {
    #[cfg(unix)]
    file: File,
}

impl FileLock {
    fn acquire(path: &Path) -> Result<Self, SshError> {
        let lock_path = path.with_extension("lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|_| SshError::Internal)?;
        #[cfg(unix)]
        {
            use std::os::unix::{fs::PermissionsExt, io::AsRawFd};
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| SshError::Internal)?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(SshError::Internal);
            }
            return Ok(Self { file });
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            Ok(Self {})
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn known_host_line(host: &str, port: u16, key: &PublicKey) -> Result<String, SshError> {
    let host = if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    };
    let key = key.to_openssh().map_err(|_| SshError::KeyParse)?;
    Ok(format!("{host} {key}"))
}

fn validate_host(host: &str) -> Result<(), SshError> {
    if host.is_empty()
        || host.len() > 255
        || host
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(SshError::InvalidRequest("host is invalid".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use russh::keys::{parse_public_key_base64, PublicKey};
    use tempfile::tempdir;

    use super::KnownHostsStore;
    use crate::ssh::model::HostKeyStatus;

    fn key(value: &str) -> PublicKey {
        parse_public_key_base64(value).unwrap()
    }

    #[test]
    fn reads_hashed_and_nonstandard_port_entries() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        fs::write(
            &path,
            "[localhost]:13265 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ\n|1|O33ESRMWPVkMYIwJ1Uw+n877jTo=|nuuC5vEqXlEZ/8BXQR7m619W6Ak= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF\n",
        )
        .unwrap();
        let store = KnownHostsStore::new(path);
        assert!(store
            .classify(
                "localhost",
                13265,
                &key("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ"),
            )
            .unwrap()
            .is_none());
        assert!(store
            .classify(
                "example.com",
                22,
                &key("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF"),
            )
            .unwrap()
            .is_none());
    }

    #[test]
    fn distinguishes_unknown_and_changed_keys() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let store = KnownHostsStore::new(path);
        let recorded = key("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ");
        let replacement =
            key("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF");
        store.save("example.com", 22, &recorded).unwrap();

        assert_eq!(
            store.classify("new.example.com", 22, &replacement).unwrap(),
            Some((HostKeyStatus::Unknown, Vec::new()))
        );
        let (status, previous) = store
            .classify("example.com", 22, &replacement)
            .unwrap()
            .unwrap();
        assert_eq!(status, HostKeyStatus::Changed);
        assert_eq!(previous.len(), 1);
    }

    #[test]
    fn writes_atomically_and_deduplicates_entries() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("nested").join("known_hosts");
        let store = KnownHostsStore::new(path.clone());
        let key = key("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ");
        store.save("localhost", 2222, &key).unwrap();
        store.save("localhost", 2222, &key).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap().lines().count(), 1);
    }
}
