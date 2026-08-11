use std::sync::Arc;

use tauri::State;

use crate::{
    error::AppError,
    ssh::{
        model::SshSessionIdRequest,
        sftp::{
            RemoteFileEntry, SftpDownloadOpenRequest, SftpPathRequest, SftpReadRequest,
            SftpRemoveRequest, SftpRenameRequest, SftpSessionInfo, SftpStatRequest,
            SftpTransferDescriptor, SftpTransferIdRequest, SftpUploadOpenRequest, SftpWriteRequest,
        },
        SshManager,
    },
};

#[tauri::command]
pub async fn sftp_open(
    request: SshSessionIdRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpSessionInfo, AppError> {
    manager.sftp_open(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_list(
    request: SftpPathRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<Vec<RemoteFileEntry>, AppError> {
    manager.sftp_list(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_stat(
    request: SftpStatRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<RemoteFileEntry, AppError> {
    manager.sftp_stat(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_mkdir(
    request: SftpPathRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.sftp_mkdir(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_rename(
    request: SftpRenameRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.sftp_rename(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_remove(
    request: SftpRemoveRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.sftp_remove(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_upload_open(
    request: SftpUploadOpenRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_open_upload(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_upload(
    request: SftpUploadOpenRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_open_upload(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_download_open(
    request: SftpDownloadOpenRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_open_download(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_download(
    request: SftpDownloadOpenRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_open_download(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_read(
    request: SftpReadRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<Vec<u8>, AppError> {
    manager
        .sftp_read(request)
        .await
        .map(|(bytes, _)| bytes)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_write(
    request: SftpWriteRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager.sftp_write(request).await.map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_close_transfer(
    request: SftpTransferIdRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_close_transfer(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_cancel_transfer(
    request: SftpTransferIdRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<SftpTransferDescriptor, AppError> {
    manager
        .sftp_cancel_transfer(request)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn sftp_close(
    request: SshSessionIdRequest,
    manager: State<'_, Arc<SshManager>>,
) -> Result<(), AppError> {
    manager.sftp_close(request).await.map_err(AppError::from)
}
