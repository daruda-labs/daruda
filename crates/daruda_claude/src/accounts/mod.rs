//! Per-account runtime isolation for Claude: config-dir layout and
//! config-dir-scoped Keychain reads. Pure/GPUI-free.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::http::FetchError;
use crate::limits::{PlanInfo, parse_keychain_credentials};
use daruda_store::accounts::AccountId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

pub mod identity;
pub mod login;

pub use identity::{AccountIdentity, read_account_identity};
pub use login::{
    LoginError, LoginOutcome, LoginProcess, LoginProcessHandle, is_oauth_denied, spawn_login,
};

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("credential store read failed: {0}")]
    Credentials(#[from] FetchError),
    #[error("keychain command failed")]
    Keychain,
}

/// Root that holds every managed account's isolated config dir.
pub fn accounts_root(data_dir: &Path) -> PathBuf {
    data_dir.join("claude-accounts")
}

/// The `CLAUDE_CONFIG_DIR` for one account.
pub fn account_config_dir(data_dir: &Path, id: AccountId) -> PathBuf {
    accounts_root(data_dir).join(id.0.to_string())
}

/// Claude Code 2.1+ scopes the macOS Keychain credential item by
/// `sha256(CLAUDE_CONFIG_DIR)` (first 8 hex). Mirrors orca `keychain.ts:85`.
pub fn scoped_keychain_service(config_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(config_dir.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let suffix: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("Claude Code-credentials-{suffix}")
}

/// Read the OAuth token + plan for a specific account's config dir.
#[cfg(target_os = "macos")]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
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
    Ok(parse_keychain_credentials(&raw)?)
}

#[cfg(not(target_os = "macos"))]
pub fn read_scoped_credentials(config_dir: &Path) -> Result<(String, PlanInfo), AccountError> {
    let path = config_dir.join(".credentials.json");
    let raw = std::fs::read_to_string(path)
        .map_err(|_| AccountError::Credentials(FetchError::NoToken))?;
    Ok(parse_keychain_credentials(&raw)?)
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

/// Startup sweep: remove per-account config dirs under [`accounts_root`]
/// that never got promoted to a [`ManagedAccount`] (login cancelled or
/// the app crashed mid-login). A subdirectory is an orphan when its name
/// doesn't match any id in `known` — this also catches garbage/leftover
/// dirs whose name was never a valid UUID to begin with, since those
/// can't match either. Best-effort: any individual dir's Keychain or
/// filesystem failure is skipped, never panics, and never aborts the
/// sweep of the remaining dirs. Only touches entries directly under
/// `accounts_root(data_dir)` — never anything outside it.
pub fn sweep_orphan_dirs(data_dir: &Path, known: &[AccountId]) {
    let root = accounts_root(data_dir);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        // No accounts root yet (no logins have ever run) — nothing to sweep.
        Err(_) => return,
    };
    // `account_config_dir` names each dir after `id.0.to_string()`, so a
    // plain string comparison against that same format is exact — no
    // UUID parsing needed.
    let known_names: Vec<String> = known.iter().map(|id| id.0.to_string()).collect();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let is_known = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| known_names.iter().any(|known_name| known_name == name));
        if is_known {
            continue;
        }
        delete_scoped_credentials(&path);
        if let Err(e) = std::fs::remove_dir_all(&path) {
            LogWriter::log(
                ErrorReport::new("Failed to remove orphaned account config dir")
                    .from_error(&e)
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("account.sweep.remove_dir_failed")
                    .build(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn scoped_service_name_is_stable_and_suffixed() {
        let a = scoped_keychain_service(Path::new("/tmp/acc/alice"));
        let a2 = scoped_keychain_service(Path::new("/tmp/acc/alice"));
        let b = scoped_keychain_service(Path::new("/tmp/acc/bob"));
        assert_eq!(a, a2, "same dir → same service name");
        assert_ne!(a, b, "different dir → different service name");
        assert!(a.starts_with("Claude Code-credentials-"));
        // sha256 앞 8 hex
        assert_eq!(a.rsplit('-').next().unwrap().len(), 8);
    }

    #[test]
    fn config_dir_is_under_data_dir_and_per_account() {
        let data = Path::new("/data");
        let id1 = daruda_store::accounts::AccountId::new();
        let id2 = daruda_store::accounts::AccountId::new();
        let d1 = account_config_dir(data, id1);
        let d2 = account_config_dir(data, id2);
        assert!(d1.starts_with("/data"));
        assert_ne!(d1, d2);
    }

    #[test]
    fn sweep_orphan_dirs_removes_unknown_and_garbage_keeps_known() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path();
        let root = accounts_root(data_dir);
        std::fs::create_dir_all(&root).expect("create accounts root");

        let known_id = daruda_store::accounts::AccountId::new();
        let known_dir = account_config_dir(data_dir, known_id);
        std::fs::create_dir_all(&known_dir).expect("create known dir");

        let unknown_id = daruda_store::accounts::AccountId::new();
        let unknown_dir = account_config_dir(data_dir, unknown_id);
        std::fs::create_dir_all(&unknown_dir).expect("create unknown dir");

        let garbage_dir = root.join("not-a-uuid");
        std::fs::create_dir_all(&garbage_dir).expect("create garbage dir");

        sweep_orphan_dirs(data_dir, &[known_id]);

        assert!(known_dir.exists(), "known account dir must be preserved");
        assert!(!unknown_dir.exists(), "unknown-uuid dir must be removed");
        assert!(!garbage_dir.exists(), "garbage-named dir must be removed");
    }

    #[test]
    fn sweep_orphan_dirs_no_op_when_root_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // accounts_root under tmp.path() was never created.
        sweep_orphan_dirs(tmp.path(), &[]);
    }
}
