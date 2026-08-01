use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    state::AppState,
    storage::{
        backup::{
            create_backup, list_backups, restore_backup, BackupManifest, BackupRequest,
            RestoreReport,
        },
        paths::StoragePaths,
    },
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRestoreRequest {
    pub backup_id: String,
}

#[tauri::command]
pub fn backup_create(
    request: BackupRequest,
    state: State<'_, AppState>,
) -> Result<BackupManifest, AppError> {
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    create_backup(&paths, &request, env!("CARGO_PKG_VERSION"))
}

#[tauri::command]
pub fn backup_list(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<Vec<BackupManifest>, AppError> {
    let _ = request;
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    list_backups(&paths)
}

#[tauri::command]
pub fn backup_restore(
    request: BackupRestoreRequest,
    state: State<'_, AppState>,
) -> Result<RestoreReport, AppError> {
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    let exists = list_backups(&paths)?
        .iter()
        .any(|manifest| manifest.backup_id == request.backup_id);
    if !exists {
        return Err(AppError::NotFound("backup".into()));
    }

    create_backup(
        &paths,
        &BackupRequest {
            reason: "before-restore".into(),
            source_version: Some(env!("CARGO_PKG_VERSION").into()),
            channel: None,
        },
        env!("CARGO_PKG_VERSION"),
    )?;
    restore_backup(&paths, &request.backup_id)
}
