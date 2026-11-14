use std::time::Duration;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::AppError;

use super::{service::configure_updater_client, version::Version};

// Mirrors `fetch` of UPDATES_URL in tabby-electron/src/services/updater.service.ts.
// Upstream reads only this latest-release endpoint and does not distinguish channels.
/// This project's own releases. Upstream points at `eugeny/tabby`; Tabby RS
/// ships separately, so its users must be sent to the Tabby RS release page.
pub const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/TimLai666/tabby-rs/releases/latest";
const RELEASE_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_RELEASE_BODY_BYTES: usize = 1024 * 1024;
const USER_AGENT: &str = "tabby-rs";

#[derive(Debug, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePageInfo {
    pub version: String,
    pub release_url: String,
    pub release_notes: String,
    pub release_date: String,
    pub is_newer: bool,
}

fn invalid_release(label: &str) -> AppError {
    AppError::InvalidData(format!("release tag {label} is invalid"))
}

// Release tags carry only `x.y.z`, so they cannot be parsed with the `-tabbyrs.N` suffix.
pub fn release_version(tag: &str) -> Result<(u64, u64, u64), AppError> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    let base = stripped.split('-').next().unwrap_or_default();
    let mut parts = base.split('.');
    let mut next = |label: &str| -> Result<u64, AppError> {
        let value = parts.next().ok_or_else(|| invalid_release(label))?;
        if value.is_empty()
            || !value.bytes().all(|byte| byte.is_ascii_digit())
            || (value.len() > 1 && value.starts_with('0'))
        {
            return Err(invalid_release(label));
        }
        value.parse::<u64>().map_err(|_| invalid_release(label))
    };
    let major = next("major")?;
    let minor = next("minor")?;
    let patch = next("patch")?;
    if parts.next().is_some() {
        return Err(invalid_release("version"));
    }
    Ok((major, minor, patch))
}

// The app version keeps the project's `-tabbyrs.N` suffix; only its base is compared.
pub fn is_newer_release(
    current_version: &str,
    release_version_text: &str,
) -> Result<bool, AppError> {
    let current = Version::parse(current_version)?;
    let remote = release_version(release_version_text)?;
    Ok(remote > (current.major, current.minor, current.patch))
}

pub fn validate_release_url(value: &str) -> Result<String, AppError> {
    let url =
        Url::parse(value).map_err(|_| AppError::InvalidData("release URL is invalid".into()))?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::PermissionDenied(
            "release URL must be HTTPS without credentials".into(),
        ));
    }
    // Defence in depth: `html_url` comes from GitHub over TLS without redirects,
    // but only this project's release pages are ever handed to the browser.
    if url.host_str() != Some(RELEASE_PAGE_HOST)
        || !url.path().starts_with(RELEASE_PAGE_PATH_PREFIX)
    {
        return Err(AppError::PermissionDenied(
            "release URL must point at this project's GitHub releases".into(),
        ));
    }
    Ok(value.to_string())
}

/// Host and path prefix every release page must have before it is opened.
pub const RELEASE_PAGE_HOST: &str = "github.com";
pub const RELEASE_PAGE_PATH_PREFIX: &str = "/TimLai666/tabby-rs/releases/";

pub fn parse_latest_release(body: &[u8]) -> Result<GithubRelease, AppError> {
    Ok(serde_json::from_slice(body)?)
}

pub fn release_page_info(
    release: &GithubRelease,
    current_version: &str,
) -> Result<ReleasePageInfo, AppError> {
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_string();
    let is_newer = is_newer_release(current_version, &version)?;
    let release_url = validate_release_url(&release.html_url)?;
    Ok(ReleasePageInfo {
        version,
        release_url,
        release_notes: release.body.clone().unwrap_or_default(),
        release_date: release.published_at.clone().unwrap_or_default(),
        is_newer,
    })
}

