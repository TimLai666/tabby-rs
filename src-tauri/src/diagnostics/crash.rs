use std::{
    fs,
    panic::PanicHookInfo,
    path::{Path, PathBuf},
};

use chrono::Utc;

use crate::{error::AppError, storage::atomic_file::atomic_write};

pub const CRASH_MARKER_FILE: &str = "crash-marker.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrashMarker {
    pub started_at: String,
    pub pid: u32,
    pub kind: String,
}

pub fn path(directory: &Path) -> PathBuf {
    directory.join(CRASH_MARKER_FILE)
}

pub fn mark_startup(directory: &Path) -> Result<(), AppError> {
    let marker_path = path(directory);
    if fs::symlink_metadata(&marker_path).is_ok() {
        return Ok(());
    }
    let marker = CrashMarker {
        started_at: Utc::now().to_rfc3339(),
        pid: std::process::id(),
        kind: "unclean-startup".into(),
    };
    atomic_write(&marker_path, &serde_json::to_vec_pretty(&marker)?)
}

pub fn mark_panic(directory: &Path) -> Result<(), AppError> {
    let marker = CrashMarker {
        started_at: Utc::now().to_rfc3339(),
        pid: std::process::id(),
        kind: "panic".into(),
    };
    atomic_write(&path(directory), &serde_json::to_vec_pretty(&marker)?)
}

pub fn clear(directory: &Path) -> Result<(), AppError> {
    let marker_path = path(directory);
    match fs::symlink_metadata(&marker_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            "refusing to remove a symbolic-link crash marker".into(),
        )),
        Ok(metadata) if metadata.is_file() => {
            fs::remove_file(marker_path)?;
            Ok(())
        }
        Ok(_) => Err(AppError::InvalidData(
            "crash marker is not a regular file".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn exists(directory: &Path) -> bool {
    fs::symlink_metadata(path(directory))
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

pub fn install_panic_hook(directory: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        let _ = mark_panic(&directory);
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{clear, exists, mark_panic, mark_startup, path};

    #[test]
    fn preserves_marker_until_clean_shutdown() {
        let temp = tempdir().unwrap();
        mark_startup(temp.path()).unwrap();
        assert!(exists(temp.path()));
        let first = std::fs::read_to_string(path(temp.path())).unwrap();
        mark_startup(temp.path()).unwrap();
        assert_eq!(first, std::fs::read_to_string(path(temp.path())).unwrap());
        clear(temp.path()).unwrap();
        assert!(!exists(temp.path()));
    }

    #[test]
    fn panic_marker_is_explicitly_typed() {
        let temp = tempdir().unwrap();
        mark_panic(temp.path()).unwrap();
        let marker: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path(temp.path())).unwrap()).unwrap();
        assert_eq!(marker["kind"], "panic");
        assert!(marker.get("message").is_none());
    }
}
