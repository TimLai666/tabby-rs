use crate::{
    error::AppError,
    transfer::{
        file_edit::{EditableFile, FileEditManager},
        manager::TransferManager,
    },
};
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareRequest {
    name: String,
    mode: u32,
    size: u64,
}
#[derive(serde::Deserialize)]
pub struct IdRequest {
    id: String,
}

#[tauri::command]
pub fn file_edit_prepare(
    request: PrepareRequest,
    edits: State<'_, Arc<FileEditManager>>,
    transfers: State<'_, Arc<TransferManager>>,
) -> Result<EditableFile, AppError> {
    edits.prepare(&request.name, request.mode, request.size, &transfers)
}
#[tauri::command]
pub fn file_edit_ready(
    request: IdRequest,
    edits: State<'_, Arc<FileEditManager>>,
) -> Result<(), AppError> {
    edits.ready(&request.id)
}
#[tauri::command]
pub fn file_edit_watch(
    request: IdRequest,
    edits: State<'_, Arc<FileEditManager>>,
    app: tauri::AppHandle,
) -> Result<(), AppError> {
    edits.watch(&request.id, move |event| {
        let _ = app.emit("fileEdit:changed", event);
    })
}
#[tauri::command]
pub fn file_edit_stop(
    request: IdRequest,
    edits: State<'_, Arc<FileEditManager>>,
) -> Result<(), AppError> {
    edits.stop(&request.id)
}
