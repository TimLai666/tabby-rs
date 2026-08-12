use std::sync::Mutex;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_updater::{Update, Updater, UpdaterExt};
use tokio::sync::watch;
use url::Url;

use crate::{
    error::AppError,
    storage::{atomic_file::sha256_hex, state_file::UpdateChannel},
};

use super::{
    manifest::{UpdateManifest, MAX_ARTIFACT_BYTES},
    version::Version,
};

const UPDATE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub channel: UpdateChannel,
    pub published_at: String,
    pub notes: String,
    pub download_size: Option<u64>,
    pub requires_config_migration: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum UpdateState {
    Idle,
    Checking,
    Available {
        info: UpdateInfo,
    },
    Downloading {
        version: String,
        downloaded: u64,
        total: Option<u64>,
    },
    ReadyToInstall {
        version: String,
    },
    Installing {
        version: String,
    },
    Failed {
        stage: UpdateStage,
        public_error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStage {
    Configuration,
    Checking,
    Downloading,
    Installing,
}

struct PendingUpdate {
    update: Update,
    manifest: UpdateManifest,
    info: UpdateInfo,
    bytes: Option<Vec<u8>>,
}

pub(crate) struct DownloadHandle {
    pub update: Update,
    pub manifest: UpdateManifest,
    pub info: UpdateInfo,
    pub cancellation: watch::Receiver<bool>,
}

pub(crate) struct ReadyUpdate {
    pub update: Update,
    pub manifest: UpdateManifest,
    pub info: UpdateInfo,
    pub bytes: Vec<u8>,
}

struct ManagerState {
    state: UpdateState,
    pending: Option<PendingUpdate>,
    cancellation: Option<watch::Sender<bool>>,
}

pub struct UpdateManager {
    state: Mutex<ManagerState>,
}

impl Default for UpdateManager {
    fn default() -> Self {
        Self {
            state: Mutex::new(ManagerState {
                state: UpdateState::Idle,
                pending: None,
                cancellation: None,
            }),
        }
    }
}

impl UpdateManager {
    pub fn state(&self) -> UpdateState {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .state
            .clone()
    }

    pub fn begin_check(&self) -> Result<(), AppError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(state.state, UpdateState::Idle | UpdateState::Failed { .. }) {
            return Err(AppError::Conflict(
                "another update operation is already running".into(),
            ));
        }
        state.pending = None;
        state.cancellation = None;
        state.state = UpdateState::Checking;
        Ok(())
    }

    pub fn finish_check(
        &self,
        update: Option<(Update, UpdateManifest, UpdateInfo)>,
    ) -> Result<Option<UpdateInfo>, AppError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(state.state, UpdateState::Checking) {
            return Err(AppError::Conflict("update check was cancelled".into()));
        }
        let Some((update, manifest, info)) = update else {
            state.state = UpdateState::Idle;
            return Ok(None);
        };
        state.state = UpdateState::Available { info: info.clone() };
        state.pending = Some(PendingUpdate {
            update,
            manifest,
            info: info.clone(),
            bytes: None,
        });
        Ok(Some(info))
    }

    pub fn begin_download(&self, version: &str) -> Result<DownloadHandle, AppError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (update, manifest, info) = {
            let pending = state
                .pending
                .as_ref()
                .ok_or_else(|| AppError::NotFound("checked update".into()))?;
            if pending.info.version != version {
                return Err(AppError::InvalidArgument(
                    "requested update version is not the checked version".into(),
                ));
            }
            (
                pending.update.clone(),
                pending.manifest.clone(),
                pending.info.clone(),
            )
        };
        if !matches!(state.state, UpdateState::Available { .. }) {
            return Err(AppError::InvalidArgument(
                "update is not ready to download".into(),
            ));
        }
        let (sender, receiver) = watch::channel(false);
        state.cancellation = Some(sender);
        state.state = UpdateState::Downloading {
            version: info.version.clone(),
            downloaded: 0,
            total: manifest.size,
        };
        Ok(DownloadHandle {
            update,
            manifest,
            info,
            cancellation: receiver,
        })
    }

    pub fn set_download_progress(&self, version: &str, downloaded: u64, total: Option<u64>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if matches!(state.state, UpdateState::Downloading { .. }) {
            state.state = UpdateState::Downloading {
                version: version.into(),
                downloaded,
                total,
            };
        }
    }

    pub fn finish_download(&self, bytes: Vec<u8>) -> Result<(), AppError> {
        let manifest = self.ready_manifest()?;
        verify_download(&bytes, &manifest)?;
        let public_key = configured_public_key()
            .ok_or_else(|| AppError::Unsupported("updater public key is not configured".into()))?;
        verify_signature(&bytes, &manifest.signature, public_key)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(state.state, UpdateState::Downloading { .. }) {
            return Err(AppError::Conflict("update download was cancelled".into()));
        }
        let pending = state
            .pending
            .as_mut()
            .ok_or_else(|| AppError::Conflict("update download was cancelled".into()))?;
        pending.bytes = Some(bytes);
        let version = pending.info.version.clone();
        state.cancellation = None;
        state.state = UpdateState::ReadyToInstall { version };
        Ok(())
    }

    fn ready_manifest(&self) -> Result<UpdateManifest, AppError> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pending
            .as_ref()
            .map(|pending| pending.manifest.clone())
            .ok_or_else(|| AppError::Conflict("update download was cancelled".into()))
    }

    pub fn take_ready(&self, version: &str) -> Result<ReadyUpdate, AppError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(state.state, UpdateState::ReadyToInstall { .. }) {
            return Err(AppError::Conflict(
                "update must be downloaded before installation".into(),
            ));
        }
        let pending = state
            .pending
            .take()
            .ok_or_else(|| AppError::NotFound("downloaded update".into()))?;
        if pending.info.version != version {
            state.pending = Some(pending);
            return Err(AppError::InvalidArgument(
                "requested update version is not the downloaded version".into(),
            ));
        }
        let Some(bytes) = pending.bytes else {
            return Err(AppError::Conflict(
                "downloaded update bytes are missing".into(),
            ));
        };
        state.state = UpdateState::Installing {
            version: version.into(),
        };
        Ok(ReadyUpdate {
            update: pending.update,
            manifest: pending.manifest,
            info: pending.info,
            bytes,
        })
    }

    pub fn restore_ready(&self, ready: ReadyUpdate) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.state = UpdateState::ReadyToInstall {
            version: ready.info.version.clone(),
        };
        state.pending = Some(PendingUpdate {
            update: ready.update,
            manifest: ready.manifest,
            info: ready.info,
            bytes: Some(ready.bytes),
        });
    }

    pub fn finish_install(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending = None;
        state.cancellation = None;
        state.state = UpdateState::Idle;
    }

    pub fn fail(&self, stage: UpdateStage, public_error: impl Into<String>) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending = None;
        state.cancellation = None;
        state.state = UpdateState::Failed {
            stage,
            public_error: public_error.into(),
        };
    }

    pub fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = state.cancellation.take() {
            let _ = sender.send(true);
        }
        state.pending = None;
        state.state = UpdateState::Idle;
    }
}

