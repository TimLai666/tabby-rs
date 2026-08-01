use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    identity::AppIdentity,
    state::AppState,
};

#[tauri::command]
pub fn identity_get(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<AppIdentity, AppError> {
    let _ = request;
    Ok(state.paths().identity())
}
