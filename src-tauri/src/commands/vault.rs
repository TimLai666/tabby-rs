use std::{sync::Arc, time::Duration};

use secrecy::SecretString;
use serde_json::Value;
use tauri::State;

use crate::{
    commands::app::EmptyRequest,
    error::AppError,
    security::{
        SecretState, StoredVault, VaultMutationResult, VaultSecretInput, VaultSecretSelector,
        VaultSnapshot, VaultStatus, VaultSummary,
    },
};

const MAX_REMEMBER_SECONDS: u64 = 24 * 60 * 60;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockVaultRequest {
    pub stored: StoredVault,
    pub passphrase: String,
    pub remember_for_seconds: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceVaultRequest {
    pub vault: VaultSnapshot,
    pub passphrase: String,
    pub remember_for_seconds: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetVaultEnabledRequest {
    pub enabled: bool,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub remember_for_seconds: Option<u64>,
}

#[derive(serde::Deserialize)]
pub struct PutSecretRequest {
    pub secret: VaultSecretInput,
}

#[derive(serde::Deserialize)]
pub struct UpdateSecretRequest {
    pub selector: VaultSecretSelector,
    pub secret: VaultSecretInput,
}

#[derive(serde::Deserialize)]
pub struct SecretSelectorRequest {
    pub selector: VaultSecretSelector,
}

#[derive(serde::Deserialize)]
pub struct SetVaultConfigRequest {
    pub config: Value,
}

#[derive(serde::Deserialize)]
pub struct PutVaultFileRequest {
    pub description: String,
    pub bytes: Vec<u8>,
}

#[derive(serde::Deserialize)]
pub struct GetVaultFileRequest {
    pub id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PutVaultFileResult {
    pub uri: String,
    pub mutation: VaultMutationResult,
}

#[tauri::command]
pub fn vault_status(
    request: EmptyRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultStatus, AppError> {
    let _ = request;
    Ok(state.status())
}

#[tauri::command]
pub fn vault_unlock(
    request: UnlockVaultRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultSummary, AppError> {
    let remember_for = remember_duration(request.remember_for_seconds)?;
    Ok(state.unlock(
        request.stored,
        SecretString::new(request.passphrase),
        remember_for,
    )?)
}

#[tauri::command]
pub fn vault_replace(
    request: ReplaceVaultRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultMutationResult, AppError> {
    let remember_for = remember_duration(request.remember_for_seconds)?;
    Ok(state.replace(
        request.vault,
        SecretString::new(request.passphrase),
        remember_for,
    )?)
}

#[tauri::command]
pub fn vault_lock(
    request: EmptyRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<(), AppError> {
    let _ = request;
    state.lock_now();
    Ok(())
}

#[tauri::command]
pub fn vault_set_enabled(
    request: SetVaultEnabledRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<Option<VaultMutationResult>, AppError> {
    if !request.enabled {
        state.lock_now();
        return Ok(None);
    }
    let passphrase = request
        .passphrase
        .ok_or_else(|| AppError::InvalidArgument("vault passphrase is required".into()))?;
    let remember_for = remember_duration(request.remember_for_seconds.unwrap_or(300))?;
    Ok(Some(state.create(
        SecretString::new(passphrase),
        remember_for,
    )?))
}

#[tauri::command]
pub fn vault_summary(
    request: EmptyRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultSummary, AppError> {
    let _ = request;
    Ok(state.summary()?)
}

#[tauri::command]
pub fn vault_snapshot(
    request: EmptyRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultSnapshot, AppError> {
    let _ = request;
    Ok(state.snapshot()?)
}

#[tauri::command]
pub fn vault_get_secret(
    request: SecretSelectorRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<Option<String>, AppError> {
    Ok(state.get_secret(&request.selector)?)
}

#[tauri::command]
pub fn vault_put_secret(
    request: PutSecretRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultMutationResult, AppError> {
    Ok(state.put_secret(request.secret)?)
}

#[tauri::command]
pub fn vault_update_secret(
    request: UpdateSecretRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultMutationResult, AppError> {
    Ok(state.update_secret(&request.selector, request.secret)?)
}

#[tauri::command]
pub fn vault_remove_secret(
    request: SecretSelectorRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultMutationResult, AppError> {
    Ok(state.remove_secret(&request.selector)?)
}

#[tauri::command]
pub fn vault_set_config(
    request: SetVaultConfigRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<VaultMutationResult, AppError> {
    Ok(state.set_config(request.config)?)
}

#[tauri::command]
pub fn vault_put_file(
    request: PutVaultFileRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<PutVaultFileResult, AppError> {
    let (uri, mutation) = state.put_file(request.description, request.bytes)?;
    Ok(PutVaultFileResult { uri, mutation })
}

#[tauri::command]
pub fn vault_get_file(
    request: GetVaultFileRequest,
    state: State<'_, Arc<SecretState>>,
) -> Result<Vec<u8>, AppError> {
    Ok(state.get_file(&request.id)?)
}

fn remember_duration(seconds: u64) -> Result<Duration, AppError> {
    if seconds > MAX_REMEMBER_SECONDS {
        return Err(AppError::InvalidArgument(
            "vault remember timeout is too large".into(),
        ));
    }
    Ok(Duration::from_secs(seconds.max(1)))
}