pub fn verify_download(bytes: &[u8], manifest: &UpdateManifest) -> Result<(), AppError> {
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES
        || manifest.size.is_some_and(|size| size != bytes.len() as u64)
    {
        return Err(AppError::InvalidData(
            "downloaded update size does not match its manifest".into(),
        ));
    }
    if !sha256_hex(bytes).eq_ignore_ascii_case(&manifest.sha256) {
        return Err(AppError::InvalidData(
            "downloaded update hash does not match its manifest".into(),
        ));
    }
    Ok(())
}

pub fn verify_signature(
    bytes: &[u8],
    encoded_signature: &str,
    encoded_public_key: &str,
) -> Result<(), AppError> {
    let public_key = BASE64
        .decode(encoded_public_key)
        .map_err(|_| AppError::InvalidData("updater public key is invalid".into()))?;
    let public_key = std::str::from_utf8(&public_key)
        .map_err(|_| AppError::InvalidData("updater public key is invalid".into()))?;
    let public_key = PublicKey::decode(public_key)
        .map_err(|_| AppError::InvalidData("updater public key is invalid".into()))?;

    let signature = BASE64
        .decode(encoded_signature)
        .map_err(|_| AppError::InvalidData("updater signature is invalid".into()))?;
    let signature = std::str::from_utf8(&signature)
        .map_err(|_| AppError::InvalidData("updater signature is invalid".into()))?;
    let signature = Signature::decode(signature)
        .map_err(|_| AppError::InvalidData("updater signature is invalid".into()))?;

    public_key
        .verify(bytes, &signature, true)
        .map_err(|_| AppError::InvalidData("updater signature does not match artifact".into()))
}

pub fn channel_name(channel: &UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Nightly => "nightly",
    }
}

pub fn configured_endpoint(channel: &UpdateChannel) -> Option<&'static str> {
    let value = match channel {
        UpdateChannel::Stable => option_env!("TABBY_RS_UPDATE_ENDPOINT_STABLE"),
        UpdateChannel::Nightly => option_env!("TABBY_RS_UPDATE_ENDPOINT_NIGHTLY"),
    }?;
    (!value.trim().is_empty()).then_some(value)
}

pub fn configured_public_key() -> Option<&'static str> {
    let value = option_env!("TABBY_RS_UPDATE_PUBLIC_KEY")?;
    (!value.trim().is_empty()).then_some(value)
}

