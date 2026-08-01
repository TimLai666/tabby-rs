use std::path::Path;

use crate::error::AppError;

use super::atomic_file::{atomic_write, file_revision, read_optional_regular_file, sha256_hex};

const MAX_CONFIG_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigReadResult {
    pub yaml: String,
    pub revision: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWriteRequest {
    pub yaml: String,
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub require_missing: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWriteResult {
    pub revision: String,
    pub path: String,
}

pub fn read_config(path: &Path) -> Result<ConfigReadResult, AppError> {
    let bytes = read_optional_regular_file(path)?;
    let (yaml, revision) = match bytes {
        Some(bytes) => {
            if bytes.len() > MAX_CONFIG_BYTES {
                return Err(AppError::InvalidData("config.yaml is too large".into()));
            }
            let revision = sha256_hex(&bytes);
            let yaml = String::from_utf8(bytes)
                .map_err(|_| AppError::InvalidData("config.yaml is not valid UTF-8".into()))?;
            (yaml, Some(revision))
        }
        None => (String::new(), None),
    };
    Ok(ConfigReadResult {
        yaml,
        revision,
        path: path.to_string_lossy().into_owned(),
    })
}

pub fn write_config(
    path: &Path,
    request: &ConfigWriteRequest,
) -> Result<ConfigWriteResult, AppError> {
    if request.yaml.len() > MAX_CONFIG_BYTES {
        return Err(AppError::InvalidArgument("config.yaml is too large".into()));
    }
    if request.yaml.contains('\0') {
        return Err(AppError::InvalidArgument(
            "config.yaml contains a NUL character".into(),
        ));
    }

    let current_revision = file_revision(path)?;
    if request.require_missing && current_revision.is_some() {
        return Err(AppError::Conflict(
            "config.yaml was created by another process".into(),
        ));
    }
    if let Some(expected) = request.expected_revision.as_deref() {
        if current_revision.as_deref() != Some(expected) {
            return Err(AppError::Conflict(
                "config.yaml changed since it was loaded".into(),
            ));
        }
    }

    atomic_write(path, request.yaml.as_bytes())?;
    Ok(ConfigWriteResult {
        revision: sha256_hex(request.yaml.as_bytes()),
        path: path.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{read_config, write_config, ConfigWriteRequest};

    #[test]
    fn detects_revision_conflicts() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        let first = write_config(
            &path,
            &ConfigWriteRequest {
                yaml: "version: 1\n".into(),
                expected_revision: None,
                require_missing: true,
            },
        )
        .unwrap();
        std::fs::write(&path, "version: external\n").unwrap();
        let conflict = write_config(
            &path,
            &ConfigWriteRequest {
                yaml: "version: 2\n".into(),
                expected_revision: Some(first.revision),
                require_missing: false,
            },
        );
        assert!(conflict.is_err());
        assert_eq!(read_config(&path).unwrap().yaml, "version: external\n");
    }

    #[test]
    fn keeps_yaml_text_unchanged() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("config.yaml");
        let yaml = "version: 1\npluginField:\n  unknown: [1, 2, 3]\n";
        write_config(
            &path,
            &ConfigWriteRequest {
                yaml: yaml.into(),
                expected_revision: None,
                require_missing: true,
            },
        )
        .unwrap();
        assert_eq!(read_config(&path).unwrap().yaml, yaml);
    }
}
