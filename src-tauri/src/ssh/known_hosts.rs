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
        let _guard = self.lock.lock().unwrap_or_else(|error| error.into_inner());
        let parent = self
            .path
            .parent()
            .ok_or_else(|| SshError::InvalidRequest("known_hosts path has no parent".into()))?;
        fs::create_dir_all(parent).map_err(|_| SshError::Internal)?;
        let _file_lock = FileLock::acquire(&self.path)?;
        let existing = fs::read_to_string(&self.path).unwrap_or_default();
        let existing_lines = existing.lines().collect::<Vec<_>>();
        let matching_lines =
            known_host_keys_path(host, port, &self.path).map_err(|_| SshError::Internal)?;
        let matching_line_numbers = matching_lines
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>();
        let matching_host_tokens = matching_line_numbers
            .iter()
            .filter_map(|line| {
                existing_lines
                    .get(line.saturating_sub(1))
                    .and_then(|value| value.split_ascii_whitespace().next())
            })
            .collect::<Vec<_>>();
        let default_host_token = host_token(host, port);
        let replacement_host_token = matching_host_tokens
            .iter()
            .find(|token| token.starts_with("|1|"))
            .or_else(|| matching_host_tokens.first())
            .copied()
            .unwrap_or(default_host_token.as_str());
        let mut content = existing_lines
            .iter()
            .enumerate()
            .filter(|(index, _)| !matching_line_numbers.contains(&(index + 1)))
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&known_host_line_for_token(replacement_host_token, key)?);
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
        if let Err(error) = replace_file(&temporary, &self.path) {
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

fn replace_file(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let temporary = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(());
    }
    #[cfg(not(windows))]
    fs::rename(temporary, destination)
}

struct FileLock {
    #[cfg(any(unix, windows))]
    file: File,
}

impl FileLock {
    fn acquire(path: &Path) -> Result<Self, SshError> {
        let lock_path = path.with_extension("lock");
        let file = {
            #[cfg(unix)]
            {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&lock_path)
                    .map_err(|_| SshError::Internal)?
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .share_mode(0)
                    .open(&lock_path)
                    .map_err(|_| SshError::Internal)?
            }
            #[cfg(not(any(unix, windows)))]
            {
                OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&lock_path)
                    .map_err(|_| SshError::Internal)?
            }
        };
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
        #[cfg(windows)]
        {
            return Ok(Self { file });
        }
        #[cfg(not(any(unix, windows)))]
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

fn host_token(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_owned()
    } else {
        format!("[{host}]:{port}")
    }
}

fn known_host_line_for_token(host: &str, key: &PublicKey) -> Result<String, SshError> {
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

    #[test]
    fn replacing_a_changed_key_removes_the_old_trusted_key() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let store = KnownHostsStore::new(path.clone());
        let first = key("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ");
        let second = key("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF");

        store.save("example.com", 22, &first).unwrap();
        store.save("example.com", 22, &second).unwrap();

        assert!(store.classify("example.com", 22, &first).unwrap().is_some());
        assert!(store
            .classify("example.com", 22, &second)
            .unwrap()
            .is_none());
        let content = fs::read_to_string(path).unwrap();
        assert!(!content
            .contains("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ"));
        assert!(content
            .contains("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF"));
    }

    #[test]
    fn replacing_a_hashed_host_preserves_the_hashed_host_token() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        fs::write(
            &path,
            "|1|O33ESRMWPVkMYIwJ1Uw+n877jTo=|nuuC5vEqXlEZ/8BXQR7m619W6Ak= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ\nother.example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF\n",
        )
        .unwrap();
        let store = KnownHostsStore::new(path.clone());
        let replacement =
            key("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF");

        store.save("example.com", 22, &replacement).unwrap();

        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("|1|O33ESRMWPVkMYIwJ1Uw+n877jTo=|nuuC5vEqXlEZ/8BXQR7m619W6Ak="));
        assert!(content.contains("other.example"));
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn replaces_an_existing_file_when_adding_a_new_host() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("known_hosts");
        let store = KnownHostsStore::new(path.clone());
        let first = key("AAAAC3NzaC1lZDI1NTE5AAAAIJdD7y3aLq454yWBdwLWbieU1ebz9/cu7/QEXn9OIeZJ");
        let second = key("AAAAC3NzaC1lZDI1NTE5AAAAILIG2T/B0l0gaqj3puu510tu9N1OkQ4znY3LYuEm5zCF");

        store.save("localhost", 22, &first).unwrap();
        store.save("example.com", 22, &second).unwrap();

        let content = fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 2);
        assert!(content.contains("localhost"));
        assert!(content.contains("example.com"));
    }
}
