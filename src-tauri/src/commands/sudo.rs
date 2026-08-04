use std::sync::Arc;

use secrecy::SecretString;
use serde_json::{Map, Value};
use tauri::State;

use crate::{
    error::AppError,
    pty::PtyManager,
    security::{SecretState, VaultSecretSelector},
    sudo::profile_id_from_secret_ref,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SudoRespondRequest {
    pub prompt_id: String,
    pub secret_ref: String,
}

#[tauri::command]
pub fn sudo_respond(
    request: SudoRespondRequest,
    manager: State<'_, Arc<PtyManager>>,
    secrets: State<'_, Arc<SecretState>>,
) -> Result<(), AppError> {
    let profile_id = profile_id_from_secret_ref(&request.secret_ref)
        .map_err(|error| AppError::InvalidArgument(error.to_string()))?;
    let mut key = Map::new();
    key.insert("profileId".into(), Value::String(profile_id.to_owned()));
    let secret = secrets
        .get_secret(&VaultSecretSelector {
            r#type: "sudo:password".into(),
            key,
        })?
        .ok_or_else(|| AppError::NotFound("saved sudo password is unavailable".into()))?;
    let secret = SecretString::new(secret);
    manager.respond_sudo(&request.prompt_id, &request.secret_ref, &secret)
}