pub fn append_bounded(buffer: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), AppError> {
    let total = buffer
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| AppError::InvalidData("release response is too large".into()))?;
    if total > limit {
        return Err(AppError::InvalidData(
            "release response is too large".into(),
        ));
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

// Performs only a GET of the release metadata. It never downloads or executes an artifact.
pub async fn fetch_latest_release() -> Result<GithubRelease, AppError> {
    let client = configure_updater_client(reqwest::Client::builder())
        .timeout(RELEASE_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| AppError::Io("release page client could not be created".into()))?;
    let mut response = client
        .get(RELEASES_LATEST_URL)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| AppError::Io("release page check failed".into()))?;
    if !response.status().is_success() {
        return Err(AppError::Io("release page check failed".into()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RELEASE_BODY_BYTES as u64)
    {
        return Err(AppError::InvalidData(
            "release response is too large".into(),
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::Io("release page check failed".into()))?
    {
        append_bounded(&mut body, &chunk, MAX_RELEASE_BODY_BYTES)?;
    }
    parse_latest_release(&body)
}

#[cfg(test)]
mod tests {
    use super::{
        append_bounded, is_newer_release, parse_latest_release, release_page_info, release_version,
        GithubRelease, ReleasePageInfo, MAX_RELEASE_BODY_BYTES,
    };

    fn release(tag: &str, url: &str, body: Option<&str>, published: Option<&str>) -> GithubRelease {
        GithubRelease {
            tag_name: tag.into(),
            html_url: url.into(),
            body: body.map(str::to_string),
            published_at: published.map(str::to_string),
        }
    }

    #[test]
    fn parses_release_tags_with_or_without_the_v_prefix() {
        assert_eq!(release_version("v1.0.235").unwrap(), (1, 0, 235));
        assert_eq!(release_version("1.0.235").unwrap(), (1, 0, 235));
        assert_eq!(release_version("v10.20.300").unwrap(), (10, 20, 300));
    }

    #[test]
    fn rejects_release_tags_that_are_not_plain_numeric_versions() {
        for value in [
            "", "v", "v1.0", "v1.0.x", "v01.0.1", "v1.0.1.2", "vv1.0.1", "v1..1",
        ] {
            assert!(release_version(value).is_err(), "{value}");
        }
    }

    #[test]
    fn newer_release_compares_against_the_app_base_version() {
        assert!(is_newer_release("1.0.231-tabbyrs.1", "1.0.235").unwrap());
        assert!(is_newer_release("1.0.231-tabbyrs.4", "1.1.0").unwrap());
        assert!(is_newer_release("1.0.231-tabbyrs.4.nightly.20260812.1", "2.0.0").unwrap());
    }

    #[test]
    fn same_or_older_release_is_not_newer() {
        assert!(!is_newer_release("1.0.231-tabbyrs.1", "1.0.231").unwrap());
        assert!(!is_newer_release("1.0.231-tabbyrs.4.nightly.20260812.1", "1.0.231").unwrap());
        assert!(!is_newer_release("1.0.231-tabbyrs.1", "1.0.230").unwrap());
    }

    #[test]
    fn rejects_app_versions_without_the_project_suffix() {
        assert!(is_newer_release("1.0.231", "1.0.235").is_err());
    }

    #[test]
    fn maps_a_release_to_the_fallback_info() {
        let info = release_page_info(
            &release(
                "v1.0.235",
                "https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235",
                Some("notes"),
                Some("2026-11-10T12:00:00Z"),
            ),
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        assert_eq!(
            info,
            ReleasePageInfo {
                version: "1.0.235".into(),
                release_url: "https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235".into(),
                release_notes: "notes".into(),
                release_date: "2026-11-10T12:00:00Z".into(),
                is_newer: true,
            }
        );
    }

    #[test]
    fn missing_body_and_date_become_empty_strings() {
        let info = release_page_info(
            &release(
                "v1.0.231",
                "https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.231",
                None,
                None,
            ),
            "1.0.231-tabbyrs.1",
        )
        .unwrap();
        assert_eq!(info.release_notes, "");
        assert_eq!(info.release_date, "");
        assert!(!info.is_newer);
    }

    #[test]
    fn release_urls_must_be_https_without_credentials() {
        for url in [
            "http://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://user:secret@github.com/TimLai666/tabby-rs/releases/tag/v1.0.235",
            "https://github.com/eugeny/tabby/releases/tag/v1.0.235",
            "https://example.com/TimLai666/tabby-rs/releases/tag/v1.0.235",
            "https://github.com/TimLai666/tabby-rs/issues/1",
            "not a url",
        ] {
            assert!(
                release_page_info(&release("v1.0.235", url, None, None), "1.0.231-tabbyrs.1")
                    .is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn parses_only_json_release_objects() {
        let parsed = parse_latest_release(
            br#"{"tag_name":"v1.0.235","html_url":"https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235","body":"b","published_at":"d","assets":[]}"#,
        )
        .unwrap();
        assert_eq!(parsed.tag_name, "v1.0.235");
        assert!(parse_latest_release(b"<html>").is_err());
        assert!(parse_latest_release(
            br#"{"html_url":"https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235"}"#
        )
        .is_err());
    }

    #[test]
    fn bounded_append_accepts_the_limit_and_keeps_the_buffer_on_overflow() {
        let mut buffer = Vec::new();
        append_bounded(&mut buffer, &[1, 2, 3], 4).unwrap();
        append_bounded(&mut buffer, &[4], 4).unwrap();
        assert_eq!(buffer, vec![1, 2, 3, 4]);
        assert!(append_bounded(&mut buffer, &[5], 4).is_err());
        assert_eq!(buffer, vec![1, 2, 3, 4]);
        let mut large = Vec::new();
        assert!(append_bounded(
            &mut large,
            &vec![0; MAX_RELEASE_BODY_BYTES + 1],
            MAX_RELEASE_BODY_BYTES
        )
        .is_err());
        assert!(large.is_empty());
    }

    #[test]
    fn serializes_the_camel_case_response_shape() {
        let value = serde_json::to_value(ReleasePageInfo {
            version: "1.0.235".into(),
            release_url: "https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235".into(),
            release_notes: String::new(),
            release_date: String::new(),
            is_newer: true,
        })
        .unwrap();
        assert_eq!(
            value["releaseUrl"],
            "https://github.com/TimLai666/tabby-rs/releases/tag/v1.0.235"
        );
        assert_eq!(value["releaseNotes"], "");
        assert_eq!(value["releaseDate"], "");
        assert_eq!(value["isNewer"], true);
        assert_eq!(value["version"], "1.0.235");
    }
}
