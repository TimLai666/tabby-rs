use url::Url;

use crate::{error::AppError, storage::state_file::UpdateChannel};

use super::version::Version;

const MAX_NOTES_BYTES: usize = 64 * 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_SIGNATURE_BYTES: usize = 4096;
const MAX_PUBLISHED_AT_BYTES: usize = 128;
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    pub schema_version: u32,
    pub channel: String,
    pub platform: String,
    pub arch: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub signature: String,
    pub notes: String,
    #[serde(alias = "pub_date")]
    pub published_at: String,
    pub size: Option<u64>,
    #[serde(default)]
    pub requires_config_migration: bool,
}

impl UpdateManifest {
    pub fn validate(
        &self,
        expected_channel: &str,
        current_version: &str,
    ) -> Result<Version, AppError> {
        self.validate_for_channel(expected_channel, current_version)
    }

    pub fn validate_for_channel(
        &self,
        expected_channel: &str,
        current_version: &str,
    ) -> Result<Version, AppError> {
        if self.schema_version != 1 {
            return Err(AppError::Unsupported(
                "unsupported update manifest schema".into(),
            ));
        }
        if self.channel != expected_channel
            || !matches!(self.channel.as_str(), "stable" | "nightly")
        {
            return Err(AppError::InvalidData(
                "update manifest channel does not match the selected channel".into(),
            ));
        }
        if self.platform != std::env::consts::OS || self.arch != std::env::consts::ARCH {
            return Err(AppError::Unsupported(
                "update manifest target does not match this device".into(),
            ));
        }
        let version = Version::parse(&self.version)?;
        if (self.channel == "stable") != version.is_stable() {
            return Err(AppError::InvalidData(
                "update manifest version does not match its channel".into(),
            ));
        }
        let current = Version::parse(current_version)?;
        let channel = match expected_channel {
            "stable" => UpdateChannel::Stable,
            "nightly" => UpdateChannel::Nightly,
            _ => return Err(AppError::InvalidData("unknown update channel".into())),
        };
        if !version.is_newer_for_channel(&current, &channel) {
            return Err(AppError::InvalidData(
                "update manifest is not newer than the current version".into(),
            ));
        }
        if self.url.len() > MAX_URL_BYTES
            || self.sha256.len() != 64
            || self.signature.len() > MAX_SIGNATURE_BYTES
            || self
                .size
                .is_some_and(|size| size == 0 || size > MAX_ARTIFACT_BYTES)
        {
            return Err(AppError::InvalidData(
                "update manifest contains an oversized or invalid field".into(),
            ));
        }
        let url = Url::parse(&self.url)
            .map_err(|_| AppError::InvalidData("update URL is invalid".into()))?;
        if url.scheme() != "https"
            || url.username() != ""
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::PermissionDenied(
                "update URL must be an HTTPS URL without credentials or fragments".into(),
            ));
        }
        if !self
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
            || self.signature.is_empty()
        {
            return Err(AppError::InvalidData(
                "update manifest hash or signature is invalid".into(),
            ));
        }
        if self.notes.len() > MAX_NOTES_BYTES
            || self.published_at.is_empty()
            || self.published_at.len() > MAX_PUBLISHED_AT_BYTES
            || self.published_at.chars().any(char::is_control)
            || chrono::DateTime::parse_from_rfc3339(&self.published_at).is_err()
        {
            return Err(AppError::InvalidData(
                "update manifest notes or publication time is invalid".into(),
            ));
        }
        Ok(version)
    }
}

#[cfg(test)]
mod tests {
    use super::{UpdateManifest, MAX_ARTIFACT_BYTES, MAX_PUBLISHED_AT_BYTES};

    fn manifest() -> UpdateManifest {
        UpdateManifest {
            schema_version: 1,
            channel: "stable".into(),
            platform: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            version: "1.0.231-tabbyrs.2".into(),
            url: "https://updates.example.test/tabby-rs.zip".into(),
            sha256: "a".repeat(64),
            signature: "signed-by-ci-key".into(),
            notes: "security fixes".into(),
            published_at: "2026-08-12T00:00:00Z".into(),
            size: Some(123),
            requires_config_migration: false,
        }
    }

    #[test]
    fn validates_target_channel_and_version() {
        assert!(manifest().validate("stable", "1.0.231-tabbyrs.1").is_ok());
    }

    #[test]
    fn rejects_wrong_target_or_unsafe_url() {
        let mut value = manifest();
        value.url = "http://updates.example.test/file".into();
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
        let mut value = manifest();
        value.platform = "not-this-platform".into();
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
    }

    #[test]
    fn rejects_nightly_manifest_in_stable_channel() {
        let mut value = manifest();
        value.channel = "nightly".into();
        value.version = "1.0.231-tabbyrs.2.nightly.20260812.1".into();
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
    }

    #[test]
    fn accepts_tauri_dynamic_manifest_fields() {
        let value = serde_json::json!({
            "version": "1.0.231-tabbyrs.2",
            "notes": "security fixes",
            "url": "https://updates.example.test/tabby-rs.AppImage",
            "signature": "signed-by-ci-key",
            "schemaVersion": 1,
            "channel": "stable",
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "sha256": "a".repeat(64),
            "publishedAt": "2026-08-12T00:00:00Z"
        });
        let manifest: UpdateManifest = serde_json::from_value(value).unwrap();
        assert_eq!(manifest.published_at, "2026-08-12T00:00:00Z");
        assert!(manifest.validate("stable", "1.0.231-tabbyrs.1").is_ok());
    }

    #[test]
    fn rejects_oversized_artifacts_and_unbounded_dates() {
        let mut value = manifest();
        value.size = Some(MAX_ARTIFACT_BYTES + 1);
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
        let mut value = manifest();
        value.published_at = "x".repeat(MAX_PUBLISHED_AT_BYTES + 1);
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
    }

    #[test]
    fn rejects_malformed_publication_time() {
        let mut value = manifest();
        value.published_at = "not-a-date".into();
        assert!(value.validate("stable", "1.0.231-tabbyrs.1").is_err());
    }
}
