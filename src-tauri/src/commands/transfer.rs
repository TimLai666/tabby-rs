use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{Emitter, State};

use crate::{
    error::AppError,
    transfer::{
        manager::{TransferDescriptor, TransferManager},
        safe_path::resolve_inside,
    },
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenUploadRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDownloadRequest {
    pub name: String,
    pub mode: u32,
    pub size: Option<u64>,
    pub destination: String,
    pub base_directory: Option<String>,
    pub relative_path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExportRequest {
    pub destination: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct TransferIdRequest {
    pub id: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadRequest {
    pub id: String,
    pub max_bytes: usize,
}

#[derive(Debug, serde::Deserialize)]
pub struct WriteRequest {
    pub id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryRequest {
    pub path: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDirectoryRequest {
    pub base_directory: String,
    pub relative_path: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub size: u64,
    pub mode: u32,
    pub children: Vec<DirectoryEntry>,
}

fn emit_progress(app: &tauri::AppHandle, descriptor: &TransferDescriptor) -> Result<(), AppError> {
    app.emit("transfer.progress", descriptor)
        .map_err(|error| AppError::Io(error.to_string()))
}

#[tauri::command]
pub fn transfer_open_upload(
    request: OpenUploadRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<Vec<TransferDescriptor>, AppError> {
    state.open_upload(&request.paths)
}

#[tauri::command]
pub fn transfer_open_download(
    request: OpenDownloadRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<TransferDescriptor, AppError> {
    state.open_download(
        &request.name,
        request.mode,
        request.size,
        &request.destination,
        request.base_directory.as_deref(),
        request.relative_path.as_deref(),
    )
}

#[tauri::command]
pub fn terminal_export(
    request: TerminalExportRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<TransferDescriptor, AppError> {
    state.open_download(
        "terminal.txt",
        0o644,
        None,
        &request.destination,
        None,
        None,
    )
}

#[tauri::command]
pub fn transfer_read(
    app: tauri::AppHandle,
    request: ReadRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<Vec<u8>, AppError> {
    let (bytes, descriptor) = state.read(&request.id, request.max_bytes)?;
    emit_progress(&app, &descriptor)?;
    Ok(bytes)
}

#[tauri::command]
pub fn transfer_write(
    app: tauri::AppHandle,
    request: WriteRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<(), AppError> {
    let descriptor = state.write(&request.id, &request.data)?;
    emit_progress(&app, &descriptor)
}

#[tauri::command]
pub fn transfer_close(
    app: tauri::AppHandle,
    request: TransferIdRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<(), AppError> {
    let descriptor = state.close(&request.id)?;
    emit_progress(&app, &descriptor)
}

#[tauri::command]
pub fn transfer_cancel(
    app: tauri::AppHandle,
    request: TransferIdRequest,
    state: State<'_, std::sync::Arc<TransferManager>>,
) -> Result<(), AppError> {
    let descriptor = state.cancel(&request.id)?;
    emit_progress(&app, &descriptor)
}

#[tauri::command]
pub fn transfer_create_directory(request: CreateDirectoryRequest) -> Result<(), AppError> {
    let path = resolve_inside(Path::new(&request.base_directory), &request.relative_path)?;
    fs::create_dir_all(path)?;
    Ok(())
}

#[tauri::command]
pub fn transfer_list_directory(request: ListDirectoryRequest) -> Result<DirectoryEntry, AppError> {
    fn walk(path: &Path, count: &mut usize) -> Result<DirectoryEntry, AppError> {
        *count += 1;
        if *count > 100_000 {
            return Err(AppError::InvalidArgument(
                "directory contains too many files".into(),
            ));
        }
        let metadata = fs::symlink_metadata(path)?;
        let directory = metadata.is_dir();
        let mut children = Vec::new();
        if directory {
            for item in fs::read_dir(path)? {
                let item = item?;
                if item.file_type()?.is_symlink() {
                    continue;
                }
                children.push(walk(&item.path(), count)?);
            }
            children.sort_by(|left, right| left.name.cmp(&right.name));
        }
        Ok(DirectoryEntry {
            name: path
                .file_name()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            directory,
            size: if directory { 0 } else { metadata.len() },
            mode: file_mode(&metadata),
            children,
        })
    }

    walk(&PathBuf::from(request.path), &mut 0)
}

fn file_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode();
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o644
    }
}
