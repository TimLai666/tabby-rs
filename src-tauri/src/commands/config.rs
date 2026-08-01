use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    state::AppState,
    storage::{
        config_file::{
            read_config, write_config, ConfigReadResult, ConfigWriteRequest, ConfigWriteResult,
        },
        paths::StoragePaths,
    },
};

#[tauri::command]
pub fn config_read(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<ConfigReadResult, AppError> {
    let _ = request;
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout()?;
    read_config(paths.config_file())
}

#[tauri::command]
pub fn config_write(
    request: ConfigWriteRequest,
    state: State<'_, AppState>,
) -> Result<ConfigWriteResult, AppError> {
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    paths.ensure_layout()?;
    write_config(paths.config_file(), &request)
}