pub fn build_updater<R: Runtime>(
    app: &AppHandle<R>,
    channel: &UpdateChannel,
) -> Result<Updater, AppError> {
    let endpoint = configured_endpoint(channel)
        .ok_or_else(|| AppError::Unsupported("updater endpoint is not configured".into()))?;
    let public_key = configured_public_key()
        .ok_or_else(|| AppError::Unsupported("updater public key is not configured".into()))?;
    let endpoint = Url::parse(endpoint)
        .map_err(|_| AppError::InvalidData("updater endpoint is invalid".into()))?;
    if endpoint.scheme() != "https"
        || endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(AppError::PermissionDenied(
            "updater endpoint must be HTTPS without credentials or fragments".into(),
        ));
    }
    let selected_channel = channel.clone();
    let builder = app
        .updater_builder()
        .pubkey(public_key)
        .endpoints(vec![endpoint])
        .map_err(|_| AppError::InvalidData("updater endpoint is invalid".into()))?;
    builder
        .timeout(UPDATE_TIMEOUT)
        .version_comparator(move |current, remote| {
            let Ok(current) = Version::parse(&current.to_string()) else {
                return false;
            };
            let Ok(remote) = Version::parse(&remote.version.to_string()) else {
                return false;
            };
            remote.is_newer_for_channel(&current, &selected_channel)
        })
        .build()
        .map_err(|_| AppError::Unsupported("updater is unavailable".into()))
}

pub fn update_info_from_remote(
    update: &Update,
    channel: &UpdateChannel,
    current_version: &str,
) -> Result<(UpdateManifest, UpdateInfo), AppError> {
    let manifest: UpdateManifest = serde_json::from_value(update.raw_json.clone())?;
    manifest.validate_for_channel(channel_name(channel), current_version)?;
    if manifest.version != update.version {
        return Err(AppError::InvalidData(
            "update manifest version does not match the updater response".into(),
        ));
    }
    if manifest.url != update.download_url.to_string() {
        return Err(AppError::InvalidData(
            "update manifest URL does not match the updater response".into(),
        ));
    }
    if manifest.signature != update.signature {
        return Err(AppError::InvalidData(
            "update manifest signature does not match the updater response".into(),
        ));
    }
    Ok((
        manifest.clone(),
        UpdateInfo {
            version: manifest.version.clone(),
            current_version: current_version.into(),
            channel: channel.clone(),
            published_at: manifest.published_at.clone(),
            notes: manifest.notes.clone(),
            download_size: manifest.size,
            requires_config_migration: manifest.requires_config_migration,
        },
    ))
}

pub fn is_cancelled(receiver: &watch::Receiver<bool>) -> bool {
    *receiver.borrow()
}

#[cfg(test)]
mod tests {
    use super::{
        channel_name, configured_endpoint, is_cancelled, verify_download, verify_signature,
        UpdateManager, UpdateState,
    };
    use crate::{storage::state_file::UpdateChannel, update::manifest::UpdateManifest};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use tokio::sync::watch;

    fn manifest(hash: &str, size: Option<u64>) -> UpdateManifest {
        UpdateManifest {
            schema_version: 1,
            channel: "stable".into(),
            platform: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            version: "1.0.231-tabbyrs.2".into(),
            url: "https://updates.example.test/tabby-rs.AppImage".into(),
            sha256: hash.into(),
            signature: "signed".into(),
            notes: "notes".into(),
            published_at: "2026-08-12T00:00:00Z".into(),
            size,
            requires_config_migration: false,
        }
    }

    #[test]
    fn verifies_hash_and_declared_size() {
        let bytes = b"signed artifact";
        let hash = crate::storage::atomic_file::sha256_hex(bytes);
        assert!(verify_download(bytes, &manifest(&hash, Some(bytes.len() as u64))).is_ok());
        assert!(
            verify_download(bytes, &manifest(&"0".repeat(64), Some(bytes.len() as u64))).is_err()
        );
        assert!(verify_download(bytes, &manifest(&hash, Some(1))).is_err());
    }

    #[test]
    fn verifies_tauri_signature_against_artifact_and_public_key() {
        let public_key = BASE64.encode(
            "untrusted comment: minisign public key E7620F1842B4E81F\n\
             RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
        );
        let signature = BASE64.encode(
            "untrusted comment: signature from minisign secret key\n\
             RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
             trusted comment: timestamp:1555779966\tfile:test\n\
             QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==",
        );

        assert!(verify_signature(b"test", &signature, &public_key).is_ok());
        assert!(verify_signature(b"Test", &signature, &public_key).is_err());
        assert!(verify_signature(b"test", &signature, &BASE64.encode("wrong")).is_err());
    }

    #[test]
    fn cancellation_receiver_is_explicit() {
        let (sender, receiver) = watch::channel(false);
        assert!(!is_cancelled(&receiver));
        sender.send(true).unwrap();
        assert!(is_cancelled(&receiver));
    }

    #[test]
    fn channel_names_are_stable_and_nightly() {
        assert_eq!(channel_name(&UpdateChannel::Stable), "stable");
        assert_eq!(channel_name(&UpdateChannel::Nightly), "nightly");
        assert!(configured_endpoint(&UpdateChannel::Stable).is_none());
    }

    #[test]
    fn manager_serializes_checks_and_cancellation() {
        let manager = UpdateManager::default();
        manager.begin_check().unwrap();
        assert!(matches!(manager.state(), UpdateState::Checking));
        assert!(manager.begin_check().is_err());
        manager.cancel();
        assert!(matches!(manager.state(), UpdateState::Idle));

        manager.begin_check().unwrap();
        assert!(manager.finish_check(None).unwrap().is_none());
        assert!(matches!(manager.state(), UpdateState::Idle));
    }
}
