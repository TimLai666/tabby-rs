use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::{
    error::AppError,
    pty::{
        ChildProcess, PtyAckRequest, PtyIdRequest, PtyKillRequest, PtyManager, PtyResizeRequest,
        PtySpawnRequest, PtySpawnResponse, PtyWriteRequest,
    },
};

#[tauri::command]
pub async fn pty_spawn(
    app: AppHandle,
    manager: State<'_, Arc<PtyManager>>,
    request: PtySpawnRequest,
) -> Result<PtySpawnResponse, AppError> {
    let manager = Arc::clone(manager.inner());
    tauri::async_runtime::spawn_blocking(move || manager.spawn(app, request))
        .await
        .map_err(|_| AppError::Io("PTY spawn task failed".into()))?
}

#[tauri::command]
pub fn pty_exists(manager: State<'_, Arc<PtyManager>>, request: PtyIdRequest) -> bool {
    manager.exists(&request.id)
}

#[tauri::command]
pub fn pty_attach(
    app: AppHandle,
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<(), AppError> {
    manager.attach(&app, &request.id)
}

#[tauri::command]
pub fn pty_detach(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<(), AppError> {
    manager.detach(&request.id)
}

#[tauri::command]
pub fn pty_write(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyWriteRequest,
) -> Result<(), AppError> {
    manager.write(&request.id, &request.data)
}

#[tauri::command]
pub fn pty_resize(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyResizeRequest,
) -> Result<(), AppError> {
    manager.resize(&request.id, request.columns, request.rows)
}

#[tauri::command]
pub fn pty_kill(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyKillRequest,
) -> Result<(), AppError> {
    manager.kill(&request.id, request.signal.as_deref())
}

#[tauri::command]
pub fn pty_ack(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyAckRequest,
) -> Result<(), AppError> {
    manager.ack(&request.id, request.bytes)
}

#[tauri::command]
pub fn pty_get_pid(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<u32, AppError> {
    manager.pid(&request.id)
}

#[tauri::command]
pub async fn pty_get_true_pid(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<u32, AppError> {
    let manager = Arc::clone(manager.inner());
    tauri::async_runtime::spawn_blocking(move || manager.true_pid(&request.id))
        .await
        .map_err(|_| AppError::Io("process inspection task failed".into()))?
}

#[tauri::command]
pub async fn pty_get_children(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<Vec<ChildProcess>, AppError> {
    let manager = Arc::clone(manager.inner());
    tauri::async_runtime::spawn_blocking(move || manager.children(&request.id))
        .await
        .map_err(|_| AppError::Io("process inspection task failed".into()))?
}

#[tauri::command]
pub async fn pty_get_cwd(
    manager: State<'_, Arc<PtyManager>>,
    request: PtyIdRequest,
) -> Result<Option<String>, AppError> {
    let manager = Arc::clone(manager.inner());
    tauri::async_runtime::spawn_blocking(move || manager.cwd(&request.id))
        .await
        .map_err(|_| AppError::Io("process inspection task failed".into()))?
}
