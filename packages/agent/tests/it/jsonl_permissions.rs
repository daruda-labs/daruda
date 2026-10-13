//! Public API tests for jsonl permissions.

use daruda_agent::jsonl::permissions::*;

#[test]
fn always_allowed_read_only_tools() {
    let c = PermissionChecker::default();
    for t in ["Read", "Glob", "Grep", "WebFetch", "WebSearch"] {
        assert!(c.is_auto_approved(t, &serde_json::json!({})), "{t}");
    }
}

#[test]
fn unknown_tool_defaults_to_needs_permission() {
    let c = PermissionChecker::default();
    assert!(!c.is_auto_approved("MysteryTool", &serde_json::json!({})));
}

#[test]
fn from_settings_real_file_does_not_panic() {
    let _ = PermissionChecker::from_settings_file();
}
