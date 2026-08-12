use crate::{
    error::AppError,
    storage::{
        atomic_file::read_optional_regular_file,
        backup::restore_backup,
        paths::StoragePaths,
        state_file::{save_state, TabbyRsState},
    },
};

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
        state = crate::storage::state_file::load_state(paths.state_file())?;
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
    let value: serde_yaml::Value = match serde_yaml::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    Ok(matches!(value, serde_yaml::Value::Mapping(_)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::recover_pending_update;
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
}
