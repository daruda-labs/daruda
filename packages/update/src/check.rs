//! Network side of the update flow: fetching release metadata from GitHub
//! and downloading this platform's release package.
//!
//! Everything here is blocking (`ureq`). This crate stays GPUI-free and
//! synchronous by design — the app-layer caller is responsible for running
//! these functions off the GPUI main thread.

use crate::{ReleaseInfo, UpdateError};
use std::fs::File;
use std::io::{self, Read as _};
use std::path::Path;
use std::time::Duration;

const RELEASES_LATEST: &str = "https://api.github.com/repos/daruda-labs/daruda/releases/latest";

/// The GitHub host serving the API and release pages.
const TRUSTED_HOST: &str = "github.com";

/// GitHub's user-content / release-asset domain. A release asset URL
/// (`github.com/.../releases/download/...`) 302-redirects to a CDN host under
/// this domain, and GitHub has renamed that host over time (`objects.` →
/// `release-assets.`). We trust any subdomain of this GitHub-owned domain
/// rather than pinning one CDN hostname that a future rename would break.
const TRUSTED_ASSET_DOMAIN: &str = "githubusercontent.com";

/// Upper bound on redirect hops we will follow when downloading a DMG.
const MAX_REDIRECTS: usize = 5;

/// Upper bound on the checksums file; the real one is a few hundred bytes.
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024;

/// GET the latest stable release from GitHub and compare against `current`.
/// GitHub's `/releases/latest` endpoint already excludes prereleases and drafts.
/// Blocking; call off the main thread.
pub fn check_latest(current: &semver::Version) -> Result<Option<ReleaseInfo>, UpdateError> {
    let suffix =
        crate::asset_suffix().ok_or(UpdateError::NoPackageForPlatform(std::env::consts::OS))?;
    check_latest_with_suffix(current, suffix)
}

/// Check the same trusted release endpoint for a deployment-specific asset.
/// The caller selects a fixed suffix, never an arbitrary download URL.
pub fn check_latest_with_suffix(
    current: &semver::Version,
    suffix: &'static str,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    // Redirects are followed with the default agent here (unlike `download_asset`,
    // which pins hosts per hop): the target is a fixed HTTPS GitHub API URL with
    // TLS certificate validation, and the only value derived from the response —
    // the `asset_url` — is independently host-checked when `download_asset` fetches
    // it. Release notes render as plain text, so a tampered body cannot inject.
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();

    let response = agent
        .get(RELEASES_LATEST)
        .set("User-Agent", "daruda-updater")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;

    let body = response
        .into_string()
        .map_err(|e| UpdateError::Http(e.to_string()))?;

    crate::release::parse_release_with_suffix(&body, current, suffix)
}

/// Download `url` to `dest`, streaming to disk. Rejects any URL whose host is
/// not GitHub-owned (defense against a tampered release pointing elsewhere).
///
/// Redirects are followed manually (ureq's automatic following is disabled) so
/// that EVERY hop's host is re-validated against the allowlist before we
/// connect to it. GitHub's asset URLs 302-redirect to its asset CDN
/// (`release-assets.githubusercontent.com`); without per-hop validation a
/// hijacked redirect could stream bytes from an arbitrary (even plain-`http`)
/// host.
/// Unverified on its own — [`download_verified`] is the entry callers use.
fn download_asset(url: &str, dest: &Path) -> Result<(), UpdateError> {
    // Overall cap; large for a package download.
    let response = get_trusted(url, Duration::from_secs(300))?;
    let mut reader = response.into_reader();
    let mut file = File::create(dest).map_err(|e| UpdateError::Io(e.to_string()))?;
    io::copy(&mut reader, &mut file).map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(())
}

/// Download `info`'s package to `dest` and check it against the digest the
/// release publishes beside it. A package that fails the check is deleted, so
/// nothing downstream can install it.
/// Blocking; call off the main thread.
pub fn download_verified(info: &ReleaseInfo, dest: &Path) -> Result<(), UpdateError> {
    let sums = fetch_checksums(&info.checksums_url)?;
    download_asset(&info.asset_url, dest)?;
    let verified = crate::verify::verify_package(dest, &info.asset_name, &sums);
    if verified.is_err() {
        // Best effort: the error below is what stops the install either way.
        std::fs::remove_file(dest).ok();
    }
    verified
}

/// The release's checksums file, read whole. It lists two packages, so
/// anything past [`MAX_CHECKSUMS_BYTES`] is not the file the workflow wrote.
fn fetch_checksums(url: &str) -> Result<String, UpdateError> {
    let response = get_trusted(url, Duration::from_secs(30))?;
    let mut sums = String::new();
    response
        .into_reader()
        .take(MAX_CHECKSUMS_BYTES)
        .read_to_string(&mut sums)
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    Ok(sums)
}

/// GET `url`, following redirects by hand so every hop's host is checked
/// against the allowlist before it is contacted.
fn get_trusted(url: &str, timeout: Duration) -> Result<ureq::Response, UpdateError> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0) // we follow manually so we can re-validate each hop
        .timeout(timeout)
        .build();

    let mut current = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        if !is_trusted_host(&current) {
            return Err(UpdateError::UntrustedHost(current));
        }

        let response = agent
            .get(&current)
            .set("User-Agent", "daruda-updater")
            .call()
            .map_err(|e| UpdateError::Http(e.to_string()))?;

        let status = response.status();
        if (300..400).contains(&status) {
            current = redirect_target(status, response.header("Location"))?;
            continue;
        }

        return Ok(response);
    }

    Err(UpdateError::Http("too many redirects".to_string()))
}

