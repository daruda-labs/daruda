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

use crate::http::FetchError;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use serde_json::Value;

/// Keychain service holding the ambient Claude login — the entry the CLI
/// writes when no per-account dir scopes it. Shared by every daruda profile
/// and by the user's own terminal usage.
#[cfg(target_os = "macos")]
const SYSTEM_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

#[allow(unused_imports)] // used only on macOS (scoped Keychain service name)
use super::layout::scoped_keychain_service;

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

/// System-wide Claude Code login: the macOS Keychain item, or the
/// `.credentials.json` file (`$CLAUDE_CONFIG_DIR`-aware) elsewhere. Every
/// failure — no login yet, non-macOS build, malformed contents — collapses to
/// [`FetchError::NoToken`] so callers render one "unavailable" state.
#[cfg(target_os = "macos")]
pub fn read_system_credentials() -> Result<(String, PlanInfo), FetchError> {
    use std::process::Command;
    let out = Command::new("security")
        .args(["find-generic-password", "-s", SYSTEM_KEYCHAIN_SERVICE, "-w"])
        .output()
        .map_err(|_| FetchError::NoToken)?;
    if !out.status.success() {
        return Err(FetchError::NoToken);
    }
    let raw = String::from_utf8(out.stdout).map_err(|_| FetchError::NoToken)?;
    parse_credentials(&raw)
}

#[cfg(not(target_os = "macos"))]
pub fn read_system_credentials() -> Result<(String, PlanInfo), FetchError> {
    let path = credentials_path().ok_or(FetchError::NoToken)?;
    let raw = std::fs::read_to_string(path).map_err(|_| FetchError::NoToken)?;
    parse_credentials(&raw)
}

#[cfg(not(target_os = "macos"))]
fn credentials_path() -> Option<std::path::PathBuf> {
    credentials_path_from(std::env::var_os("CLAUDE_CONFIG_DIR"), dirs::home_dir())
}

/// Pure core of [`credentials_path`], split out so the `$CLAUDE_CONFIG_DIR`
/// override and the `~/.claude` default can be unit-tested without mutating
/// the real process environment (parallel `cargo test` runs share one).
#[cfg(not(target_os = "macos"))]
fn credentials_path_from(
    config_dir: Option<std::ffi::OsString>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if let Some(dir) = config_dir {
        return Some(std::path::PathBuf::from(dir).join(".credentials.json"));
    }
    Some(home?.join(".claude").join(".credentials.json"))
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

/// Read the OAuth token + plan for a specific account's config dir.
///
/// On macOS the Keychain is where the CLI normally puts them, but not the only
/// place it ever has: a build that writes `.credentials.json` into the config
/// dir instead would otherwise read as "no credentials" — and a *successful*
/// login reported that way is not merely a wrong label, it makes the add flow
/// discard the directory it just created. Trying the file after the Keychain
/// costs one `stat` on the normal path.
#[cfg(target_os = "macos")]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    match read_scoped_keychain_credentials(config_dir) {
        Ok(found) => Ok(found),
        Err(keychain_error) => read_credentials_file(config_dir).map_err(|_| keychain_error),
    }
}

#[cfg(target_os = "macos")]
fn read_scoped_keychain_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    use std::process::Command;
    let service = scoped_keychain_service(config_dir);
    let out = Command::new("security")
        .args(["find-generic-password", "-s", &service, "-w"])
        .output()
        .map_err(|_| AccountError::Keychain)?;
    if !out.status.success() {
        return Err(AccountError::Credentials(FetchError::NoToken));
    }
    let raw = String::from_utf8(out.stdout).map_err(|_| AccountError::Keychain)?;
    Ok(parse_credentials(&raw)?)
}

