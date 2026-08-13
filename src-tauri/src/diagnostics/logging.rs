use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::redaction::Redactor;

pub const LOG_FILE_NAME: &str = "tabby-rs.log";
pub const MAX_LOG_FILE_BYTES: u64 = 512 * 1024;
pub const MAX_LOG_FILES: usize = 5;
pub const MAX_LOG_BYTES: u64 = MAX_LOG_FILE_BYTES * MAX_LOG_FILES as u64;
const MAX_EVENT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub enabled: bool,
    pub directory: String,
    pub file_count: usize,
    pub bytes: u64,
    pub max_file_bytes: u64,
    pub max_files: usize,
    pub max_bytes: u64,
    pub crash_marker_present: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Log,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEvent {
    timestamp: DateTime<Utc>,
    level: LogLevel,
    target: String,
    message: String,
    fields: BTreeMap<String, serde_json::Value>,
    correlation_id: Option<String>,
}

fn parse_level(value: &str) -> Result<LogLevel, AppError> {
    match value {
        "debug" => Ok(LogLevel::Debug),
        "info" => Ok(LogLevel::Info),
        "warn" => Ok(LogLevel::Warn),
        "error" => Ok(LogLevel::Error),
        "log" => Ok(LogLevel::Log),
        _ => Err(AppError::InvalidArgument(
            "diagnostic log level is invalid".into(),
        )),
    }
}

fn redact_fields(
    redactor: &Redactor,
    fields: &BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    let value = serde_json::Value::Object(fields.clone().into_iter().collect());
    match redactor.redact_json(&value) {
        serde_json::Value::Object(fields) => fields.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

#[derive(Debug)]
pub struct LogWriter {
    directory: PathBuf,
    redactor: Redactor,
    lock: Mutex<()>,
}

impl LogWriter {
    pub fn from_environment(directory: impl Into<PathBuf>) -> Self {
        Self::from_storage_directory_with_secrets(directory, &[])
    }

    pub fn from_storage_directory_with_secrets(
        directory: impl Into<PathBuf>,
        known_secrets: &[String],
    ) -> Self {
        let directory = directory.into();
        Self::new(
            &directory,
            Redactor::from_storage_directory_with_secrets(&directory, known_secrets),
        )
    }

    pub fn new(directory: impl Into<PathBuf>, redactor: Redactor) -> Self {
        Self {
            directory: directory.into(),
            redactor,
            lock: Mutex::new(()),
        }
    }

    pub fn append(
        &self,
        level: &str,
        target: &str,
        message: &str,
        fields: &BTreeMap<String, serde_json::Value>,
        correlation_id: Option<&str>,
    ) -> Result<(), AppError> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let event = LogEvent {
            timestamp: Utc::now(),
            level: parse_level(level)?,
            target: self.redactor.redact_text(target).text,
            message: self.redactor.redact_text(message).text,
            fields: redact_fields(&self.redactor, fields),
            correlation_id: correlation_id.map(str::to_owned),
        };
        let mut bytes = serde_json::to_vec(&event)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_EVENT_BYTES {
            return Err(AppError::InvalidArgument(
                "diagnostic log event is too large".into(),
            ));
        }
        fs::create_dir_all(&self.directory)?;
        self.rotate_if_needed(bytes.len() as u64)?;
        let path = self.current_path();
        reject_symlink(&path)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        Ok(())
    }

    pub fn status(&self, enabled: bool) -> Result<LogStatus, AppError> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(LogStatus {
            enabled,
            directory: self.directory.to_string_lossy().into_owned(),
            file_count: self.files()?.len(),
            bytes: self.total_bytes()?,
            max_file_bytes: MAX_LOG_FILE_BYTES,
            max_files: MAX_LOG_FILES,
            max_bytes: MAX_LOG_BYTES,
            crash_marker_present: super::crash::exists(&self.directory),
        })
    }

    pub fn clear(&self) -> Result<(), AppError> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.directory.exists() {
            return Ok(());
        }
        for path in self.files()? {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn current_path(&self) -> PathBuf {
        self.directory.join(LOG_FILE_NAME)
    }

    fn archived_path(&self, index: usize) -> PathBuf {
        self.directory.join(format!("{LOG_FILE_NAME}.{index}"))
    }

    fn rotate_if_needed(&self, incoming_bytes: u64) -> Result<(), AppError> {
        let current = self.current_path();
        let current_bytes = match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(AppError::Io("diagnostic log is not a regular file".into()));
                }
                metadata.len()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        if current_bytes == 0 || current_bytes + incoming_bytes <= MAX_LOG_FILE_BYTES {
            return Ok(());
        }
        for index in (1..MAX_LOG_FILES).rev() {
            let source = if index == 1 {
                current.clone()
            } else {
                self.archived_path(index - 1)
            };
            let destination = self.archived_path(index);
            if source.exists() {
                reject_symlink(&source)?;
                if destination.exists() {
                    reject_symlink(&destination)?;
                    fs::remove_file(&destination)?;
                }
                fs::rename(source, destination)?;
            }
        }
        Ok(())
    }

    fn files(&self) -> Result<Vec<PathBuf>, AppError> {
        let mut files = Vec::new();
        let current = self.current_path();
        if current.exists() {
            files.push(current);
        }
        for index in 1..MAX_LOG_FILES {
            let path = self.archived_path(index);
            if path.exists() {
                files.push(path);
            }
        }
        Ok(files)
    }

    fn total_bytes(&self) -> Result<u64, AppError> {
        self.files()?
            .into_iter()
            .map(|path| Ok(fs::symlink_metadata(path)?.len()))
            .try_fold(0_u64, |total, bytes| bytes.map(|bytes| total + bytes))
    }
}

