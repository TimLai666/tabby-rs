use std::sync::Arc;

use tauri::State;

use crate::telnet::{
    TelnetConnectRequest, TelnetManager, TelnetResizeRequest, TelnetSessionIdRequest,
    TelnetSessionInfo, TelnetWriteRequest,
};

#[tauri::command]
pub async fn telnet_connect(
    app: tauri::AppHandle,
    request: TelnetConnectRequest,
    manager: State<'_, Arc<TelnetManager>>,
) -> Result<TelnetSessionInfo, crate::error::AppError> {
    manager.connect(app, request).await
}

#[tauri::command]
pub async fn telnet_write(
    request: TelnetWriteRequest,
    manager: State<'_, Arc<TelnetManager>>,
) -> Result<(), crate::error::AppError> {
    manager.write(request).await
}

#[tauri::command]
pub async fn telnet_resize(
    request: TelnetResizeRequest,
    manager: State<'_, Arc<TelnetManager>>,
) -> Result<(), crate::error::AppError> {
    manager.resize(request).await
}

#[tauri::command]
pub async fn telnet_close(
    request: TelnetSessionIdRequest,
    manager: State<'_, Arc<TelnetManager>>,
) -> Result<(), crate::error::AppError> {
    manager.close(request).await
}