#[cfg(not(target_os = "macos"))]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    read_credentials_file(config_dir)
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
    #[cfg(target_os = "macos")]
    {
        use sha2::{Digest, Sha256};
        use std::process::Command;
        let out = Command::new("security")
            .args(["find-generic-password", "-s", SYSTEM_KEYCHAIN_SERVICE, "-w"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let mut hasher = Sha256::new();
        hasher.update(&out.stdout);
        Some(format!("{:x}", hasher.finalize()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Best-effort delete of the scoped macOS Keychain item a Claude Code
/// login writes into `config_dir`'s isolated `CLAUDE_CONFIG_DIR` (see
/// [`scoped_keychain_service`]). Called by every `app`-side cleanup path
/// that discards a login attempt without keeping it (dedup hit, denied,
/// timed out, failed, or cancelled) so a discarded login never leaves an
/// orphaned OS credential behind. Mirrors the `security
/// delete-generic-password` invocation in
/// `app/src/telegram/keychain.rs::delete_token`. "Item not found" is the
/// expected common case (the login never got far enough to write
/// credentials) and is silently ignored; any other failure is logged,
/// not surfaced — same "no functional impact" call as the config-dir
/// removal this runs alongside.
#[cfg(target_os = "macos")]
pub fn delete_scoped_credentials(config_dir: &Path) {
    use std::process::{Command, Stdio};

    let service = scoped_keychain_service(config_dir);
    let output = Command::new("security")
        .args(["delete-generic-password", "-s", &service])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !stderr.contains("could not be found") {
                LogWriter::log(
                    ErrorReport::new("Failed to delete scoped account Keychain item")
                        .message(stderr)
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("account.add.cleanup_keychain_failed")
                        .build(),
                );
            }
        }
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Failed to delete scoped account Keychain item")
                    .from_error(&e)
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("account.add.cleanup_keychain_failed")
                    .build(),
            );
        }
    }
}

/// Non-macOS no-op: the scoped credential lives in `.credentials.json`
/// inside `config_dir`, already removed by the caller's directory
/// cleanup (`std::fs::remove_dir_all`) — there is no separate OS
/// credential store to touch.
#[cfg(not(target_os = "macos"))]
pub fn delete_scoped_credentials(_config_dir: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_extract_token_and_plan_info() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "sk-ant-oat01-abc",
                "refreshToken": "sk-ant-ort01-def",
                "expiresAt": 1778112000000,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "team",
                "rateLimitTier": "default_claude_ai_5x"
            }
        }"#;
        let (token, plan) = parse_credentials(raw).unwrap();
        assert_eq!(token, "sk-ant-oat01-abc");
        assert_eq!(plan.tier.as_deref(), Some("team"));
        assert_eq!(plan.qualifier.as_deref(), Some("default_claude_ai_5x"));
    }

    #[test]
    fn credentials_tolerate_missing_plan_fields() {
        // Older payloads carry only the token — the read must still proceed.
        let (token, plan) =
            parse_credentials(r#"{ "claudeAiOauth": { "accessToken": "tok" } }"#).unwrap();
        assert_eq!(token, "tok");
        assert_eq!(plan.tier, None);
        assert_eq!(plan.qualifier, None);
    }

    #[test]
    fn credentials_without_a_token_is_no_token() {
        let err =
            parse_credentials(r#"{ "claudeAiOauth": { "subscriptionType": "pro" } }"#).unwrap_err();
        assert!(matches!(err, FetchError::NoToken));
    }

    #[test]
    fn malformed_json_is_no_token() {
        assert!(matches!(
            parse_credentials("{ not json").unwrap_err(),
            FetchError::NoToken
        ));
    }

    #[test]
    fn credentials_accept_the_flat_shape() {
        // Claude Code CLI's Linux/Windows `.credentials.json` shape was never
        // confirmed against a live install — this fixture is the defensive
        // fallback (fields at the top level), not a verified-real sample.
        let raw = r#"{
            "accessToken": "sk-ant-oat01-flat",
            "subscriptionType": "pro",
            "rateLimitTier": "default_claude_ai"
        }"#;
        let (token, plan) = parse_credentials(raw).unwrap();
        assert_eq!(token, "sk-ant-oat01-flat");
        assert_eq!(plan.tier.as_deref(), Some("pro"));
        assert_eq!(plan.qualifier.as_deref(), Some("default_claude_ai"));
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn credentials_path_prefers_the_claude_config_dir_override() {
        assert_eq!(
            credentials_path_from(
                Some(std::ffi::OsString::from("/custom/claude-dir")),
                Some(std::path::PathBuf::from("/home/someone")),
            ),
            Some(std::path::PathBuf::from(
                "/custom/claude-dir/.credentials.json"
            ))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn credentials_path_falls_back_to_home_dot_claude() {
        assert_eq!(
            credentials_path_from(None, Some(std::path::PathBuf::from("/home/someone"))),
            Some(std::path::PathBuf::from(
                "/home/someone/.claude/.credentials.json"
            ))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn credentials_path_is_none_without_config_dir_or_home() {
        assert_eq!(credentials_path_from(None, None), None);
    }

    /// A config dir whose credentials landed in the file rather than the
    /// Keychain still has credentials. Reading only the Keychain reports a
    /// successful login as a failed one — and the add flow deletes the
    /// directory on that answer.
    #[test]
    fn a_config_dir_with_only_a_credentials_file_is_still_signed_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","subscriptionType":"pro"}}"#,
        )
        .expect("fixture");
        let (token, plan) = read_scoped_credentials(dir.path()).expect("the file is read");
        assert_eq!(token, "sk-ant-oat01-x");
        assert_eq!(plan.tier.as_deref(), Some("pro"));
    }

    /// Neither store holding anything is still "signed out".
    #[test]
    fn an_empty_config_dir_has_no_credentials() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read_scoped_credentials(dir.path()).is_err());
    }

    /// Reading it twice with nothing in between must agree — otherwise the
    /// comparison this exists for would report a clobber on every login.
    #[test]
    fn the_ambient_digest_is_stable_across_reads() {
        assert_eq!(system_credentials_digest(), system_credentials_digest());
    }

    /// And it must never be the secret itself.
    #[test]
    fn the_ambient_digest_is_a_digest() {
        if let Some(d) = system_credentials_digest() {
            assert_eq!(d.len(), 64, "sha256 hex");
            assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!d.contains("sk-ant"));
        }
    }
}