fn reject_symlink(path: &Path) -> Result<(), AppError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() {
            return Err(AppError::Io(
                "diagnostic log path is not a regular file".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use std::collections::BTreeMap;

    use super::{LogWriter, MAX_LOG_FILES};
    use crate::diagnostics::crash::{clear, mark_startup};
    use crate::diagnostics::redaction::Redactor;

    fn writer(path: &std::path::Path) -> LogWriter {
        LogWriter::new(path, Redactor::default())
    }

    #[test]
    fn rotates_files_and_keeps_the_bound() {
        let temp = tempdir().unwrap();
        let writer = writer(temp.path());
        let message = "x".repeat(64 * 1024);
        for _ in 0..(MAX_LOG_FILES * 9) {
            writer
                .append("info", "test", &message, &BTreeMap::new(), None)
                .unwrap();
        }
        let status = writer.status(true).unwrap();
        assert!(status.file_count <= MAX_LOG_FILES);
        assert!(status.bytes <= status.max_bytes);
        assert!(temp.path().join("tabby-rs.log.1").is_file());
    }

    #[test]
    fn redacts_event_before_writing() {
        let temp = tempdir().unwrap();
        let writer = LogWriter::new(
            temp.path(),
            Redactor::new(crate::diagnostics::redaction::RedactionContext {
                known_secrets: vec!["top-secret".into()],
                ..Default::default()
            }),
        );
        writer
            .append(
                "error",
                "test",
                "token=top-secret",
                &BTreeMap::from([(
                    "password".into(),
                    serde_json::Value::String("top-secret".into()),
                )]),
                None,
            )
            .unwrap();
        let contents = std::fs::read_to_string(temp.path().join("tabby-rs.log")).unwrap();
        assert!(!contents.contains("top-secret"));
        assert!(contents.contains("<SECRET>"));
        assert!(contents.contains("<REDACTED>"));
    }

    #[test]
    fn status_reports_crash_marker_state() {
        let temp = tempdir().unwrap();
        let writer = writer(temp.path());

        assert!(!writer.status(true).unwrap().crash_marker_present);
        mark_startup(temp.path()).unwrap();
        assert!(writer.status(true).unwrap().crash_marker_present);
        clear(temp.path()).unwrap();
        assert!(!writer.status(true).unwrap().crash_marker_present);
    }

    #[test]
    fn redacts_source_before_writing() {
        let temp = tempdir().unwrap();
        let writer = LogWriter::new(
            temp.path(),
            Redactor::new(crate::diagnostics::redaction::RedactionContext {
                hosts: vec!["server.internal".into()],
                ..Default::default()
            }),
        );
        writer
            .append(
                "info",
                "ssh-server.internal-22",
                "connected",
                &BTreeMap::new(),
                None,
            )
            .unwrap();
        let contents = std::fs::read_to_string(temp.path().join("tabby-rs.log")).unwrap();
        assert!(!contents.contains("server.internal"));
        assert!(contents.contains("ssh-<HOST:1>-22"));
    }

    #[test]
    fn rejects_oversized_events() {
        let temp = tempdir().unwrap();
        let writer = writer(temp.path());
        let error = writer
            .append(
                "info",
                "test",
                &"x".repeat(128 * 1024),
                &BTreeMap::new(),
                None,
            )
            .unwrap_err();
        assert!(error.to_string().contains("too large"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_log_files() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        std::fs::write(&target, "private").unwrap();
        std::os::unix::fs::symlink(&target, temp.path().join("tabby-rs.log")).unwrap();
        let error = writer(temp.path())
            .append("info", "test", "message", &BTreeMap::new(), None)
            .unwrap_err();
        assert!(error.to_string().contains("regular file"));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "private");
    }

    #[test]
    fn writes_the_typed_log_event_contract() {
        let temp = tempdir().unwrap();
        let writer = writer(temp.path());
        writer
            .append(
                "warn",
                "ssh-session",
                "disconnected",
                &BTreeMap::from([("durationMs".into(), serde_json::json!(12))]),
                Some("corr-1"),
            )
            .unwrap();
        let line = std::fs::read_to_string(temp.path().join("tabby-rs.log")).unwrap();
        let event: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert!(event["timestamp"].as_str().is_some());
        assert_eq!(event["level"], "warn");
        assert_eq!(event["target"], "ssh-session");
        assert_eq!(event["correlationId"], "corr-1");
        assert_eq!(event["fields"]["durationMs"], 12);
        assert!(event.get("source").is_none());
    }
}
