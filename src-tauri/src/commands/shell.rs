use crate::{
    error::AppError,
    shell::{
        detect_shells, prepare_spawn, DetectShellsRequest, PrepareSpawnRequest,
        PreparedSpawnRequest, ShellDetectionResult,
    },
};

#[tauri::command]
pub async fn shell_detect(
    request: DetectShellsRequest,
) -> Result<ShellDetectionResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || Ok(detect_shells(request)))
        .await
        .map_err(|_| AppError::Io("shell detection task failed".into()))?
}

#[tauri::command]
pub async fn shell_prepare_spawn(
    request: PrepareSpawnRequest,
) -> Result<PreparedSpawnRequest, AppError> {
    tauri::async_runtime::spawn_blocking(move || prepare_spawn(request))
        .await
        .map_err(|_| AppError::Io("shell spawn preparation task failed".into()))?
}
