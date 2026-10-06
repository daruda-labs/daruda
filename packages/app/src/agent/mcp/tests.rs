//! Integration tests across the parser + persist layers.
//!
//! Per-module tests live next to their owners (`parse::parse_tests`,
//! `persist::persist_tests`). This file covers behaviour that spans
//! both — the lossless round-trip contract that holds the whole tab
//! together.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::persist::{
    McpServerDraft, delete_server, set_disabled, update_server, write_atomic, write_server,
};
use super::{
    McpLocation, McpScope, McpServer, McpTransport, NameError, format_env_lines, parse,
    parse_env_lines, validate_command, validate_name, validate_url,
};

/// Read a top-level (`mcpServers`) document into `(servers, raw)` —
/// stands in for the removed `parse_personal_settings` in tests that
/// only exercise the top-level layout shared by `.mcp.json` and the
/// User scope.
fn parse_top(path: &std::path::Path) -> (Vec<McpServer>, Value) {
    let raw = parse::read_json_or_empty(path).unwrap();
    let servers = parse::extract_servers_at(&raw, &McpLocation::TopLevel, McpScope::User);
    (servers, raw)
}

#[test]
fn lossless_round_trip_preserves_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");

    let original = json!({
        "permissions": {
            "allow": ["Read", "Write(**/*.md)"],
            "deny":  []
        },
        "enableAllProjectMcpServers": true,
        "hooks": {
            "Stop": [{ "matcher": ".*", "hooks": [] }]
        },
        "mcpServers": {
            "alpha": {
                "type": "stdio",
                "command": "node",
                "args": ["main.js"],
                "env": { "FOO": "bar" },
                "experimentalFlag": true,
                "policy": { "k": [1, 2, 3] }
            }
        }
    });
    write_atomic(&path, &original).unwrap();

    let (servers, raw) = parse_top(&path);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "alpha");
    assert_eq!(servers[0].transport, McpTransport::Stdio);
    assert!(servers[0].extra.contains_key("experimentalFlag"));

    let mut raw = raw;
    set_disabled(
        &mut raw,
        &path,
        McpScope::User,
        &McpLocation::TopLevel,
        "alpha",
        true,
    )
    .unwrap();

    // Re-read; every sibling key must still be there byte-for-byte.
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(on_disk["permissions"], original["permissions"]);
    assert_eq!(on_disk["enableAllProjectMcpServers"], Value::Bool(true));
    assert_eq!(on_disk["hooks"], original["hooks"]);
    assert_eq!(
        on_disk["mcpServers"]["alpha"]["experimentalFlag"],
        Value::Bool(true)
    );
    assert_eq!(
        on_disk["mcpServers"]["alpha"]["policy"]["k"],
        json!([1, 2, 3])
    );
}

#[test]
fn write_server_appends_without_disturbing_existing_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".mcp.json");
    let mut raw = json!({
        "mcpServers": {
            "alpha": { "type": "stdio", "command": "node" }
        }
    });
    write_atomic(&path, &raw).unwrap();

    let draft = McpServerDraft {
        name: "beta".into(),
        transport: McpTransport::Sse,
        command: None,
        args: vec![],
        url: Some("https://example.com/mcp".into()),
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        disabled: false,
        extra: BTreeMap::new(),
    };
    write_server(
        &mut raw,
        &path,
        McpScope::Project,
        &McpLocation::TopLevel,
        &draft,
    )
    .unwrap();

    let (servers, _) = parse::parse_project_mcp(&path).unwrap();
    assert_eq!(servers.len(), 2);
    let beta = servers.iter().find(|s| s.name == "beta").unwrap();
    assert_eq!(beta.transport, McpTransport::Sse);
    assert_eq!(beta.url.as_deref(), Some("https://example.com/mcp"));

    let alpha = servers.iter().find(|s| s.name == "alpha").unwrap();
    assert_eq!(alpha.transport, McpTransport::Stdio);
}

#[test]
fn update_server_rewrites_target_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut raw = json!({
        "mcpServers": {
            "alpha": { "type": "stdio", "command": "node", "args": ["a.js"] },
            "beta":  { "type": "stdio", "command": "node", "args": ["b.js"] }
        }
    });
    write_atomic(&path, &raw).unwrap();

    let draft = McpServerDraft {
        name: "alpha".into(),
        transport: McpTransport::Stdio,
        command: Some("deno".into()),
        args: vec!["task".into(), "start".into()],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        disabled: false,
        extra: BTreeMap::new(),
    };
    update_server(
        &mut raw,
        &path,
        McpScope::User,
        &McpLocation::TopLevel,
        &draft,
    )
    .unwrap();

    let (servers, _) = parse_top(&path);
    let alpha = servers.iter().find(|s| s.name == "alpha").unwrap();
    assert_eq!(alpha.command.as_deref(), Some("deno"));
    assert_eq!(alpha.args, vec!["task", "start"]);

    let beta = servers.iter().find(|s| s.name == "beta").unwrap();
    assert_eq!(beta.command.as_deref(), Some("node"));
    assert_eq!(beta.args, vec!["b.js"]);
}

