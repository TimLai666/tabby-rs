use tauri::State;

use crate::{commands::app::EmptyRequest, error::AppError, launch::LaunchContext, state::AppState};

#[tauri::command]
pub fn app_initial_launch(
    window: tauri::WebviewWindow,
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<Option<LaunchContext>, AppError> {
    let _ = request;
    Ok(state.launches().take(window.label()))
}
