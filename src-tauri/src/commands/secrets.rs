use std::sync::Arc;

use tauri::State;

use crate::{
    error::AppError,
    security::{
        execute_secret_import, plan_secret_import, CredentialState, SecretImportPlan,
        SecretImportReport, SecretImportSelection, SecretImportSource, SecretState,
    },
    state::AppState,
    storage::paths::StoragePaths,
};

#[tauri::command]
pub async fn secret_import_plan(
    request: SecretImportSource,
    app_state: State<'_, AppState>,
) -> Result<SecretImportPlan, AppError> {
    let paths = StoragePaths::from_app_paths(app_state.paths());
    tauri::async_runtime::spawn_blocking(move || plan_secret_import(&paths, &request))
        .await
        .map_err(|_| AppError::Io("secret import planning task failed".into()))?
}

#[tauri::command]
pub async fn secret_import_execute(
    request: SecretImportSelection,
    app_state: State<'_, AppState>,
    secret_state: State<'_, Arc<SecretState>>,
    credential_state: State<'_, CredentialState>,
) -> Result<SecretImportReport, AppError> {
    let paths = StoragePaths::from_app_paths(app_state.paths());
    let secrets = Arc::clone(secret_state.inner());
    let credentials = credential_state.store();
    tauri::async_runtime::spawn_blocking(move || {
        execute_secret_import(&paths, request, &secrets, credentials)
    })
    .await
    .map_err(|_| AppError::Io("secret import task failed".into()))?
}
