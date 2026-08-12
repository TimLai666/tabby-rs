use std::{collections::BTreeMap, path::Path};

use chrono::{DateTime, Utc};

use crate::error::AppError;

use super::atomic_file::{atomic_write, read_optional_regular_file};

pub const STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Nightly,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum FirstRunImportState {
    NotStarted,
    Running {
        started_at: DateTime<Utc>,
        journal: String,
    },
    Completed {
        completed_at: DateTime<Utc>,
        report: String,
    },
    Failed {
        failed_at: DateTime<Utc>,
        report: String,
    },
}

impl Default for FirstRunImportState {
    fn default() -> Self {
        Self::NotStarted
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct SafeModeState {
    pub last_forced: bool,
    pub suspected_plugins: Vec<String>,
    pub attempt_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub plugins: Vec<String>,
    pub last_started_plugin: Option<String>,
    pub last_completed_plugin: Option<String>,
    pub failure_phase: Option<String>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagnosticsState {
    pub local_logging_enabled: bool,
}

impl Default for DiagnosticsState {
    fn default() -> Self {
        Self {
            local_logging_enabled: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TabbyRsState {
    pub schema_version: u32,
    pub first_run_import: FirstRunImportState,
    pub update_channel: UpdateChannel,
    pub last_stable_backup: Option<String>,
    pub safe_mode: SafeModeState,
    pub diagnostics: DiagnosticsState,
    pub pending_plugins: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for TabbyRsState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            first_run_import: FirstRunImportState::NotStarted,
            update_channel: UpdateChannel::Stable,
            last_stable_backup: None,
            safe_mode: SafeModeState::default(),
            diagnostics: DiagnosticsState::default(),
            pending_plugins: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

pub fn load_state(path: &Path) -> Result<TabbyRsState, AppError> {
    let Some(bytes) = read_optional_regular_file(path)? else {
        return Ok(TabbyRsState::default());
    };
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(AppError::InvalidData("tabby-rs.json is too large".into()));
    }
    let state: TabbyRsState = serde_json::from_slice(&bytes)?;
    if state.schema_version > STATE_SCHEMA_VERSION {
        return Err(AppError::Unsupported(format!(
            "state schema {} requires a newer Tabby RS",
            state.schema_version
        )));
    }
    Ok(state)
}

pub fn save_state(path: &Path, state: &TabbyRsState) -> Result<(), AppError> {
    if state.schema_version > STATE_SCHEMA_VERSION {
        return Err(AppError::Unsupported(format!(
            "cannot write unknown state schema {}",
            state.schema_version
        )));
    }
    let mut bytes = serde_json::to_vec_pretty(state)?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{load_state, save_state, TabbyRsState};

    #[test]
    fn preserves_unknown_top_level_fields() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("tabby-rs.json");
        std::fs::write(
            &path,
            r#"{"schemaVersion":1,"futureFeature":{"enabled":true}}"#,
        )
        .unwrap();
        let state = load_state(&path).unwrap();
        assert_eq!(state.extra["futureFeature"]["enabled"], true);
        save_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();
        assert_eq!(reloaded.extra["futureFeature"]["enabled"], true);
    }

    #[test]
    fn creates_defaults_when_state_is_missing() {
        let temp = tempdir().unwrap();
        let state = load_state(&temp.path().join("missing.json")).unwrap();
        assert_eq!(state.schema_version, 1);
        assert!(state.pending_plugins.is_empty());
    }

    #[test]
    fn state_round_trips() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("tabby-rs.json");
        let mut state = TabbyRsState::default();
        state.safe_mode.attempt_id = Some("attempt-1".into());
        state.safe_mode.plugins = vec!["tabby-demo".into()];
        state.safe_mode.last_started_plugin = Some("tabby-demo".into());
        save_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();
        assert_eq!(reloaded.schema_version, 1);
        assert_eq!(reloaded.safe_mode.attempt_id.as_deref(), Some("attempt-1"));
        assert_eq!(reloaded.safe_mode.plugins, vec!["tabby-demo"]);
        assert_eq!(
            reloaded.safe_mode.last_started_plugin.as_deref(),
            Some("tabby-demo")
        );
    }
}
