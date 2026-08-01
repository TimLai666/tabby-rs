use secrecy::ExposeSecret;
use tauri::State;

use crate::{
    error::AppError,
    security::{
        CredentialAddress, CredentialNamespace, CredentialState, CredentialStore,
    },
};

#[derive(serde::Deserialize)]
pub struct PutCredentialRequest {
    pub service: String,
    pub account: String,
    pub value: String,
}

#[tauri::command]
pub async fn keychain_get(
    request: CredentialAddress,
    state: State<'_, CredentialState>,
) -> Result<Option<String>, AppError> {
    let store = state.store();
    run_blocking(move || {
        Ok(store
            .get(CredentialNamespace::TabbyRs, &request)?
            .map(|value| value.expose_secret().to_owned()))
    })
    .await
}

#[tauri::command]
pub async fn keychain_put(
    request: PutCredentialRequest,
    state: State<'_, CredentialState>,
) -> Result<(), AppError> {
    let store = state.store();
    run_blocking(move || {
        store.put(
            CredentialNamespace::TabbyRs,
            &CredentialAddress {
                service: request.service,
                account: request.account,
            },
            &request.value,
        )?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn keychain_delete(
    request: CredentialAddress,
    state: State<'_, CredentialState>,
) -> Result<bool, AppError> {
    let store = state.store();
    run_blocking(move || Ok(store.delete(CredentialNamespace::TabbyRs, &request)?)).await
}

async fn run_blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| AppError::Io("credential task failed".into()))?
}
