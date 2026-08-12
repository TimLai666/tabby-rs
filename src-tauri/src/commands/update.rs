use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    state::AppState,
    storage::{
        backup::{create_backup, BackupRequest},
        paths::StoragePaths,
        state_file::{PendingUpdateState, UpdateChannel},
    },
    update::service::{
        build_updater, configured_endpoint, configured_public_key, is_cancelled,
        update_info_from_remote, DownloadHandle, UpdateInfo, UpdateStage,
    },
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVersionRequest {
    pub version: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelRequest {
    pub channel: UpdateChannel,
}

fn emit_state(app: &AppHandle, state: &AppState) {
    let _ = app.emit("update.state", state.update_manager().state());
}

fn public_update_error(stage: UpdateStage) -> AppError {
    AppError::Io(format!("update {stage:?} failed"))
}

async fn check_update(app: &AppHandle, state: &AppState) -> Result<Option<UpdateInfo>, AppError> {
    let generation = state.update_manager().begin_check()?;
    emit_state(app, state);
    let channel = state.persisted_state().update_channel;
    let current_version = app.package_info().version.to_string();
    let updater = match build_updater(app, &channel) {
        Ok(updater) => updater,
        Err(error) => {
            state.update_manager().fail_check(
                generation,
                UpdateStage::Configuration,
                "updater is not configured",
            );
            emit_state(app, state);
            return Err(error);
        }
    };
    let remote = match updater.check().await {
        Ok(remote) => remote,
        Err(_) => {
            state.update_manager().fail_check(
                generation,
                UpdateStage::Checking,
                "update check failed",
            );
            emit_state(app, state);
            return Err(public_update_error(UpdateStage::Checking));
        }
    };
    let Some(remote) = remote else {
        let result = state.update_manager().finish_check(generation, None)?;
        emit_state(app, state);
        return Ok(result);
    };
    let (manifest, info) = match update_info_from_remote(&remote, &channel, &current_version) {
        Ok(value) => value,
        Err(error) => {
            state.update_manager().fail_check(
                generation,
                UpdateStage::Checking,
                "update metadata was rejected",
            );
            emit_state(app, state);
            return Err(error);
        }
    };
    let result = state
        .update_manager()
        .finish_check(generation, Some((remote, manifest, info)))?;
    emit_state(app, state);
    Ok(result)
}

#[tauri::command]
pub async fn update_check(
    request: EmptyRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<UpdateInfo>, AppError> {
    let _ = request;
    check_update(&app, &state).await
}

#[tauri::command]
pub async fn update_download(
    request: UpdateVersionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let handle: DownloadHandle = state.update_manager().begin_download(&request.version)?;
    emit_state(&app, &state);
    let manager = Arc::clone(state.update_manager());
    let progress_app = app.clone();
    let progress_version = handle.info.version.clone();
    let progress_total = handle.manifest.size;
    let update = handle.update;
    let mut cancellation = handle.cancellation;
    let mut downloaded = 0_u64;
    let download = update.download(
        |chunk, content_length| {
            downloaded = downloaded.saturating_add(chunk as u64);
            manager.set_download_progress(
                &progress_version,
                downloaded,
                progress_total.or(content_length),
            );
            let _ = progress_app.emit("update.state", manager.state());
        },
        || {},
    );
    tokio::pin!(download);
    let bytes = match tokio::select! {
        result = &mut download => result.map_err(|_| public_update_error(UpdateStage::Downloading)),
        changed = cancellation.changed() => {
            let _ = changed;
            Err(AppError::Conflict("update download cancelled".into()))
        }
    } {
        Ok(bytes) => bytes,
        Err(error) => {
            if is_cancelled(&cancellation) {
                state.update_manager().cancel();
            } else {
                state
                    .update_manager()
                    .fail(UpdateStage::Downloading, "update download failed");
            }
            emit_state(&app, &state);
            return Err(error);
        }
    };
    if is_cancelled(&cancellation) {
        state.update_manager().cancel();
        emit_state(&app, &state);
        return Err(AppError::Conflict("update download cancelled".into()));
    }
    if let Err(error) = state.update_manager().finish_download(bytes) {
        state
            .update_manager()
            .fail(UpdateStage::Downloading, "download verification failed");
        emit_state(&app, &state);
        return Err(error);
    }
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn update_cancel(
    request: EmptyRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let _ = request;
    state.update_manager().cancel();
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn update_install(
    request: UpdateVersionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let ready = state.update_manager().take_ready(&request.version)?;
    emit_state(&app, &state);
    let current_state = state.persisted_state();
    let paths = StoragePaths::from_app_paths(state.paths());
    let backup = {
        let _guard = state.lock_storage();
        create_backup(
            &paths,
            &BackupRequest {
                reason: "before-update".into(),
                source_version: Some(app.package_info().version.to_string()),
                channel: Some(current_state.update_channel.clone()),
            },
            &app.package_info().version.to_string(),
        )
    };
    let backup = match backup {
        Ok(backup) => backup,
        Err(_) => {
            state.update_manager().restore_ready(ready);
            emit_state(&app, &state);
            return Err(AppError::Io("update backup could not be created".into()));
        }
    };
    let target_version = ready.info.version.clone();
    if state
        .update_persisted_state(|persisted| {
            if current_state.update_channel == UpdateChannel::Stable {
                persisted.last_stable_backup = Some(backup.backup_id.clone());
            }
            persisted.pending_update = Some(PendingUpdateState {
                target_version: target_version.clone(),
                backup_id: backup.backup_id.clone(),
                channel: current_state.update_channel.clone(),
            });
        })
        .is_err()
    {
        state.update_manager().restore_ready(ready);
        emit_state(&app, &state);
        return Err(AppError::Io("update state could not be persisted".into()));
    }
    if ready.update.install(&ready.bytes).is_err() {
        let _ = state.update_persisted_state(|persisted| persisted.pending_update = None);
        state.update_manager().restore_ready(ready);
        emit_state(&app, &state);
        return Err(public_update_error(UpdateStage::Installing));
    }
    state.update_manager().finish_install();
    emit_state(&app, &state);
    app.request_restart();
    Ok(())
}

#[tauri::command]
pub fn update_get_channel(
    request: EmptyRequest,
    state: State<'_, AppState>,
) -> Result<UpdateChannel, AppError> {
    let _ = request;
    Ok(state.persisted_state().update_channel)
}

#[tauri::command]
pub async fn update_set_channel(
    request: UpdateChannelRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let current_state = state.persisted_state();
    if current_state.update_channel == request.channel {
        return Ok(());
    }
    let paths = StoragePaths::from_app_paths(state.paths());
    let backup = {
        let _guard = state.lock_storage();
        create_backup(
            &paths,
            &BackupRequest {
                reason: "before-channel-switch".into(),
                source_version: Some(app.package_info().version.to_string()),
                channel: Some(current_state.update_channel.clone()),
            },
            &app.package_info().version.to_string(),
        )
    }
    .map_err(|_| AppError::Io("channel switch backup could not be created".into()))?;
    state.update_manager().cancel();
    state.update_persisted_state(|persisted| {
        if current_state.update_channel == UpdateChannel::Stable {
            persisted.last_stable_backup = Some(backup.backup_id.clone());
        }
        persisted.update_channel = request.channel.clone();
    })?;
    emit_state(&app, &state);
    if configured_endpoint(&request.channel).is_some() && configured_public_key().is_some() {
        check_update(&app, &state).await?;
    }
    Ok(())
}