#[test]
fn headers_on_stdio_round_trip_through_extra() {
    // A user (or another tool) may write a `headers` entry on a stdio
    // server — daruda must round-trip it verbatim without dropping it
    // on Edit.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut raw = json!({
        "mcpServers": {
            "alpha": {
                "type": "stdio",
                "command": "node",
                "headers": { "X-Stash": "value" }
            }
        }
    });
    write_atomic(&path, &raw).unwrap();

    let (servers, _) = parse_top(&path);
    let alpha = &servers[0];
    assert_eq!(alpha.transport, McpTransport::Stdio);
    // headers is empty on the typed field for Stdio; the original
    // value is held in `extra` until rewrite.
    assert!(alpha.headers.is_empty());
    assert!(alpha.extra.contains_key("headers"));

    // Edit (transport unchanged) must leave the original headers
    // intact.
    let draft = McpServerDraft::from_server(alpha);
    update_server(
        &mut raw,
        &path,
        McpScope::User,
        &McpLocation::TopLevel,
        &draft,
    )
    .unwrap();
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        on_disk["mcpServers"]["alpha"]["headers"]["X-Stash"],
        Value::String("value".into())
    );
}

#[test]
fn delete_then_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let mut raw = json!({
        "permissions": { "allow": ["Read"] },
        "mcpServers": {
            "alpha": { "type": "stdio", "command": "node" },
            "beta":  { "type": "stdio", "command": "node" }
        }
    });
    write_atomic(&path, &raw).unwrap();

    delete_server(
        &mut raw,
        &path,
        McpScope::User,
        &McpLocation::TopLevel,
        "alpha",
    )
    .unwrap();

    let (servers, on_disk) = parse_top(&path);
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "beta");
    assert!(on_disk.get("permissions").is_some());
}

#[test]
fn reload_claude_json_hash_gate_skips_reparse_on_unchanged_content() {
    use super::McpState;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".claude.json");
    let lane = std::path::PathBuf::from("/repo/a");
    std::fs::write(
        &path,
        br#"{"mcpServers":{"u1":{"command":"u"}},"projects":{"/repo/a":{"mcpServers":{"l1":{"command":"l"}}}}}"#,
    )
    .unwrap();

    let mut state = McpState::default();
    // First load parses, populates both scopes, and records the hash.
    assert!(state.reload_claude_json_at(&path, Some(&lane)).unwrap());
    assert_eq!(state.user.len(), 1);
    assert_eq!(state.local.get(&lane).unwrap().len(), 1);

    // Corrupt the in-memory User vec. An identical-content reload must
    // hash-hit and skip the re-parse, leaving the corrupted vec intact —
    // proof the expensive parse was avoided.
    state.user.clear();
    let changed = state.reload_claude_json_at(&path, Some(&lane)).unwrap();
    assert!(!changed, "unchanged content must report no change");
    assert!(
        state.user.is_empty(),
        "hash hit must skip the User re-parse"
    );

    // Change the file: hash differs → parse runs → User recomputed and
    // the lane's Local emptied (no longer present under `projects`).
    std::fs::write(
        &path,
        br#"{"mcpServers":{"u1":{"command":"u"},"u2":{"command":"u2"}}}"#,
    )
    .unwrap();
    assert!(state.reload_claude_json_at(&path, Some(&lane)).unwrap());
    assert_eq!(state.user.len(), 2);
    assert!(
        state.local.get(&lane).unwrap().is_empty(),
        "lane dropped from projects → empty Local"
    );
}

#[test]
fn snapshot_merges_project_lane_and_cwd_with_lane_winning() {
    use super::McpState;
    let dir = tempfile::tempdir().unwrap();
    let lane = dir.path().join("lane");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&lane).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        lane.join(".mcp.json"),
        br#"{"mcpServers":{"shared":{"command":"LANE"},"lane_only":{"command":"l"}}}"#,
    )
    .unwrap();
    std::fs::write(
        cwd.join(".mcp.json"),
        br#"{"mcpServers":{"shared":{"command":"CWD"},"cwd_only":{"command":"c"}}}"#,
    )
    .unwrap();

    let mut state = McpState::default();
    state.reload_project(Some(&lane)).unwrap();
    state.reload_project(Some(&cwd)).unwrap();

    // Merge of both dirs, alphabetically sorted. `lane` is listed
    // first so it wins same-name collisions.
    let snap = state.snapshot_for(Some(&lane), &[lane.clone(), cwd.clone()]);
    let names: Vec<&str> = snap.project.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["cwd_only", "lane_only", "shared"]);
    // Same-name collision → the nearest (lane) entry wins (it is the
    // write target).
    let shared = snap.project.iter().find(|s| s.name == "shared").unwrap();
    assert_eq!(shared.command.as_deref(), Some("LANE"));
    // Write target stays the lane root regardless of the merge.
    assert_eq!(snap.project_mcp_path, Some(lane.join(".mcp.json")));

    // A repeated dir must not double-count.
    let snap_same = state.snapshot_for(Some(&lane), &[lane.clone(), lane.clone()]);
    assert_eq!(snap_same.project.len(), 2);

    // lane-only dir list (backward behaviour).
    let snap_lane_only = state.snapshot_for(Some(&lane), std::slice::from_ref(&lane));
    let lane_names: Vec<&str> = snap_lane_only
        .project
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(lane_names, vec!["lane_only", "shared"]);
}

