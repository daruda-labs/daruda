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
    /// The `browser_download_url` of this platform's package.
    pub asset_url: String,
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

/// How this platform's package ends. `None` where daruda publishes none —
/// there is nothing to offer, which is not the same fact as being current.
///
/// A suffix rather than a whole name: the release workflow spells the version
/// into the file, and matching it here would repeat what the tag already says.
pub fn asset_suffix() -> Option<&'static str> {
    asset_suffix_for(std::env::consts::OS)
}

/// [`asset_suffix`] with the host as a value, so the Windows answer is
/// checked from a macOS run. Kept next to `.github/workflows/release.yml`,
/// which is what actually names these files.
fn asset_suffix_for(os: &str) -> Option<&'static str> {
    match os {
        "macos" => Some(".dmg"),
        "windows" => Some("-windows-x86_64.zip"),
        _ => None,
    }
}

/// Parse a GitHub `releases/latest` JSON payload.
///
/// Returns `Ok(Some(info))` if the release version is strictly newer than
/// `current`, `Ok(None)` if it's a prerelease/draft or equal to or older than
/// `current`, and `Err(..)` if the JSON is malformed, the tag isn't a valid
/// semver version, or the release carries no package for this platform.
///
/// Prereleases and drafts are rejected here regardless of endpoint, so callers
/// don't have to rely on `/releases/latest` (which already excludes them) to
/// avoid surfacing e.g. a `0.3.0-beta` as a newer stable release.
pub fn parse_release(
    json: &str,
    current: &semver::Version,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    let suffix = asset_suffix().ok_or(UpdateError::NoPackageForPlatform(std::env::consts::OS))?;
    parse_release_with_suffix(json, current, suffix)
}

/// [`parse_release`] against a named package suffix, so a platform's
/// selection is checked without being run on it.
fn parse_release_with_suffix(
    json: &str,
    current: &semver::Version,
    suffix: &'static str,
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

    let asset_url = release
        .assets
        .iter()
        .find(|asset| asset.name.to_lowercase().ends_with(suffix))
        .map(|asset| asset.browser_download_url.clone())
        .ok_or(UpdateError::NoAssetForPlatform(suffix))?;

    Ok(Some(ReleaseInfo {
        version,
        tag: release.tag_name,
        asset_url,
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

    /// A real release carries both packages. Picking by suffix is the whole
    /// mechanism: before it, every platform took the `.dmg`.
    const BOTH_PACKAGES_JSON: &str = "{\
        \"tag_name\": \"v0.3.0\",\
        \"assets\": [\
            { \"name\": \"daruda-0.3.0.dmg\", \"browser_download_url\": \"https://github.com/d/mac\" },\
            { \"name\": \"daruda-0.3.0-windows-x86_64.zip\", \"browser_download_url\": \"https://github.com/d/win\" }\
        ]\
    }";

    fn url_for(os: &str) -> Result<String, UpdateError> {
        let current = semver::Version::parse("0.2.0").unwrap();
        let suffix = asset_suffix_for(os).ok_or(UpdateError::NoPackageForPlatform("test"))?;
        Ok(
            parse_release_with_suffix(BOTH_PACKAGES_JSON, &current, suffix)
                .unwrap()
                .unwrap()
                .asset_url,
        )
    }

    #[test]
    fn each_platform_takes_its_own_package() {
        assert_eq!(url_for("macos").unwrap(), "https://github.com/d/mac");
        assert_eq!(url_for("windows").unwrap(), "https://github.com/d/win");
    }

    /// No Linux package is published, and saying "up to date" would be a
    /// different claim than "there is nothing here for you".
    #[test]
    fn a_platform_with_no_package_is_refused_not_called_current() {
        assert!(matches!(
            url_for("linux"),
            Err(UpdateError::NoPackageForPlatform(_))
        ));
        assert_eq!(asset_suffix_for("linux"), None);
    }

    /// The suffix must not match the other platform's file: `.dmg` and
    /// `-windows-x86_64.zip` share no ending, and a release missing one
    /// package has to say so rather than hand over the other.
    #[test]
    fn a_release_missing_this_platforms_package_is_an_error() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let mac_only = "{\"tag_name\": \"v0.3.0\", \"assets\": [\
            { \"name\": \"daruda-0.3.0.dmg\", \"browser_download_url\": \"https://github.com/d/mac\" }]}";

        let result =
            parse_release_with_suffix(mac_only, &current, asset_suffix_for("windows").unwrap());

        assert!(matches!(result, Err(UpdateError::NoAssetForPlatform(_))));
    }

    #[test]
    fn selects_dmg_asset_among_multiple() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let info = parse_release_with_suffix(RELEASE_JSON, &current, ".dmg")
            .unwrap()
            .unwrap();
        assert_eq!(
            info.asset_url,
            "https://github.com/daruda-labs/daruda/releases/download/v0.3.0/daruda-0.3.0.dmg"
        );
        assert_eq!(info.version, semver::Version::parse("0.3.0").unwrap());
        assert_eq!(info.tag, "v0.3.0");
        assert_eq!(info.notes, "## Changes\n- fixed things");
    }

    #[test]
    fn newer_release_returns_some() {
        let current = semver::Version::parse("0.2.0").unwrap();
        let result = parse_release_with_suffix(RELEASE_JSON, &current, ".dmg").unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn equal_release_returns_none() {
        let current = semver::Version::parse("0.3.0").unwrap();
        let result = parse_release_with_suffix(RELEASE_JSON, &current, ".dmg").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn older_current_returns_none() {
        let current = semver::Version::parse("0.4.0").unwrap();
        let result = parse_release_with_suffix(RELEASE_JSON, &current, ".dmg").unwrap();
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
        let result = parse_release_with_suffix(json, &current, ".dmg");
        assert!(matches!(
            result,
            Err(UpdateError::NoAssetForPlatform(".dmg"))
        ));
    }

    #[test]
    fn unparseable_tag_errors() {
        let json = r#"{
            "tag_name": "garbage",
            "body": "",
            "assets": []
        }"#;
        let current = semver::Version::parse("0.2.0").unwrap();
        let result = parse_release_with_suffix(json, &current, ".dmg");
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
        assert!(
            parse_release_with_suffix(json, &current, ".dmg")
                .unwrap()
                .is_none()
        );
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
        assert!(
            parse_release_with_suffix(json, &current, ".dmg")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn normalize_tag_strips_leading_v() {
        assert_eq!(normalize_tag("v1.2.3"), "1.2.3");
        assert_eq!(normalize_tag("1.2.3"), "1.2.3");
    }
}
