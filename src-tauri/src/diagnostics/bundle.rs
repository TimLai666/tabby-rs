use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::{
    error::AppError,
    plugins::manifest,
    state::AppState,
    storage::{
        atomic_file::read_optional_regular_file, config_file::read_config,
        paths::validate_single_component,
    },
};

use super::logging::MAX_LOG_FILE_BYTES;

const MAX_PREVIEW_BYTES: usize = 16 * 1024;
const MAX_PREVIEW_TOTAL_BYTES: usize = 64 * 1024;
const MAX_BUNDLE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFile {
    pub path: String,
    pub size: usize,
    pub content: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPreview {
    pub schema_version: u32,
    pub generated_at: String,
    pub files: Vec<PreviewFile>,
    pub redaction_warning: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleManifest {
    schema_version: u32,
    generated_at: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    path: String,
    sha256: String,
    redacted: bool,
}

struct BundleEntry {
    path: String,
    data: Vec<u8>,
    redacted: bool,
}

pub fn preview(state: &AppState, include_logs: bool) -> Result<DiagnosticsPreview, AppError> {
    let entries = collect_entries(state, include_logs)?;
    let mut remaining = MAX_PREVIEW_TOTAL_BYTES;
    let files = entries
        .into_iter()
        .map(|entry| {
            let take = entry.data.len().min(MAX_PREVIEW_BYTES).min(remaining);
            remaining -= take;
            PreviewFile {
                path: entry.path,
                size: entry.data.len(),
                content: String::from_utf8_lossy(&entry.data[..take]).into_owned(),
                redacted: entry.redacted,
            }
        })
        .collect();
    Ok(DiagnosticsPreview {
        schema_version: 1,
        generated_at: Utc::now().to_rfc3339(),
        files,
        redaction_warning: "自由文字遮蔽不是百分之百可靠，請在匯出前檢查預覽。".into(),
    })
}

pub fn export(state: &AppState, destination: &str, include_logs: bool) -> Result<String, AppError> {
    let destination = validate_destination(destination)?;
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(AppError::Conflict(
            "diagnostic bundle destination already exists".into(),
        ));
    }
    let entries = collect_entries(state, include_logs)?;
    let total = entries.iter().map(|entry| entry.data.len()).sum::<usize>();
    if total > MAX_BUNDLE_BYTES {
        return Err(AppError::InvalidData(
            "diagnostic bundle is too large".into(),
        ));
    }
    let generated_at = Utc::now().to_rfc3339();
    let manifest = BundleManifest {
        schema_version: 1,
        generated_at,
        files: entries
            .iter()
            .map(|entry| ManifestFile {
                path: entry.path.clone(),
                sha256: sha256_hex(&entry.data),
                redacted: entry.redacted,
            })
            .collect(),
    };
    let mut all_entries = entries;
    all_entries.push(BundleEntry {
        path: "manifest.json".into(),
        data: serde_json::to_vec_pretty(&manifest)?,
        redacted: true,
    });

    let parent = destination
        .parent()
        .ok_or_else(|| AppError::InvalidArgument("diagnostic destination has no parent".into()))?;
    if !parent.is_dir() {
        return Err(AppError::NotFound(
            "diagnostic destination directory was not found".into(),
        ));
    }
    let temporary = parent.join(format!(
        ".tabby-rs-diagnostics-{}-{}.tmp",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    ));
    let result = write_zip(&temporary, &all_entries).and_then(|_| {
        match fs::rename(&temporary, &destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(
                AppError::Conflict("diagnostic bundle destination already exists".into()),
            ),
            Err(error) => Err(error.into()),
        }
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|_| destination.to_string_lossy().into_owned())
}

fn collect_entries(state: &AppState, include_logs: bool) -> Result<Vec<BundleEntry>, AppError> {
    let persisted_state = state.persisted_state();
    let mut entries = vec![
        BundleEntry {
            path: "system.json".into(),
            data: serde_json::to_vec_pretty(&serde_json::json!({
                "schemaVersion": 1,
                "appVersion": env!("CARGO_PKG_VERSION"),
                "channel": match persisted_state.update_channel {
                    crate::storage::state_file::UpdateChannel::Stable => "stable",
                    crate::storage::state_file::UpdateChannel::Nightly => "nightly",
                },
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
            }))?,
            redacted: true,
        },
        BundleEntry {
            path: "config-summary.json".into(),
            data: config_summary(state)?,
            redacted: true,
        },
        BundleEntry {
            path: "plugins.json".into(),
            data: plugin_summary(state)?,
            redacted: true,
        },
    ];
    if include_logs {
        for (name, data) in read_logs(state.paths().logs_dir())? {
            entries.push(BundleEntry {
                path: format!("logs/{name}"),
                data,
                redacted: true,
            });
        }
    }
    if let Some(marker) =
        read_optional_regular_file(&crate::diagnostics::crash::path(state.paths().logs_dir()))?
    {
        entries.push(BundleEntry {
            path: "crash-marker.json".into(),
            data: marker,
            redacted: true,
        });
    }
    Ok(entries)
}

fn config_summary(state: &AppState) -> Result<Vec<u8>, AppError> {
    let config = read_config(&state.paths().data_dir().join("config.yaml"))?;
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(&config.yaml);
    let mut top_level_keys = parsed
        .ok()
        .and_then(|value| value.as_mapping().cloned())
        .map(|mapping| {
            mapping
                .keys()
                .filter_map(|key| key.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    top_level_keys.sort();
    Ok(serde_json::to_vec_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "configPresent": config.revision.is_some(),
        "topLevelKeys": top_level_keys,
    }))?)
}

fn plugin_summary(state: &AppState) -> Result<Vec<u8>, AppError> {
    let safe_mode = state.persisted_state().safe_mode;
    let plugins = manifest::list_installed(state.paths().plugins_dir())
        .unwrap_or_default()
        .into_iter()
        .map(|plugin| {
            let (load_status, failure_code) =
                plugin_diagnostic_status(&safe_mode, &plugin.package_name);
            serde_json::json!({
                "packageName": plugin.package_name,
                "version": plugin.version,
                "isBuiltin": plugin.is_builtin,
                "isLegacy": plugin.is_legacy,
                "loadStatus": load_status,
                "failureCode": failure_code,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_vec_pretty(&plugins)?)
}

fn plugin_diagnostic_status(
    safe_mode: &crate::storage::state_file::SafeModeState,
    package_name: &str,
) -> (&'static str, Option<String>) {
    if safe_mode
        .suspected_plugins
        .iter()
        .any(|suspected| suspected == package_name)
    {
        (
            "failed",
            safe_mode
                .failure_code
                .clone()
                .or_else(|| safe_mode.failure_phase.clone()),
        )
    } else {
        ("installed", None)
    }
}

fn read_logs(directory: &Path) -> Result<Vec<(String, Vec<u8>)>, AppError> {
    let Some(metadata) = fs::symlink_metadata(directory).ok() else {
        return Ok(Vec::new());
    };
    if !metadata.is_dir() {
        return Err(AppError::Io(
            "diagnostic logs path is not a directory".into(),
        ));
    }
    let redactor = crate::diagnostics::redaction::Redactor::from_storage_directory(directory);
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "tabby-rs.log" && !name.starts_with("tabby-rs.log.") {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() {
            return Err(AppError::Io(
                "diagnostic log entry is not a regular file".into(),
            ));
        }
        if metadata.len() > MAX_LOG_FILE_BYTES {
            return Err(AppError::InvalidData(
                "diagnostic log entry is too large".into(),
            ));
        }
        let bytes = read_optional_regular_file(&entry.path())?.unwrap_or_default();
        let redacted = redactor
            .redact_text(&String::from_utf8_lossy(&bytes))
            .text
            .into_bytes();
        files.push((name, redacted));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn validate_destination(value: &str) -> Result<PathBuf, AppError> {
    if value.is_empty() || value.contains('\0') {
        return Err(AppError::InvalidArgument(
            "diagnostic destination is invalid".into(),
        ));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(AppError::InvalidArgument(
            "diagnostic destination must be absolute".into(),
        ));
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            AppError::InvalidArgument("diagnostic destination has no file name".into())
        })?;
    validate_single_component(name, "diagnostic destination file name")?;
    Ok(path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_zip(path: &Path, entries: &[BundleEntry]) -> Result<(), AppError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let mut central = Vec::new();
    let mut offset = 0_u32;
    for entry in entries {
        let name = entry.path.as_bytes();
        let data = &entry.data;
        let crc = crc32(data);
        let size = u32::try_from(data.len())
            .map_err(|_| AppError::InvalidData("diagnostic entry is too large".into()))?;
        let name_len = u16::try_from(name.len())
            .map_err(|_| AppError::InvalidData("diagnostic entry path is too long".into()))?;
        let start = offset;
        write_u32(&mut file, 0x0403_4b50)?;
        write_u16(&mut file, 20)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u16(&mut file, 0)?;
        write_u32(&mut file, crc)?;
        write_u32(&mut file, size)?;
        write_u32(&mut file, size)?;
        write_u16(&mut file, name_len)?;
        write_u16(&mut file, 0)?;
        file.write_all(name)?;
        file.write_all(data)?;
        offset = offset
            .checked_add(30 + name.len() as u32 + size)
            .ok_or_else(|| AppError::InvalidData("diagnostic bundle is too large".into()))?;

        let mut record = Vec::new();
        write_u32(&mut record, 0x0201_4b50)?;
        write_u16(&mut record, 20)?;
        write_u16(&mut record, 20)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u32(&mut record, crc)?;
        write_u32(&mut record, size)?;
        write_u32(&mut record, size)?;
        write_u16(&mut record, name_len)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u16(&mut record, 0)?;
        write_u32(&mut record, 0)?;
        write_u32(&mut record, start)?;
        record.extend_from_slice(name);
        central.extend(record);
    }
    let central_offset = offset;
    file.write_all(&central)?;
    let central_size = u32::try_from(central.len())
        .map_err(|_| AppError::InvalidData("diagnostic bundle directory is too large".into()))?;
    write_u32(&mut file, 0x0605_4b50)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, 0)?;
    write_u16(&mut file, entries.len() as u16)?;
    write_u16(&mut file, entries.len() as u16)?;
    write_u32(&mut file, central_size)?;
    write_u32(&mut file, central_offset)?;
    write_u16(&mut file, 0)?;
    file.sync_all()?;
    Ok(())
}

fn write_u16(writer: &mut impl Write, value: u16) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn write_u32(writer: &mut impl Write, value: u32) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{crc32, plugin_diagnostic_status, validate_destination, write_zip, BundleEntry};
    use crate::storage::state_file::SafeModeState;

    #[test]
    fn zip_entries_use_fixed_relative_paths() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("bundle.zip");
        write_zip(
            &path,
            &[BundleEntry {
                path: "system.json".into(),
                data: br#"{"ok":true}"#.to_vec(),
                redacted: true,
            }],
        )
        .unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(crc32(br#"{"ok":true}"#), 0xa7d4_5f90);
    }

    #[test]
    fn rejects_relative_or_nested_destination_names() {
        assert!(validate_destination("bundle.zip").is_err());
        assert!(validate_destination("/tmp/../bundle.zip").is_ok());
        assert!(validate_destination("/tmp/a/bundle.zip").is_ok());
    }

    #[test]
    fn reports_plugin_failure_code_without_exposing_failure_message() {
        let state = SafeModeState {
            suspected_plugins: vec!["tabby-broken".into()],
            failure_code: Some("node-runtime-required".into()),
            failure_message: Some("require(\"fs\") is blocked".into()),
            ..SafeModeState::default()
        };

        assert_eq!(
            plugin_diagnostic_status(&state, "tabby-broken"),
            ("failed", Some("node-runtime-required".into()))
        );
        assert_eq!(
            plugin_diagnostic_status(&state, "tabby-good"),
            ("installed", None)
        );
    }
}