/// Validate a redirect `Location` before following it. Only ABSOLUTE `https://`
/// targets are accepted; a missing, relative, or non-`https` `Location` is
/// rejected rather than guessed at. (The host itself is re-validated against
/// the allowlist by the next loop iteration.)
fn redirect_target(status: u16, location: Option<&str>) -> Result<String, UpdateError> {
    let location =
        location.ok_or_else(|| UpdateError::Http(format!("redirect {status} without Location")))?;
    if !location.starts_with("https://") {
        return Err(UpdateError::UntrustedHost(location.to_string()));
    }
    Ok(location.to_string())
}

/// Extract the host from a URL and check it against the trust allowlist.
/// Requires `https://`, drops any userinfo (`user@host`) and port
/// (`host:port`), then accepts exactly `github.com` or any host equal to or a
/// subdomain of the GitHub-owned asset domain `githubusercontent.com`. The
/// subdomain check requires a leading dot so a lookalike like
/// `githubusercontent.com.evil.com` or `evilgithubusercontent.com` is rejected.
fn is_trusted_host(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };

    let authority = rest.split('/').next().unwrap_or("");
    let host_and_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = host_and_port.split(':').next().unwrap_or(host_and_port);
    let host = host.to_ascii_lowercase();

    host == TRUSTED_HOST
        || host == TRUSTED_ASSET_DOMAIN
        || host.ends_with(&format!(".{TRUSTED_ASSET_DOMAIN}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_github_release_download_url() {
        assert!(is_trusted_host(
            "https://github.com/daruda-labs/daruda/releases/download/v0.3.0/x.dmg"
        ));
    }

    #[test]
    fn accepts_github_objects_host() {
        assert!(is_trusted_host(
            "https://objects.githubusercontent.com/github-production-release-asset/x.dmg"
        ));
    }

    #[test]
    fn accepts_github_release_assets_host() {
        // The host a release-asset download actually 302-redirects to today.
        assert!(is_trusted_host(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?sig=x"
        ));
    }

    #[test]
    fn accepts_github_com_with_explicit_port() {
        assert!(is_trusted_host("https://github.com:443/x.dmg"));
    }

    #[test]
    fn accepts_github_com_after_stripping_userinfo() {
        assert!(is_trusted_host("https://evil.com@github.com/x.dmg"));
    }

    #[test]
    fn rejects_subdomain_lookalike() {
        assert!(!is_trusted_host("https://github.com.evil.com/x.dmg"));
    }

    #[test]
    fn rejects_asset_domain_lookalikes() {
        // Suffix appended after the trusted domain.
        assert!(!is_trusted_host(
            "https://githubusercontent.com.evil.com/x.dmg"
        ));
        // Trusted domain glued to a longer label without a dot boundary.
        assert!(!is_trusted_host("https://evilgithubusercontent.com/x.dmg"));
    }

    #[test]
    fn rejects_unrelated_host() {
        assert!(!is_trusted_host("https://notgithub.com/x.dmg"));
    }

    #[test]
    fn rejects_non_https_scheme() {
        assert!(!is_trusted_host("http://github.com/x.dmg"));
    }

    #[test]
    fn accepts_case_insensitive_host() {
        assert!(is_trusted_host("https://GitHub.com/x.dmg"));
    }

    #[test]
    fn download_asset_rejects_untrusted_initial_host() {
        // First-hop allowlist gate: an untrusted URL is rejected before any
        // connection is attempted, so this never touches the network.
        let dest = std::env::temp_dir().join("daruda-update-should-not-exist.dmg");
        let result = download_asset("https://evil.example/x.dmg", &dest);
        assert!(matches!(result, Err(UpdateError::UntrustedHost(_))));
    }

    /// The checksums are fetched under the same host rule as the package, and
    /// first: a release pointing them elsewhere downloads nothing.
    #[test]
    fn download_verified_rejects_untrusted_checksums_before_any_download() {
        let dest = tempfile::tempdir().unwrap().path().join("daruda.dmg");
        let info = ReleaseInfo {
            version: semver::Version::new(0, 3, 0),
            tag: "v0.3.0".into(),
            asset_url: "https://github.com/x.dmg".into(),
            asset_name: "x.dmg".into(),
            checksums_url: "https://evil.example/SHA256SUMS.txt".into(),
            notes: String::new(),
        };
        assert!(matches!(
            download_verified(&info, &dest),
            Err(UpdateError::UntrustedHost(_))
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn redirect_target_rejects_non_https_location() {
        let result = redirect_target(302, Some("http://evil.example/x.dmg"));
        assert!(matches!(result, Err(UpdateError::UntrustedHost(_))));
    }

    #[test]
    fn redirect_target_rejects_relative_location() {
        let result = redirect_target(302, Some("/download/x.dmg"));
        assert!(matches!(result, Err(UpdateError::UntrustedHost(_))));
    }

    #[test]
    fn redirect_target_rejects_missing_location() {
        let result = redirect_target(302, None);
        assert!(matches!(result, Err(UpdateError::Http(_))));
    }

    #[test]
    fn redirect_target_accepts_absolute_https_location() {
        let result = redirect_target(302, Some("https://objects.githubusercontent.com/x.dmg"));
        assert_eq!(
            result.unwrap(),
            "https://objects.githubusercontent.com/x.dmg"
        );
    }
}
