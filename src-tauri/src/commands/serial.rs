use std::sync::Arc;

use tauri::State;

use crate::serial::{
    list_serial_ports, SerialManager, SerialOpenRequest, SerialSessionIdRequest, SerialSessionInfo,
    SerialSignalRequest, SerialSignalState, SerialWriteRequest,
};

#[tauri::command]
pub async fn serial_list() -> Result<Vec<crate::serial::SerialPortInfo>, crate::error::AppError> {
    tokio::task::spawn_blocking(list_serial_ports)
        .await
        .map_err(|error| crate::error::AppError::Io(error.to_string()))?
}

#[tauri::command]
pub async fn serial_open(
    app: tauri::AppHandle,
    request: SerialOpenRequest,
    manager: State<'_, Arc<SerialManager>>,
) -> Result<SerialSessionInfo, crate::error::AppError> {
    let manager = Arc::clone(&manager);
    tokio::task::spawn_blocking(move || manager.open(app, request))
        .await
        .map_err(|error| crate::error::AppError::Io(error.to_string()))?
}

#[tauri::command]
pub async fn serial_write(
    request: SerialWriteRequest,
    manager: State<'_, Arc<SerialManager>>,
) -> Result<(), crate::error::AppError> {
    manager.write(request).await
}

#[tauri::command]
pub async fn serial_set_signals(
    request: SerialSignalRequest,
    manager: State<'_, Arc<SerialManager>>,
) -> Result<(), crate::error::AppError> {
    manager.set_signals(request).await
}

#[tauri::command]
pub async fn serial_get_signals(
    request: SerialSessionIdRequest,
    manager: State<'_, Arc<SerialManager>>,
) -> Result<SerialSignalState, crate::error::AppError> {
    manager.get_signals(request).await
}

#[tauri::command]
pub async fn serial_close(
    request: SerialSessionIdRequest,
    manager: State<'_, Arc<SerialManager>>,
) -> Result<(), crate::error::AppError> {
    manager.close(request).await
}
