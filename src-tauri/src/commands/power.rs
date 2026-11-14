use tauri::State;

use crate::{error::AppError, state::AppState};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerInhibitAcquired {
    pub id: u64,
}

// Both commands are async and run the native call on a blocking worker: the
// Linux backend talks to D-Bus with a 2 s timeout and macOS spawns a child,
// which must not block the main thread or the async runtime.

#[tauri::command]
pub async fn power_inhibit_acquire(
    state: State<'_, AppState>,
) -> Result<PowerInhibitAcquired, AppError> {
    let inhibitor = state.power_inhibitor();
    let id = tauri::async_runtime::spawn_blocking(move || inhibitor.acquire())
        .await
        .map_err(|error| AppError::Io(error.to_string()))?
        .map_err(AppError::Conflict)?;
    Ok(PowerInhibitAcquired { id })
}

#[tauri::command]
pub async fn power_inhibit_release(state: State<'_, AppState>, id: u64) -> Result<(), AppError> {
    let inhibitor = state.power_inhibitor();
    tauri::async_runtime::spawn_blocking(move || {
        inhibitor.release(id);
    })
    .await
    .map_err(|error| AppError::Io(error.to_string()))?;
    Ok(())
}
