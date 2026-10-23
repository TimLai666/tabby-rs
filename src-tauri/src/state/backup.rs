use super::AppState;
use crate::{
    error::AppError,
    storage::{
        backup::{
            create_backup, list_backups, prepare_restore, BackupRequest, PreparedRestore,
            RestoreReport,
        },
        paths::StoragePaths,
        state_file::load_state,
    },
};

impl AppState {
    pub(crate) fn restore_backup(&self, backup_id: &str) -> Result<RestoreReport, AppError> {
        let _guard = self.lock_storage();
        let paths = StoragePaths::from_app_paths(self.paths());
        let exists = list_backups(&paths)?
            .iter()
            .any(|manifest| manifest.backup_id == backup_id);
        if !exists {
            return Err(AppError::NotFound("backup".into()));
        }

        let prepared = prepare_restore(&paths, backup_id)
            .map_err(|error| AppError::Io(format!("restore failed before any changes: {error}")))?;

        let safety = create_backup(
            &paths,
            &BackupRequest {
                reason: "before-restore".into(),
                source_version: Some(env!("CARGO_PKG_VERSION").into()),
                channel: None,
            },
            env!("CARGO_PKG_VERSION"),
        )?;

        match self.restore_snapshot(&paths, prepared) {
            Ok(report) => Ok(report),
            Err(restore_error) => {
                let rollback = prepare_restore(&paths, &safety.backup_id)
                    .and_then(|prepared| self.restore_snapshot(&paths, prepared));
                match rollback {
                    Ok(_) => Err(AppError::Io(format!(
                        "restore failed and the safety snapshot was reapplied: {restore_error}"
                    ))),
                    Err(rollback_error) => Err(AppError::Io(format!(
                        "restore failed and the safety rollback also failed: {restore_error}; rollback: {rollback_error}"
                    ))),
                }
            }
        }
    }

    fn restore_snapshot(
        &self,
        paths: &StoragePaths,
        prepared: PreparedRestore,
    ) -> Result<RestoreReport, AppError> {
        let report = prepared.apply()?;
        let restored = load_state(paths.state_file())?;
        *self
            .persisted_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = restored;
        Ok(report)
    }
}
