//! Shared-MCP mirroring: copy the user's canonical `~/.claude.json`
//! `mcpServers` into a managed account's isolated `<config_dir>/.claude.json`
//! so the account's adapter sees the same servers as the system default.
//! Pure/GPUI-free (filesystem + JSON merge, best-effort).

use std::path::Path;

use serde_json::{Map, Value};

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