#[test]
fn validate_name_matrix() {
    assert_eq!(validate_name(""), Err(NameError::Empty));
    assert!(validate_name("filesystem").is_ok());
    assert!(validate_name("Github_v2").is_ok());
    assert!(validate_name("a-b-c").is_ok());
    assert!(matches!(
        validate_name("-leading"),
        Err(NameError::InvalidLeading { .. })
    ));
    assert!(matches!(
        validate_name("space here"),
        Err(NameError::InvalidChar { .. })
    ));
    let too_long = "x".repeat(super::MAX_NAME_LEN + 1);
    assert!(matches!(
        validate_name(&too_long),
        Err(NameError::TooLong { .. })
    ));
}

#[test]
fn validate_url_scheme_required_for_remote_only() {
    assert!(validate_url("", McpTransport::Stdio).is_ok());
    assert!(validate_url("", McpTransport::Http).is_err());
    assert!(validate_url("ftp://example.com", McpTransport::Http).is_err());
    assert!(validate_url("https://example.com", McpTransport::Sse).is_ok());
    assert!(validate_url("http://localhost:8080", McpTransport::Http).is_ok());
}

#[test]
fn validate_command_required_for_stdio_only() {
    assert!(validate_command("", McpTransport::Stdio).is_err());
    assert!(validate_command("", McpTransport::Http).is_ok());
    assert!(validate_command("npx", McpTransport::Stdio).is_ok());
}

#[test]
fn parse_env_lines_skips_comments_and_blank_lines() {
    let text = "# this is a comment\n\nFOO=bar\n  TOKEN=secret\n";
    let env = parse_env_lines(text).unwrap();
    assert_eq!(env.get("FOO").unwrap(), "bar");
    assert_eq!(env.get("TOKEN").unwrap(), "secret");
    assert_eq!(env.len(), 2);
}

#[test]
fn parse_env_lines_trims_value_padding() {
    let env = parse_env_lines("KEY=  padded  ").unwrap();
    assert_eq!(env.get("KEY").unwrap(), "padded");
}

#[test]
fn parse_env_lines_preserves_embedded_whitespace() {
    let env = parse_env_lines("MSG=hello world\n").unwrap();
    assert_eq!(env.get("MSG").unwrap(), "hello world");
}

#[test]
fn parse_env_lines_accepts_empty_value() {
    let env = parse_env_lines("UNSET=").unwrap();
    assert_eq!(env.get("UNSET").unwrap(), "");
    assert_eq!(env.len(), 1);
}

#[test]
fn parse_env_lines_rejects_bad_form() {
    assert!(parse_env_lines("no_equals_here").is_err());
    assert!(parse_env_lines("=novalue").is_err());
}

#[test]
fn format_env_lines_round_trip_sorted() {
    let env: BTreeMap<String, String> = [
        ("ZED".to_string(), "1".to_string()),
        ("ALPHA".to_string(), "2".to_string()),
    ]
    .into_iter()
    .collect();
    let text = format_env_lines(&env);
    // BTreeMap iteration is sorted — ALPHA precedes ZED.
    assert!(text.starts_with("ALPHA="));
}

#[test]
fn server_command_preview_truncates_with_ellipsis() {
    let s = McpServer {
        name: "n".into(),
        scope: McpScope::User,
        transport: McpTransport::Stdio,
        command: Some("a".repeat(100)),
        args: vec![],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        disabled: false,
        extra: BTreeMap::new(),
    };
    let preview = s.command_preview();
    let count = preview.chars().count();
    assert_eq!(count, super::PREVIEW_MAX_CHARS);
    assert!(preview.ends_with('…'));
}

#[test]
fn malformed_detection() {
    let stdio_no_cmd = McpServer {
        name: "x".into(),
        scope: McpScope::User,
        transport: McpTransport::Stdio,
        command: None,
        args: vec![],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        disabled: false,
        extra: BTreeMap::new(),
    };
    assert!(stdio_no_cmd.is_malformed());

    let http_no_url = McpServer {
        name: "y".into(),
        scope: McpScope::User,
        transport: McpTransport::Http,
        command: None,
        args: vec![],
        url: None,
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        disabled: false,
        extra: BTreeMap::new(),
    };
    assert!(http_no_url.is_malformed());
}
