//! Per-account runtime isolation for Claude: config-dir layout and
//! config-dir-scoped Keychain reads. Pure/GPUI-free.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::http::FetchError;
use crate::limits::{PlanInfo, parse_keychain_credentials};
use daruda_store::accounts::AccountId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::persistence::save_json_atomic;

/// Filename of the Claude Code config file that holds `mcpServers`
/// (User/Local scopes) and, for a managed account's isolated
/// `CLAUDE_CONFIG_DIR`, that account's `oauthAccount`. Mirrors
/// `app::agent::mcp::parse::CLAUDE_JSON_FILE` — kept as a local
/// constant so this GPUI-free crate doesn't depend on `app`.
const CLAUDE_JSON_FILE: &str = ".claude.json";

/// Top-level key shared across every account's config file.
const MCP_SERVERS_KEY: &str = "mcpServers";

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

/// Mirror the shared `mcpServers` config from the user's canonical
/// `~/.claude.json` (the file `app::agent::mcp::parse::claude_json_path`
/// resolves, and what the MCP tab edits) into a managed account's
/// isolated `<config_dir>/.claude.json`, so the account's ACP adapter
/// process — which only reads MCP servers from *its own*
/// `CLAUDE_CONFIG_DIR` — sees the same servers as the system default.
///
/// MCP is shared across accounts by decision (Plan B): there is one
/// `mcpServers` set, not one per account. Every other key in the
/// account file — most importantly `oauthAccount`, written there by
/// that account's own login — is left untouched.
///
/// Best-effort: a missing/unreadable/unparseable canonical file is a
/// no-op (never clobbers the account file just because the canonical
/// read failed), `dirs::home_dir()` returning `None` is a no-op, and any
/// write failure is logged, never panics. Call this on a background
/// thread before spawning a managed-account agent-chat adapter — the
/// canonical file can be multi-megabyte.
///
/// v1 scope: agent-chat only (the actual MCP consumer). Terminal panes
/// launched under a managed account do not get this mirror yet — a
/// deferred extension, not a correctness gap for the current use.
pub fn mirror_shared_mcp_servers(config_dir: &Path) {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    mirror_into(&home.join(CLAUDE_JSON_FILE), config_dir);
}

/// Seam for [`mirror_shared_mcp_servers`]: takes the canonical file's
/// path directly so tests don't depend on the real `$HOME`.
fn mirror_into(canonical_path: &Path, config_dir: &Path) {
    let canonical_raw = match std::fs::read_to_string(canonical_path) {
        Ok(raw) => raw,
        // Missing/unreadable canonical file — nothing to mirror, and
        // clobbering the account file's `mcpServers` on a read failure
        // would be worse than leaving it as-is.
        Err(_) => return,
    };
    let canonical_value: Value = match serde_json::from_str(&canonical_raw) {
        Ok(value) => value,
        Err(_) => return,
    };
    let mcp_servers = canonical_value
        .get(MCP_SERVERS_KEY)
        .cloned()
        // Absent key = shared state is "no servers", not "leave
        // whatever the account file already has".
        .unwrap_or_else(|| Value::Object(Map::new()));

    let account_path = config_dir.join(CLAUDE_JSON_FILE);
    let mut account_value: Value = match std::fs::read_to_string(&account_path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Map::new())),
        // File genuinely absent — start fresh (nothing to preserve).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        // The file exists but couldn't be read (transient IO/permission
        // error): do NOT write a fresh object back — that would silently
        // discard the account's `oauthAccount` (login identity). Skip the
        // mirror this time; a later spawn retries.
        Err(_) => return,
    };
    match account_value.as_object_mut() {
        Some(map) => {
            map.insert(MCP_SERVERS_KEY.to_string(), mcp_servers);
        }
        // The existing file's top level isn't an object (corrupt) —
        // replace it wholesale with a fresh object holding only the
        // mirrored servers, rather than silently dropping the mirror.
        None => {
            let mut map = Map::new();
            map.insert(MCP_SERVERS_KEY.to_string(), mcp_servers);
            account_value = Value::Object(map);
        }
    }

    if let Err(e) = save_json_atomic(config_dir, &account_path, &account_value) {
        LogWriter::log(
            ErrorReport::new("Failed to mirror shared MCP servers into account config")
                .from_error(&e)
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("account.mcp.mirror_write_failed")
                .build(),
        );
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

    #[test]
    fn mirror_into_preserves_oauth_account_and_copies_mcp_servers() {
        let home = tempfile::tempdir().expect("home tempdir");
        let canonical_path = home.path().join(CLAUDE_JSON_FILE);
        std::fs::write(
            &canonical_path,
            serde_json::json!({
                "mcpServers": {
                    "shared-server": {
                        "command": "npx",
                        "args": ["shared-mcp"],
                    }
                },
                "someOtherUserKey": "ignored",
            })
            .to_string(),
        )
        .expect("write canonical .claude.json");

        let config_dir = tempfile::tempdir().expect("config_dir tempdir");
        let account_path = config_dir.path().join(CLAUDE_JSON_FILE);
        std::fs::write(
            &account_path,
            serde_json::json!({
                "oauthAccount": {
                    "emailAddress": "managed@example.com",
                },
            })
            .to_string(),
        )
        .expect("write account .claude.json");

        mirror_into(&canonical_path, config_dir.path());

        let merged: Value = serde_json::from_str(
            &std::fs::read_to_string(&account_path).expect("read merged account file"),
        )
        .expect("parse merged account file");
        assert_eq!(
            merged["mcpServers"]["shared-server"]["command"],
            Value::String("npx".to_string()),
            "mirrored mcpServers must land in the account file"
        );
        assert_eq!(
            merged["oauthAccount"]["emailAddress"],
            Value::String("managed@example.com".to_string()),
            "pre-existing oauthAccount must be preserved"
        );
    }

    #[test]
    fn mirror_into_no_op_when_canonical_missing() {
        let home = tempfile::tempdir().expect("home tempdir");
        let canonical_path = home.path().join(CLAUDE_JSON_FILE);
        // Canonical file was never created.

        let config_dir = tempfile::tempdir().expect("config_dir tempdir");
        let account_path = config_dir.path().join(CLAUDE_JSON_FILE);
        std::fs::write(
            &account_path,
            serde_json::json!({"oauthAccount": {"emailAddress": "managed@example.com"}})
                .to_string(),
        )
        .expect("write account .claude.json");

        mirror_into(&canonical_path, config_dir.path());

        let untouched: Value = serde_json::from_str(
            &std::fs::read_to_string(&account_path).expect("read account file"),
        )
        .expect("parse account file");
        assert_eq!(
            untouched["oauthAccount"]["emailAddress"],
            Value::String("managed@example.com".to_string()),
            "account file must be left untouched when the canonical read fails"
        );
        assert!(
            untouched.get(MCP_SERVERS_KEY).is_none(),
            "no mcpServers key should be introduced on a canonical-missing no-op"
        );
    }
}
