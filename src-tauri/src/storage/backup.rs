use std::{fs, path::Path};

use chrono::{DateTime, Utc};

use crate::error::AppError;

use super::{
    atomic_file::{atomic_write, read_optional_regular_file, read_required_regular_file, sha256_hex},
    paths::StoragePaths,
    state_file::UpdateChannel,
};

const BACKUP_SCHEMA_VERSION: u32 = 1;
const MAX_BACKUPS: usize = 20;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRequest {
    pub reason: String,
    #[serde(default)]
    pub source_version: Option<String>,
    #[serde(default)]
    pub channel: Option<UpdateChannel>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub schema_version: u32,
    pub backup_id: String,
    pub created_at: DateTime<Utc>,
    pub reason: String,
    pub source_version: String,
    pub channel: UpdateChannel,
    pub files: Vec<BackupFile>,
    #[serde(default)]
    pub absent: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub backup_id: String,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
}

pub fn create_backup(
    paths: &StoragePaths,
    request: &BackupRequest,
    app_version: &str,
) -> Result<BackupManifest, AppError> {
    paths.ensure_layout()?;
    let reason = normalized_reason(&request.reason)?;
    let created_at = Utc::now();
    let backup_id = format!(
        "{}-{}-{}",
        created_at.timestamp_millis(),
        std::process::id(),
        reason
    );
    let backup_dir = paths.backup_dir(&backup_id)?;
    fs::create_dir(&backup_dir)?;
    let files_dir = backup_dir.join("files");
    fs::create_dir(&files_dir)?;

    let mut files = Vec::new();
    let mut absent = Vec::new();
    for (relative, source) in [
        ("config.yaml", paths.config_file()),
        ("tabby-rs.json", paths.state_file()),
    ] {
        let Some(bytes) = read_optional_regular_file(source)? else {
            absent.push(relative.into());
            continue;
        };
        atomic_write(&files_dir.join(relative), &bytes)?;
        files.push(BackupFile {
            path: relative.into(),
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
        });
    }

    let manifest = BackupManifest {
        schema_version: BACKUP_SCHEMA_VERSION,
        backup_id: backup_id.clone(),
        created_at,
        reason: request.reason.trim().into(),
        source_version: request
            .source_version
            .clone()
            .unwrap_or_else(|| app_version.to_owned()),
        channel: request.channel.clone().unwrap_or_default(),
        files,
        absent,
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    atomic_write(&backup_dir.join("manifest.json"), &manifest_bytes)?;
    prune_backups(paths, &backup_id)?;
    Ok(manifest)
}

pub fn list_backups(paths: &StoragePaths) -> Result<Vec<BackupManifest>, AppError> {
    paths.ensure_layout()?;
    let mut manifests = Vec::new();
    for entry in fs::read_dir(paths.backups_dir())? {
        let entry = entry?;
        let metadata = entry.file_type()?;
        if !metadata.is_dir() || metadata.is_symlink() {
            continue;
        }
        let manifest_path = entry.path().join("manifest.json");
        let Some(bytes) = read_optional_regular_file(&manifest_path)? else {
            continue;
        };
        let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
        if manifest.schema_version != BACKUP_SCHEMA_VERSION {
            continue;
        }
        manifests.push(manifest);
    }
    manifests.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(manifests)
}

pub fn restore_backup(
    paths: &StoragePaths,
    backup_id: &str,
) -> Result<RestoreReport, AppError> {
    let backup_dir = paths.backup_dir(backup_id)?;
    let manifest_bytes = read_required_regular_file(&backup_dir.join("manifest.json"))?;
    let manifest: BackupManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != BACKUP_SCHEMA_VERSION || manifest.backup_id != backup_id {
        return Err(AppError::InvalidData("backup manifest identity mismatch".into()));
    }

    let mut prepared = Vec::new();
    for file in &manifest.files {
        let target = target_for_relative(paths, &file.path)?;
        let source = backup_dir.join("files").join(&file.path);
        let bytes = read_required_regular_file(&source)?;
        if bytes.len() as u64 != file.size || sha256_hex(&bytes) != file.sha256 {
            return Err(AppError::InvalidData(format!(
                "backup checksum mismatch for {}",
                file.path
            )));
        }
        prepared.push((file.path.clone(), target.to_path_buf(), bytes));
    }
    let removals = manifest
        .absent
        .iter()
        .map(|relative| {
            target_for_relative(paths, relative).map(|target| (relative.clone(), target.to_path_buf()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut restored = Vec::new();
    for (relative, target, bytes) in prepared {
        atomic_write(&target, &bytes)?;
        restored.push(relative);
    }
    let mut removed = Vec::new();
    for (relative, target) in removals {
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(AppError::PermissionDenied(
                    "refusing to remove a symbolic link during restore".into(),
                ));
            }
            Ok(metadata) if metadata.is_file() => {
                fs::remove_file(target)?;
                removed.push(relative);
            }
            Ok(_) => {
                return Err(AppError::InvalidData(
                    "restore target is not a regular file".into(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(RestoreReport {
        backup_id: backup_id.into(),
        restored,
        removed,
    })
}

fn target_for_relative<'a>(
    paths: &'a StoragePaths,
    relative: &str,
) -> Result<&'a Path, AppError> {
    match relative {
        "config.yaml" => Ok(paths.config_file()),
        "tabby-rs.json" => Ok(paths.state_file()),
        _ => Err(AppError::InvalidData(
            "backup manifest contains an unmanaged path".into(),
        )),
    }
}

fn normalized_reason(reason: &str) -> Result<String, AppError> {
    let trimmed = reason.trim();
    if trimmed.is_empty() || trimmed.len() > 120 || trimmed.chars().any(char::is_control) {
        return Err(AppError::InvalidArgument("invalid backup reason".into()));
    }
    let mut slug = String::new();
    for character in trimmed.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    Ok(if slug.is_empty() { "backup" } else { slug }.into())
}

fn prune_backups(paths: &StoragePaths, keep_id: &str) -> Result<(), AppError> {
    let mut directories = fs::read_dir(paths.backups_dir())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .map(|kind| kind.is_dir() && !kind.is_symlink())
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    directories.sort_by_key(|entry| entry.file_name());
    let remove_count = directories.len().saturating_sub(MAX_BACKUPS);
    for entry in directories.into_iter().take(remove_count) {
        if entry.file_name().to_string_lossy() == keep_id {
            continue;
        }
        fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{create_backup, restore_backup, BackupRequest};
    use crate::storage::{atomic_file::atomic_write, paths::StoragePaths};

    #[test]
    fn verifies_and_restores_managed_files() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        atomic_write(paths.state_file(), b"{\"schemaVersion\":1}\n").unwrap();
        let manifest = create_backup(
            &paths,
            &BackupRequest {
                reason: "manual test".into(),
                source_version: None,
                channel: None,
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"version: 2\n").unwrap();
        restore_backup(&paths, &manifest.backup_id).unwrap();
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
    }

    #[test]
    fn removes_files_that_were_absent_at_backup_time() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let manifest = create_backup(
            &paths,
            &BackupRequest {
                reason: "empty snapshot".into(),
                source_version: None,
                channel: None,
            },
            "test",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"new file").unwrap();
        let report = restore_backup(&paths, &manifest.backup_id).unwrap();
        assert!(report.removed.contains(&"config.yaml".into()));
        assert!(!paths.config_file().exists());
    }

    #[test]
    fn refuses_checksum_mismatches_before_restore() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let manifest = create_backup(
            &paths,
            &BackupRequest {
                reason: "tamper test".into(),
                source_version: None,
                channel: None,
            },
            "test",
        )
        .unwrap();
        std::fs::write(
            paths
                .backup_dir(&manifest.backup_id)
                .unwrap()
                .join("files/config.yaml"),
            b"tampered",
        )
        .unwrap();
        assert!(restore_backup(&paths, &manifest.backup_id).is_err());
    }
}
