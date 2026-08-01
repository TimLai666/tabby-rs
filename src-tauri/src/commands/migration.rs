use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    state::AppState,
    storage::{
        migration::{
            detect_import_plans, execute_import, ImportPlan, ImportReport, ImportSelection,
        },
        paths::StoragePaths,
    },
};

#[tauri::command]
pub fn migration_detect(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<Vec<ImportPlan>, AppError> {
    let _ = request;
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    detect_import_plans(&paths)
}

#[tauri::command]
pub fn migration_execute(
    request: ImportSelection,
    state: State<'_, AppState>,
) -> Result<ImportReport, AppError> {
    let _guard = state.lock_storage();
    let paths = StoragePaths::from_app_paths(state.paths());
    execute_import(&paths, &request, env!("CARGO_PKG_VERSION"))
}
