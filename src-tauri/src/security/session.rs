use std::{
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};

use super::vault_v1::{StoredVault, Vault, VaultCodecs, VaultError, VaultSecret};

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub unlocked: bool,
    pub expires_in_seconds: Option<u64>,
    pub secret_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSummary {
    pub config: Value,
    pub secrets: Vec<VaultSecretSummary>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VaultSecretSummary {
    pub r#type: String,
    pub key: Map<String, Value>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultSnapshot {
    pub config: Value,
    pub secrets: Vec<VaultSnapshotSecret>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct VaultSnapshotSecret {
    pub r#type: String,
    pub key: Map<String, Value>,
    pub value: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct VaultSecretSelector {
    pub r#type: String,
    pub key: Map<String, Value>,
}

#[derive(serde::Deserialize)]
pub struct VaultSecretInput {
    pub r#type: String,
    pub key: Map<String, Value>,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultMutationResult {
    pub stored: StoredVault,
    pub summary: VaultSummary,
}

pub struct SecretState {
    codecs: VaultCodecs,
    inner: Mutex<SecretSession>,
}

struct SecretSession {
    vault: Option<Vault>,
    passphrase: Option<SecretString>,
    stored: Option<StoredVault>,
    expires_at: Option<Instant>,
}

impl Default for SecretState {
    fn default() -> Self {
        Self {
            codecs: VaultCodecs::default(),
            inner: Mutex::new(SecretSession {
                vault: None,
                passphrase: None,
                stored: None,
                expires_at: None,
            }),
        }
    }
}

impl SecretState {
    pub fn status(&self) -> VaultStatus {
        let mut inner = self.lock();
        expire_if_needed(&mut inner);
        VaultStatus {
            unlocked: inner.vault.is_some(),
            expires_in_seconds: inner
                .expires_at
                .map(|deadline| deadline.saturating_duration_since(Instant::now()).as_secs()),
            secret_count: inner
                .vault
                .as_ref()
                .map(|vault| vault.secrets.len())
                .unwrap_or(0),
        }
    }

    pub fn unlock(
        &self,
        stored: StoredVault,
        passphrase: SecretString,
        remember_for: Duration,
    ) -> Result<VaultSummary, VaultError> {
        let vault = self.codecs.decrypt(&stored, &passphrase)?;
        let summary = summarize(&vault);
        let mut inner = self.lock();
        clear_session(&mut inner);
        inner.vault = Some(vault);
        inner.passphrase = Some(passphrase);
        inner.stored = Some(stored);
        inner.expires_at = Some(Instant::now() + remember_for);
        Ok(summary)
    }

    pub fn create(
        &self,
        passphrase: SecretString,
        remember_for: Duration,
    ) -> Result<VaultMutationResult, VaultError> {
        self.replace(
            VaultSnapshot {
                config: Value::Null,
                secrets: Vec::new(),
            },
            passphrase,
            remember_for,
        )
    }

    pub fn replace(
        &self,
        snapshot: VaultSnapshot,
        passphrase: SecretString,
        remember_for: Duration,
    ) -> Result<VaultMutationResult, VaultError> {
        let mut secrets = Vec::with_capacity(snapshot.secrets.len());
        for secret in snapshot.secrets {
            validate_secret(&secret.r#type, &secret.key)?;
            secrets.push(VaultSecret {
                r#type: secret.r#type,
                key: secret.key,
                value: SecretString::new(secret.value),
            });
        }
        let vault = Vault {
            config: snapshot.config,
            secrets,
        };
        let stored = self.codecs.encrypt(1, &vault, &passphrase)?;
        let summary = summarize(&vault);
        let mut inner = self.lock();
        clear_session(&mut inner);
        inner.vault = Some(vault);
        inner.passphrase = Some(passphrase);
        inner.stored = Some(stored.clone());
        inner.expires_at = Some(Instant::now() + remember_for);
        Ok(VaultMutationResult { stored, summary })
    }

    pub fn lock_now(&self) {
        clear_session(&mut self.lock());
    }

    pub fn summary(&self) -> Result<VaultSummary, VaultError> {
        let inner = self.unlocked()?;
        Ok(summarize(inner.vault.as_ref().expect("checked above")))
    }

    pub fn snapshot(&self) -> Result<VaultSnapshot, VaultError> {
        let inner = self.unlocked()?;
        let vault = inner.vault.as_ref().expect("checked above");
        Ok(VaultSnapshot {
            config: vault.config.clone(),
            secrets: vault
                .secrets
                .iter()
                .map(|secret| VaultSnapshotSecret {
                    r#type: secret.r#type.clone(),
                    key: secret.key.clone(),
                    value: secret.value.expose_secret().to_owned(),
                })
                .collect(),
        })
    }

    pub fn get_secret(&self, selector: &VaultSecretSelector) -> Result<Option<String>, VaultError> {
        validate_selector(selector)?;
        let inner = self.unlocked()?;
        let vault = inner.vault.as_ref().expect("checked above");
        let secret = find_secret(vault, selector).or_else(|| {
            if !selector.key.contains_key("host") {
                return None;
            }
            let mut fallback = selector.clone();
            fallback.key.insert("host".into(), Value::Null);
            find_secret(vault, &fallback)
        });
        Ok(secret.map(|secret| secret.value.expose_secret().to_owned()))
    }

    pub fn put_secret(&self, input: VaultSecretInput) -> Result<VaultMutationResult, VaultError> {
        validate_secret(&input.r#type, &input.key)?;
        self.mutate(|vault| {
            vault.secrets.retain(|secret| {
                secret.r#type != input.r#type || !key_matches(&input.key, &secret.key)
            });
            vault.secrets.push(VaultSecret {
                r#type: input.r#type,
                key: input.key,
                value: SecretString::new(input.value),
            });
            Ok(())
        })
    }

    pub fn update_secret(
        &self,
        selector: &VaultSecretSelector,
        input: VaultSecretInput,
    ) -> Result<VaultMutationResult, VaultError> {
        validate_selector(selector)?;
        validate_secret(&input.r#type, &input.key)?;
        self.mutate(|vault| {
            let Some(target) = vault.secrets.iter_mut().find(|secret| {
                secret.r#type == selector.r#type && key_matches(&selector.key, &secret.key)
            }) else {
                return Ok(());
            };
            target.r#type = input.r#type;
            target.key = input.key;
            target.value = SecretString::new(input.value);
            Ok(())
        })
    }

    pub fn remove_secret(
        &self,
        selector: &VaultSecretSelector,
    ) -> Result<VaultMutationResult, VaultError> {
        validate_selector(selector)?;
        self.mutate(|vault| {
            vault.secrets.retain(|secret| {
                secret.r#type != selector.r#type || !key_matches(&selector.key, &secret.key)
            });
            Ok(())
        })
    }

    pub fn set_config(&self, config: Value) -> Result<VaultMutationResult, VaultError> {
        self.mutate(|vault| {
            vault.config = config;
            Ok(())
        })
    }

    pub fn put_file(
        &self,
        description: String,
        bytes: Vec<u8>,
    ) -> Result<(String, VaultMutationResult), VaultError> {
        if description.trim().is_empty()
            || description.len() > 512
            || description.chars().any(char::is_control)
            || bytes.len() > MAX_FILE_BYTES
        {
            return Err(VaultError::InvalidSecret);
        }
        let mut id = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut id)
            .map_err(|_| VaultError::RandomFailed)?;
        let id = hex::encode(id);
        let mut key = Map::new();
        key.insert("description".into(), Value::String(description));
        key.insert("id".into(), Value::String(id.clone()));
        let mutation = self.put_secret(VaultSecretInput {
            r#type: "file".into(),
            key,
            value: BASE64_STANDARD.encode(bytes),
        })?;
        Ok((format!("vault://{id}"), mutation))
    }

    pub fn get_file(&self, id: &str) -> Result<Vec<u8>, VaultError> {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(VaultError::InvalidSecret);
        }
        let mut key = Map::new();
        key.insert("id".into(), Value::String(id.into()));
        let encoded = self
            .get_secret(&VaultSecretSelector {
                r#type: "file".into(),
                key,
            })?
            .ok_or(VaultError::InvalidSecret)?;
        let bytes = BASE64_STANDARD
            .decode(encoded)
            .map_err(|_| VaultError::InvalidSecret)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(VaultError::InvalidSecret);
        }
        Ok(bytes)
    }

    fn mutate(
        &self,
        operation: impl FnOnce(&mut Vault) -> Result<(), VaultError>,
    ) -> Result<VaultMutationResult, VaultError> {
        let mut inner = self.unlocked()?;
        operation(inner.vault.as_mut().expect("checked above"))?;
        let stored = self.codecs.encrypt(
            1,
            inner.vault.as_ref().expect("checked above"),
            inner.passphrase.as_ref().expect("checked above"),
        )?;
        inner.stored = Some(stored.clone());
        Ok(VaultMutationResult {
            summary: summarize(inner.vault.as_ref().expect("checked above")),
            stored,
        })
    }

    fn unlocked(&self) -> Result<MutexGuard<'_, SecretSession>, VaultError> {
        let mut inner = self.lock();
        expire_if_needed(&mut inner);
        if inner.vault.is_none() || inner.passphrase.is_none() {
            return Err(VaultError::Locked);
        }
        Ok(inner)
    }

    fn lock(&self) -> MutexGuard<'_, SecretSession> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn expire_if_needed(inner: &mut SecretSession) {
    let expired = inner
        .expires_at
        .map(|deadline| Instant::now() >= deadline)
        .unwrap_or(false);
    if expired {
        clear_session(inner);
    }
}

fn clear_session(inner: &mut SecretSession) {
    inner.vault = None;
    inner.passphrase = None;
    inner.stored = None;
    inner.expires_at = None;
}

fn summarize(vault: &Vault) -> VaultSummary {
    VaultSummary {
        config: vault.config.clone(),
        secrets: vault
            .secrets
            .iter()
            .map(|secret| VaultSecretSummary {
                r#type: secret.r#type.clone(),
                key: secret.key.clone(),
            })
            .collect(),
    }
}

fn validate_selector(selector: &VaultSecretSelector) -> Result<(), VaultError> {
    validate_secret(&selector.r#type, &selector.key)
}

fn validate_secret(r#type: &str, key: &Map<String, Value>) -> Result<(), VaultError> {
    if r#type.is_empty()
        || r#type.len() > 128
        || r#type.chars().any(char::is_control)
        || key.len() > 32
    {
        return Err(VaultError::InvalidSecret);
    }
    for (name, value) in key {
        if name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
            || matches!(value, Value::Array(_) | Value::Object(_))
        {
            return Err(VaultError::InvalidSecret);
        }
    }
    Ok(())
}

fn find_secret<'a>(vault: &'a Vault, selector: &VaultSecretSelector) -> Option<&'a VaultSecret> {
    vault
        .secrets
        .iter()
        .find(|secret| secret.r#type == selector.r#type && key_matches(&selector.key, &secret.key))
}

fn key_matches(selector: &Map<String, Value>, stored: &Map<String, Value>) -> bool {
    selector
        .iter()
        .all(|(name, value)| stored.get(name) == Some(value))
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use secrecy::SecretString;
    use serde_json::{json, Map, Value};

    use super::{
        SecretState, VaultSecretInput, VaultSecretSelector, VaultSnapshot, VaultSnapshotSecret,
    };
    use crate::security::vault_v1::{VaultCodec, VaultV1};

    fn password_key() -> Map<String, Value> {
        let mut key = Map::new();
        key.insert("host".into(), Value::String("example.test".into()));
        key.insert("port".into(), Value::from(22));
        key.insert("user".into(), Value::String("alice".into()));
        key
    }

    #[test]
    fn mutations_return_a_decryptable_updated_store() {
        let state = SecretState::default();
        state
            .create(
                SecretString::new("passphrase".into()),
                Duration::from_secs(60),
            )
            .unwrap();
        let mutation = state
            .put_secret(VaultSecretInput {
                r#type: "ssh:password".into(),
                key: password_key(),
                value: "fixture-secret".into(),
            })
            .unwrap();
        let decoded = VaultV1
            .decrypt(&mutation.stored, &SecretString::new("passphrase".into()))
            .unwrap();
        assert_eq!(decoded.secrets.len(), 1);
        assert_eq!(
            state
                .get_secret(&VaultSecretSelector {
                    r#type: "ssh:password".into(),
                    key: password_key(),
                })
                .unwrap(),
            Some("fixture-secret".into())
        );
    }

    #[test]
    fn expiration_clears_access_to_secret_values() {
        let state = SecretState::default();
        state
            .create(
                SecretString::new("passphrase".into()),
                Duration::from_millis(5),
            )
            .unwrap();
        thread::sleep(Duration::from_millis(15));
        assert!(!state.status().unlocked);
        assert!(state.snapshot().is_err());
    }

    #[test]
    fn stores_encrypted_config_in_the_same_session() {
        let state = SecretState::default();
        state
            .create(
                SecretString::new("passphrase".into()),
                Duration::from_secs(60),
            )
            .unwrap();
        let mutation = state
            .set_config(json!({ "profiles": ["fixture"] }))
            .unwrap();
        assert_eq!(mutation.summary.config["profiles"][0], "fixture");
    }

    #[test]
    fn replaces_the_whole_vault_and_changes_passphrase_once() {
        let state = SecretState::default();
        let mutation = state
            .replace(
                VaultSnapshot {
                    config: json!({ "encrypted": true }),
                    secrets: vec![VaultSnapshotSecret {
                        r#type: "ssh:password".into(),
                        key: password_key(),
                        value: "fixture-secret".into(),
                    }],
                },
                SecretString::new("new-passphrase".into()),
                Duration::from_secs(60),
            )
            .unwrap();
        assert!(VaultV1
            .decrypt(
                &mutation.stored,
                &SecretString::new("new-passphrase".into())
            )
            .is_ok());
        assert!(VaultV1
            .decrypt(
                &mutation.stored,
                &SecretString::new("old-passphrase".into())
            )
            .is_err());
    }
}
