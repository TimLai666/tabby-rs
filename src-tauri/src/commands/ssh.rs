use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::{
    error::AppError,
    security::{CredentialState, SecretState},
    ssh::{
        self,
        model::{
            HostKeyDecisionRequest, SshAuthResponseRequest, SshConnectRequest, SshResizeRequest,
            SshSessionIdRequest, SshSessionInfo, SshWriteRequest,
        },
        SshImportPreview, SshImportReport, SshImportSelection, SshImportSource, SshManager,
    },
    state::AppState,
    storage::{
        config_file::{read_config, write_config, ConfigWriteRequest},
        paths::StoragePaths,
    },
};

#[tauri::command]
pub fn ssh_list_private_keys() -> Result<Vec<String>, AppError> {
    Ok(ssh::private_key_candidates())
}

#[tauri::command]
pub fn ssh_import_preview(
    mut request: SshImportSource,
    app_state: State<'_, AppState>,
) -> Result<SshImportPreview, AppError> {
    let paths = StoragePaths::from_app_paths(app_state.paths());
    let config = read_config(paths.config_file())?;
    request.existing_profile_ids = profile_ids(&config.yaml);
    let mut preview = ssh::preview(&request)?;
    preview.revision = config.revision;
    Ok(preview)
}

#[tauri::command]
pub fn ssh_import_apply(
    request: SshImportSelection,
    app_state: State<'_, AppState>,
) -> Result<SshImportReport, AppError> {
    let _guard = app_state.lock_storage();
    let paths = StoragePaths::from_app_paths(app_state.paths());
    paths.ensure_layout()?;
    let config_path = paths.config_file().to_path_buf();
    let current = read_config(&config_path)?;
    let (yaml, imported, skipped, failed) = ssh::apply(
        &config_path,
        request,
        &current.yaml,
        current.revision.as_deref(),
    )?;
    let result = write_config(
        &config_path,
        &ConfigWriteRequest {
            yaml,
            expected_revision: current.revision,
            require_missing: false,
        },
    )?;
    Ok(SshImportReport {
        imported,
        skipped,
        failed,
        revision: result.revision,
        path: result.path,
    })
}

fn profile_ids(yaml: &str) -> Vec<String> {
    serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|value| value.get("profiles").cloned())
        .and_then(|value| value.as_sequence().cloned())
        .into_iter()
        .flatten()
        .filter_map(|profile| {
            profile
                .get("id")
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    request: SshConnectRequest,
    manager: State<'_, Arc<SshManager>>,
    secrets: State<'_, Arc<SecretState>>,
    credentials: State<'_, CredentialState>,
) -> Result<SshSessionInfo, AppError> {
    manager
        .connect(
            app,
            request,
            Arc::clone(secrets.inner()),
            credentials.inner().clone(),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn ssh_host_key_decision(
    request: HostKeyDecisionRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager
        .host_key_decision(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn ssh_auth_response(
    request: SshAuthResponseRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.auth_response(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn ssh_write(
    request: SshWriteRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.write(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn ssh_resize(
    request: SshResizeRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.resize(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn ssh_close(
    request: SshSessionIdRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.close(request).await.map_err(AppError::from)
}
