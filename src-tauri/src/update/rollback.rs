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
        restore_backup(paths, &pending.backup_id)?;
        state = crate::storage::state_file::load_state(paths.state_file())?;
        state.extra.insert(
            "lastUpdateRecovery".into(),
            serde_json::json!({
                "reason": "config-incompatible",
                "backupId": pending.backup_id,
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
}
