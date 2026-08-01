use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    identity::{AppIdentity, CliAliasStatus},
    state::AppState,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAliasRequest {
    pub enabled: bool,
}

#[tauri::command]
pub fn identity_get(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<AppIdentity, AppError> {
    let _ = request;
    Ok(state.paths().identity())
}

#[tauri::command]
pub fn identity_alias_status(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<CliAliasStatus, AppError> {
    let _ = request;
    Ok(state.paths().alias_status())
}

#[tauri::command]
pub fn identity_set_alias(
    request: SetAliasRequest,
    state: State<'_, AppState>,
) -> Result<CliAliasStatus, AppError> {
    state.paths().set_alias_enabled(request.enabled)
}
