//! Tests for the public `daruda_agent::skills::plugins` API.

use daruda_agent::skills::plugins::*;
use std::fs;

#[test]
fn parses_real_world_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("installed_plugins.json");
    fs::write(
        &manifest,
        r#"{
              "version": 2,
              "plugins": {
                "swift-lsp@claude-plugins-official": [{
                  "scope": "user",
                  "installPath": "/Users/x/.claude/plugins/cache/x/swift-lsp/1.0.0",
                  "version": "1.0.0",
                  "installedAt": "2026-02-06T09:30:57.914Z"
                }],
                "another@some-market": [{
                  "scope": "project",
                  "installPath": "/Users/x/.claude/plugins/cache/another/0.0.1",
                  "version": "0.0.1"
                }]
              }
            }"#,
    )
    .unwrap();
    let mut installs = read_installed_plugins(&manifest);
    installs.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(installs.len(), 2);
    assert_eq!(installs[0].id, "another@some-market");
    assert_eq!(installs[1].id, "swift-lsp@claude-plugins-official");
    assert_eq!(installs[1].scope, "user");
    assert_eq!(installs[1].version, "1.0.0");
}

#[test]
fn missing_file_returns_empty_vec() {
    let tmp = tempfile::tempdir().unwrap();
    let installs = read_installed_plugins(&tmp.path().join("does_not_exist.json"));
    assert!(installs.is_empty());
}

#[test]
fn malformed_json_is_swallowed() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("installed_plugins.json");
    fs::write(&manifest, "{ this isn't json").unwrap();
    assert!(read_installed_plugins(&manifest).is_empty());
}

#[test]
fn one_id_with_multiple_installs_yields_one_entry_each() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = tmp.path().join("installed_plugins.json");
    fs::write(
        &manifest,
        r#"{
              "version": 2,
              "plugins": {
                "shared@m": [
                  {"scope": "user", "installPath": "/a", "version": "1"},
                  {"scope": "project", "installPath": "/b", "version": "1"}
                ]
              }
            }"#,
    )
    .unwrap();
    let installs = read_installed_plugins(&manifest);
    assert_eq!(installs.len(), 2);
    assert!(installs.iter().all(|p| p.id == "shared@m"));
}
