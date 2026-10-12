//! Seed a managed account with canonical MCP servers once.
//! Existing account configuration is authoritative and never overwritten.

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

/// Seed an absent account MCP map from the canonical config before launch.
/// Existing maps (including empty maps) and malformed account files are
/// preserved. This compatibility name remains for the two existing callers.
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
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => return,
        },
        // File genuinely absent — start fresh (nothing to preserve).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        // The file exists but couldn't be read (transient IO/permission
        // error): do NOT write a fresh object back — that would silently
        // discard the account's `oauthAccount` (login identity). Skip the
        // mirror this time; a later spawn retries.
        Err(_) => return,
    };
    let Some(map) = account_value.as_object_mut() else {
        return;
    };
    if map.contains_key(MCP_SERVERS_KEY) {
        return;
    }
    map.insert(MCP_SERVERS_KEY.to_string(), mcp_servers);

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
    #[test]
    fn preparation_preserves_account_edits_empty_maps_and_invalid_files() {
        let home = tempfile::tempdir().unwrap();
        let canonical = home.path().join(CLAUDE_JSON_FILE);
        std::fs::write(
            &canonical,
            r#"{"mcpServers":{"shared":{"command":"shared"}}}"#,
        )
        .unwrap();
        let account = tempfile::tempdir().unwrap();
        let path = account.path().join(CLAUDE_JSON_FILE);
        for source in [
            r#"{"mcpServers":{"private":{"command":"private"}},"projects":{"/a":{}}}"#,
            r#"{"mcpServers":{},"oauthAccount":{"id":"account-a"}}"#,
            "{broken",
            "[]",
        ] {
            std::fs::write(&path, source).unwrap();
            mirror_into(&canonical, account.path());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        }
    }
}
