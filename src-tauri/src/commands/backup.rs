use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    state::AppState,
    storage::{
        backup::{create_backup, list_backups, BackupManifest, BackupRequest, RestoreReport},
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
    state.restore_backup(&request.backup_id)
}
