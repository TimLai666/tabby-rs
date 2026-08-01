use std::{
    fs::{self, File},
    io::Write,
    path::Path,
};

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::error::AppError;

pub fn read_optional_regular_file(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(AppError::PermissionDenied(
            "refusing to read a symbolic link from managed storage".into(),
        ));
    }
    if !metadata.is_file() {
        return Err(AppError::InvalidData(
            "managed storage path is not a regular file".into(),
        ));
    }
    Ok(Some(fs::read(path)?))
}

pub fn read_required_regular_file(path: &Path) -> Result<Vec<u8>, AppError> {
    read_optional_regular_file(path)?.ok_or_else(|| AppError::NotFound("managed file".into()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn file_revision(path: &Path) -> Result<Option<String>, AppError> {
    Ok(read_optional_regular_file(path)?.map(|bytes| sha256_hex(&bytes)))
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::InvalidArgument("managed path has no parent".into()))?;
    fs::create_dir_all(parent)?;

    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(AppError::PermissionDenied(
                "refusing to replace a symbolic link in managed storage".into(),
            ));
        }
        if !metadata.is_file() {
            return Err(AppError::InvalidData(
                "managed storage target is not a regular file".into(),
            ));
        }
    }

    let existing_permissions = fs::metadata(path).ok().map(|metadata| metadata.permissions());
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.as_file_mut().write_all(bytes)?;
    temp.as_file_mut().flush()?;
    temp.as_file().sync_all()?;
    if let Some(permissions) = existing_permissions {
        temp.as_file().set_permissions(permissions)?;
    }
    temp.persist(path)
        .map_err(|error| AppError::Io(error.error.to_string()))?;

    #[cfg(unix)]
    File::open(parent)?.sync_all()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{atomic_write, file_revision, read_required_regular_file};

    #[test]
    fn replaces_complete_files_and_updates_revision() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        atomic_write(&path, b"version: 1\n").unwrap();
        let first_revision = file_revision(&path).unwrap().unwrap();
        atomic_write(&path, b"version: 2\n").unwrap();
        let second_revision = file_revision(&path).unwrap().unwrap();
        assert_ne!(first_revision, second_revision);
        assert_eq!(read_required_regular_file(&path).unwrap(), b"version: 2\n");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_targets() {
        let temp = tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"secret").unwrap();
        let link = temp.path().join("config.yaml");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(atomic_write(&link, b"replacement").is_err());
        assert_eq!(fs::read(outside).unwrap(), b"secret");
    }
}
