//! Credential store access: read an account's OAuth token + plan, and
//! best-effort delete its scoped Keychain item. Pure/GPUI-free. The macOS
//! path shells out to `security`; other platforms read the
//! `.credentials.json` file Claude Code writes instead.
//!
//! Two scopes: [`read_system_credentials`] for the ambient login every
//! profile shares, [`read_scoped_credentials`] for one managed account's
//! isolated config dir. Either way daruda reads an entry another program
//! owns and never writes one.

use std::path::Path;

#[cfg(not(target_os = "macos"))]
mod file;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "macos"))]
use file as backend;
#[cfg(target_os = "macos")]
use macos as backend;

pub use backend::{delete_scoped_credentials, read_scoped_credentials, read_system_credentials};

use crate::http::FetchError;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("credential store read failed: {0}")]
    Credentials(#[from] FetchError),
    #[error("keychain command failed")]
    Keychain,
}

/// Subscription metadata carried alongside the OAuth token. Both fields are
/// pass-through strings — providers add tiers and plan names without notice,
/// so daruda displays them verbatim rather than mapping to an enum that would
/// go stale.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanInfo {
    /// Plan tier as the provider names it — "team", "max", "pro", "plus".
    pub tier: Option<String>,
    /// Qualifier refining the tier. Anthropic's rate-limit tier
    /// ("default_claude_ai_5x", carrying the 5x/20x multiplier) is the only
    /// one so far; domains without one leave it `None`.
    pub qualifier: Option<String>,
}

/// Extract `(access token, plan info)` from the credentials JSON. A missing
/// token is fatal; missing subscription fields just leave [`PlanInfo`] slots
/// `None`.
///
/// Accepts two shapes: macOS nests the OAuth fields under `claudeAiOauth`;
/// the Linux/Windows file's exact shape was never confirmed against a live
/// install, so the same fields are also tried at the top level.
fn parse_credentials(raw: &str) -> Result<(String, PlanInfo), FetchError> {
    let v: Value = serde_json::from_str(raw.trim()).map_err(|_| FetchError::NoToken)?;
    let oauth = match &v["claudeAiOauth"] {
        Value::Object(_) => &v["claudeAiOauth"],
        _ => &v,
    };
    let token = oauth["accessToken"]
        .as_str()
        .map(str::to_string)
        .ok_or(FetchError::NoToken)?;
    let plan = PlanInfo {
        tier: oauth["subscriptionType"].as_str().map(str::to_string),
        qualifier: oauth["rateLimitTier"].as_str().map(str::to_string),
    };
    Ok((token, plan))
}

/// The `.credentials.json` the CLI writes inside a config dir — the only store
/// off macOS, and the fallback on it.
fn read_credentials_file(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    let raw = std::fs::read_to_string(config_dir.join(".credentials.json"))
        .map_err(|_| AccountError::Credentials(FetchError::NoToken))?;
    Ok(parse_credentials(&raw)?)
}

/// A digest of the **ambient** credential store entry — the one a login the
/// user ran themselves writes, shared by every profile.
///
/// A digest rather than the value: the caller only needs to know whether it
/// changed, and a secret that is never held cannot be logged by accident.
///
/// This exists to bracket a *managed* login. The reference implementation
/// daruda's account layer was ported from snapshots this entry before such a
/// login and restores it afterwards, because the CLI has been observed to
/// write it even when pointed at a config dir — which would silently replace
/// the user's own sign-in with the managed account's. Whether the installed
/// CLI still does that is unverified here, so daruda compares rather than
/// writes: a clobber that happens becomes visible instead of silent, and a
/// store daruda never writes to cannot be corrupted by this check.
///
/// `None` when there is no entry to read (including off macOS, where there is
/// no ambient Keychain item at all).
#[must_use]
pub fn system_credentials_digest() -> Option<String> {
    backend::system_credentials_digest()
}

/// Hex SHA-256 of a credential store entry, so a comparison never holds it.
#[cfg(any(test, target_os = "macos"))]
fn secret_digest(secret: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(secret))
}

#[cfg(test)]
mod tests;
