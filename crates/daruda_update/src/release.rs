//! Pure parsing of GitHub `releases/latest` JSON payloads into `ReleaseInfo`.
//!
//! No networking happens here — callers fetch the JSON body themselves and
//! pass it to `parse_release`.

use crate::UpdateError;
use serde::Deserialize;

/// A parsed, newer-than-current release ready to be downloaded and installed.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub version: semver::Version,
    /// The original release tag, e.g. `"v0.3.0"`.
    pub tag: String,
    /// The `browser_download_url` of the release's `.dmg` asset.
    pub dmg_url: String,
    /// The release body/notes, verbatim.
    pub notes: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Strip a leading `v` from a release tag before semver parsing.
fn normalize_tag(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

/// Parse a GitHub `releases/latest` JSON payload.
///
/// Returns `Ok(Some(info))` if the release version is strictly newer than
/// `current`, `Ok(None)` if it's a prerelease/draft or equal to or older than
/// `current`, and `Err(..)` if the JSON is malformed, the tag isn't a valid
/// semver version, or the release has no `.dmg` asset.
///
/// Prereleases and drafts are rejected here regardless of endpoint, so callers
/// don't have to rely on `/releases/latest` (which already excludes them) to
/// avoid surfacing e.g. a `0.3.0-beta` as a newer stable release.
pub fn parse_release(
    json: &str,
    current: &semver::Version,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    let release: GithubRelease =
        serde_json::from_str(json).map_err(|e| UpdateError::Parse(e.to_string()))?;

    if release.prerelease || release.draft {
        return Ok(None);
    }

    let version = semver::Version::parse(normalize_tag(&release.tag_name))
        .map_err(|e| UpdateError::Parse(e.to_string()))?;

    if version <= *current {
        return Ok(None);
    }

    let dmg_url = release
        .assets
        .iter()
        .find(|asset| asset.name.to_lowercase().ends_with(".dmg"))
        .map(|asset| asset.browser_download_url.clone())
        .ok_or(UpdateError::NoDmgAsset)?;

    Ok(Some(ReleaseInfo {
        version,
        tag: release.tag_name,
        dmg_url,
        notes: release.body.unwrap_or_default(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE_JSON: &str = "{\
        \"tag_name\": \"v0.3.0\",\
        \"body\": \"## Changes\\n- fixed things\",\
        \"assets\": [\
            { \"name\": \"daruda-0.3.0.dmg\", \"browser_download_url\": \"https://github.com/daruda-labs/daruda/releases/download/v0.3.0/daruda-0.3.0.dmg\" },\
            { \"name\": \"something-else.txt\", \"browser_download_url\": \"https://example.com/other\" }\
        ]\
    }";

    #[test]
    fn selects_dmg_asset_among_multiple() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let info = parse_release(RELEASE_JSON, &current).unwrap().unwrap();
        assert_eq!(
            info.dmg_url,
            "https://github.com/daruda-labs/daruda/releases/download/v0.3.0/daruda-0.3.0.dmg"
        );
        assert_eq!(info.version, semver::Version::parse("0.3.0").unwrap());
        assert_eq!(info.tag, "v0.3.0");
        assert_eq!(info.notes, "## Changes\n- fixed things");
    }

    #[test]
    fn newer_release_returns_some() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let result = parse_release(RELEASE_JSON, &current).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn equal_release_returns_none() {
        let current = semver::Version::parse("0.3.0").unwrap();
        let result = parse_release(RELEASE_JSON, &current).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn older_current_returns_none() {
        let current = semver::Version::parse("0.4.0").unwrap();
        let result = parse_release(RELEASE_JSON, &current).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn no_dmg_asset_errors() {
        let json = r#"{
            "tag_name": "v0.3.0",
            "body": "",
            "assets": [
                { "name": "something-else.txt", "browser_download_url": "https://example.com/other" }
            ]
        }"#;
        let current = semver::Version::parse("0.2.0").unwrap();
        let result = parse_release(json, &current);
        assert!(matches!(result, Err(UpdateError::NoDmgAsset)));
    }

    #[test]
    fn unparseable_tag_errors() {
        let json = r#"{
            "tag_name": "garbage",
            "body": "",
            "assets": []
        }"#;
        let current = semver::Version::parse("0.2.0").unwrap();
        let result = parse_release(json, &current);
        assert!(matches!(result, Err(UpdateError::Parse(_))));
    }

    #[test]
    fn prerelease_returns_none_even_when_newer() {
        let json = r#"{
            "tag_name": "v0.9.0",
            "prerelease": true,
            "body": "",
            "assets": [
                { "name": "daruda-0.9.0.dmg", "browser_download_url": "https://github.com/daruda-labs/daruda/releases/download/v0.9.0/daruda-0.9.0.dmg" }
            ]
        }"#;
        let current = semver::Version::parse("0.2.0").unwrap();
        assert!(parse_release(json, &current).unwrap().is_none());
    }

    #[test]
    fn draft_returns_none_even_when_newer() {
        let json = r#"{
            "tag_name": "v0.9.0",
            "draft": true,
            "body": "",
            "assets": [
                { "name": "daruda-0.9.0.dmg", "browser_download_url": "https://github.com/daruda-labs/daruda/releases/download/v0.9.0/daruda-0.9.0.dmg" }
            ]
        }"#;
        let current = semver::Version::parse("0.2.0").unwrap();
        assert!(parse_release(json, &current).unwrap().is_none());
    }

    #[test]
    fn normalize_tag_strips_leading_v() {
        assert_eq!(normalize_tag("v1.2.3"), "1.2.3");
        assert_eq!(normalize_tag("1.2.3"), "1.2.3");
    }
}
