use url::Url;

use crate::error::AppError;

use super::version::Version;

const MAX_NOTES_BYTES: usize = 64 * 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_SIGNATURE_BYTES: usize = 4096;

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
    pub published_at: String,
    pub size: Option<u64>,
}

impl UpdateManifest {
    pub fn validate(
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
        if !version.is_newer_than(&current) {
            return Err(AppError::InvalidData(
                "update manifest is not newer than the current version".into(),
            ));
        }
        if self.url.len() > MAX_URL_BYTES
            || self.sha256.len() != 64
            || self.signature.len() > MAX_SIGNATURE_BYTES
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
        if self.notes.len() > MAX_NOTES_BYTES || self.published_at.is_empty() {
            return Err(AppError::InvalidData(
                "update manifest notes or publication time is invalid".into(),
            ));
        }
        Ok(version)
    }
}

#[cfg(test)]
mod tests {
    use super::UpdateManifest;

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
}
