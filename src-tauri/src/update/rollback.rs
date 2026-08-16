use std::collections::BTreeMap;

use crate::{
    error::AppError,
    storage::{
        atomic_file::{atomic_write, read_optional_regular_file},
        backup::restore_backup,
        paths::StoragePaths,
        state_file::{load_state, save_state, PendingUpdateState, TabbyRsState},
    },
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdateJournal {
    pub target_version: String,
    pub backup_id: String,
    pub channel: crate::storage::state_file::UpdateChannel,
}

/// Keep the last known Stable snapshot available while a Nightly-to-Stable
/// transition is being validated. The current update backup may contain
/// Nightly data even after the channel has been switched to Stable.
pub fn remember_stable_backup(state: &mut TabbyRsState, backup_id: &str) {
    if state.last_stable_backup.is_none() {
        state.last_stable_backup = Some(backup_id.to_owned());
    }
}

pub fn write_pending_update_journal(
    paths: &StoragePaths,
    pending: &PendingUpdateState,
) -> Result<(), AppError> {
    let journal = PendingUpdateJournal {
        target_version: pending.target_version.clone(),
        backup_id: pending.backup_id.clone(),
        channel: pending.channel.clone(),
    };
    let mut bytes = serde_json::to_vec_pretty(&journal)?;
    bytes.push(b'\n');
    atomic_write(paths.pending_update_file(), &bytes)
}

pub fn clear_pending_update_journal(paths: &StoragePaths) -> Result<(), AppError> {
    match std::fs::symlink_metadata(paths.pending_update_file()) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::PermissionDenied(
            "refusing to remove a symbolic link from managed storage".into(),
        )),
        Ok(metadata) if !metadata.is_file() => Err(AppError::InvalidData(
            "pending update journal is not a regular file".into(),
        )),
        Ok(_) => {
            std::fs::remove_file(paths.pending_update_file())?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn recover_pending_update_from_disk(
    paths: &StoragePaths,
    current_version: &str,
) -> Result<TabbyRsState, AppError> {
    let journal = read_optional_regular_file(paths.pending_update_file())?
        .map(|bytes| serde_json::from_slice::<PendingUpdateJournal>(&bytes))
        .transpose()?;
    let state_result = load_state(paths.state_file());

    let Some(journal) = journal else {
        return recover_pending_update(paths, state_result?, current_version);
    };

    let state_is_corrupt = state_result.is_err();
    let state = match state_result {
        Ok(state) => state,
        Err(_error) if journal.target_version == current_version => TabbyRsState::default(),
        Err(error) => return Err(error),
    };
    let recovered = recover_journal(paths, state, &journal, current_version, state_is_corrupt)?;
    clear_pending_update_journal(paths)?;
    Ok(recovered)
}

fn recover_journal(
    paths: &StoragePaths,
    mut state: TabbyRsState,
    journal: &PendingUpdateJournal,
    current_version: &str,
    state_is_corrupt: bool,
) -> Result<TabbyRsState, AppError> {
    if journal.target_version == current_version
        && (state_is_corrupt || !config_is_readable(paths)?)
    {
        restore_backup(paths, &journal.backup_id)?;
        state = load_state(paths.state_file())?;
        state.extra.insert(
            "lastUpdateRecovery".into(),
            serde_json::json!({
                "reason": if state_is_corrupt { "state-incompatible" } else { "config-incompatible" },
                "backupId": journal.backup_id,
                "targetVersion": journal.target_version,
            }),
        );
    }
    state.pending_update = None;
    save_state(paths.state_file(), &state)?;
    Ok(state)
}

pub fn recover_pending_update(
    paths: &StoragePaths,
    mut state: TabbyRsState,
    current_version: &str,
) -> Result<TabbyRsState, AppError> {
    let Some(pending) = state.pending_update.clone() else {
        return Ok(state);
    };

    if pending.target_version == current_version && !config_is_readable(paths)? {
        let backup_id = if matches!(
            pending.channel,
            crate::storage::state_file::UpdateChannel::Stable
        ) {
            state
                .last_stable_backup
                .clone()
                .unwrap_or_else(|| pending.backup_id.clone())
        } else {
            pending.backup_id.clone()
        };
        restore_backup(paths, &backup_id)?;
        state = load_state(paths.state_file())?;
        state.extra.insert(
            "lastUpdateRecovery".into(),
            serde_json::json!({
                "reason": "config-incompatible",
                "backupId": backup_id,
                "targetVersion": pending.target_version,
            }),
        );
        save_state(paths.state_file(), &state)?;
        return Ok(state);
    }

    state.pending_update = None;
    save_state(paths.state_file(), &state)?;
    Ok(state)
}

fn config_is_readable(paths: &StoragePaths) -> Result<bool, AppError> {
    let Some(bytes) = read_optional_regular_file(paths.config_file())? else {
        return Ok(true);
    };
    if bytes.is_empty() {
        return Ok(true);
    }
    let value: BTreeMap<String, serde_json::Value> = match serde_yaml::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(!value.is_empty() || bytes.iter().all(u8::is_ascii_whitespace))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::{
        clear_pending_update_journal, recover_pending_update, recover_pending_update_from_disk,
        remember_stable_backup, write_pending_update_journal,
    };
    use crate::storage::{
        atomic_file::atomic_write,
        backup::{create_backup, BackupRequest},
        paths::StoragePaths,
        state_file::{save_state, PendingUpdateState, TabbyRsState, UpdateChannel},
    };

    #[test]
    fn restores_before_update_backup_when_new_config_is_incompatible() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: backup.backup_id.clone(),
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["backupId"],
            backup.backup_id
        );
    }

    #[test]
    fn clears_stale_marker_when_old_version_remains_available() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: "backup".into(),
            channel: UpdateChannel::Stable,
        });
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.1").unwrap();
        assert!(recovered.pending_update.is_none());
    }

    #[test]
    fn restores_when_config_is_yaml_but_not_a_bootstrap_mapping() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"- valid-yaml-but-not-a-config-map\n").unwrap();

        let mut state = TabbyRsState::default();
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.2".into(),
            backup_id: backup.backup_id.clone(),
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
    }

    #[test]
    fn stable_recovery_prefers_last_stable_backup_over_nightly_update_backup() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"channel: stable\n").unwrap();
        let stable_backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-nightly".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();

        atomic_write(paths.config_file(), b"channel: nightly\n").unwrap();
        let nightly_backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-stable".into(),
                source_version: Some("1.0.231-tabbyrs.2.nightly.20260812.1".into()),
                channel: Some(UpdateChannel::Nightly),
            },
            "1.0.231-tabbyrs.2.nightly.20260812.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();

        let mut state = TabbyRsState::default();
        state.last_stable_backup = Some(stable_backup.backup_id.clone());
        state.pending_update = Some(PendingUpdateState {
            target_version: "1.0.231-tabbyrs.3".into(),
            backup_id: nightly_backup.backup_id,
            channel: UpdateChannel::Stable,
        });
        state.extra = BTreeMap::new();
        save_state(paths.state_file(), &state).unwrap();

        let recovered = recover_pending_update(&paths, state, "1.0.231-tabbyrs.3").unwrap();
        assert_eq!(
            std::fs::read(paths.config_file()).unwrap(),
            b"channel: stable\n"
        );
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["backupId"],
            stable_backup.backup_id
        );
    }

    #[test]
    fn stable_update_does_not_replace_last_stable_backup_with_nightly_snapshot() {
        let mut state = TabbyRsState::default();
        state.last_stable_backup = Some("stable-before-nightly".into());

        remember_stable_backup(&mut state, "nightly-before-stable");

        assert_eq!(
            state.last_stable_backup.as_deref(),
            Some("stable-before-nightly")
        );
    }

    #[test]
    fn journal_recovers_when_state_file_is_corrupt() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        atomic_write(paths.state_file(), b"{broken").unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: backup.backup_id.clone(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(std::fs::read(paths.config_file()).unwrap(), b"version: 1\n");
        assert!(!paths.pending_update_file().exists());
        assert_eq!(
            recovered.extra["lastUpdateRecovery"]["reason"],
            "state-incompatible"
        );
    }

    #[test]
    fn journal_recovers_when_state_schema_is_newer() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 1\n").unwrap();
        let backup = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some("1.0.231-tabbyrs.1".into()),
                channel: Some(UpdateChannel::Stable),
            },
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        atomic_write(paths.config_file(), b"invalid: [\n").unwrap();
        atomic_write(paths.state_file(), br#"{"schemaVersion":99}"#).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: backup.backup_id,
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn journal_is_cleared_after_successful_restart() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: 2\n").unwrap();
        save_state(paths.state_file(), &TabbyRsState::default()).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: "unused".into(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").unwrap();
        assert!(recovered.pending_update.is_none());
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn old_version_clears_install_failure_marker_without_restoring() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.config_file(), b"version: old\n").unwrap();
        save_state(paths.state_file(), &TabbyRsState::default()).unwrap();
        write_pending_update_journal(
            &paths,
            &PendingUpdateState {
                target_version: "1.0.231-tabbyrs.2".into(),
                backup_id: "unused".into(),
                channel: UpdateChannel::Stable,
            },
        )
        .unwrap();

        let recovered = recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.1").unwrap();
        assert!(recovered.pending_update.is_none());
        assert_eq!(
            std::fs::read(paths.config_file()).unwrap(),
            b"version: old\n"
        );
        assert!(!paths.pending_update_file().exists());
    }

    #[test]
    fn malformed_journal_is_preserved_for_diagnosis() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        atomic_write(paths.pending_update_file(), b"{broken").unwrap();

        assert!(recover_pending_update_from_disk(&paths, "1.0.231-tabbyrs.2").is_err());
        assert!(paths.pending_update_file().exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_remove_a_journal_symlink() {
        let temp = tempdir().unwrap();
        let paths = StoragePaths::from_data_dir(temp.path().join("data"));
        paths.ensure_layout().unwrap();
        let outside = temp.path().join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, paths.pending_update_file()).unwrap();

        assert!(clear_pending_update_journal(&paths).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }
}
