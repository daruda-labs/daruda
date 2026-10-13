//! Public API tests for hooks installer.

use daruda_agent::hooks::installer::*;
use serde_json::{Value, json};
use std::fs::{self};
use std::path::Path;
use tempfile::TempDir;

fn paths_in(dir: &TempDir) -> InstallerPaths {
    let home = dir.path();
    InstallerPaths {
        notify_script: home.join(".daruda").join("hooks").join("notify.sh"),
        claude_settings: home.join(".claude").join("settings.json"),
    }
}

fn read_settings(p: &Path) -> Value {
    let bytes = fs::read(p).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn install_writes_notify_script_executable() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    install(&paths).unwrap();

    let bytes = fs::read(&paths.notify_script).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("#!/usr/bin/env bash"));
    assert!(text.contains("--hook"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&paths.notify_script)
            .unwrap()
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "notify.sh should be executable");
    }
}

#[test]
fn notification_matcher_filters_subtypes() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    install(&paths).unwrap();
    let settings = read_settings(&paths.claude_settings);
    let m = settings["hooks"]["Notification"][0]["matcher"]
        .as_str()
        .unwrap();
    assert_eq!(m, "permission_prompt|elicitation_dialog");
    // Stop has no matcher (fires always).
    let m = settings["hooks"]["Stop"][0]["matcher"].as_str().unwrap();
    assert_eq!(m, "");
}

#[test]
fn install_is_idempotent() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    install(&paths).unwrap();
    install(&paths).unwrap();
    install(&paths).unwrap();

    let settings = read_settings(&paths.claude_settings);
    for event in SUBSCRIBED_EVENTS {
        let array = settings["hooks"][*event].as_array().unwrap();
        assert_eq!(array.len(), 1, "{event} got duplicated");
    }
}

#[test]
fn is_installed_negative_on_missing_or_malformed_file() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    assert!(!is_installed(&paths));

    fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
    fs::write(&paths.claude_settings, b"{ malformed").unwrap();
    assert!(!is_installed(&paths));
}

#[test]
fn install_refuses_when_event_value_is_not_array() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
    // User has hand-written hooks.PreToolUse as a *string* (wrong
    // schema). We must not silently overwrite the field.
    let pre = json!({
        "hooks": {
            "PreToolUse": "broken-string-value"
        }
    });
    fs::write(
        &paths.claude_settings,
        serde_json::to_vec_pretty(&pre).unwrap(),
    )
    .unwrap();

    let err = install(&paths).expect_err("install should refuse bad schema");
    match err {
        InstallerError::BadSchema {
            event,
            actual_kind: _,
        } => {
            assert_eq!(event, "PreToolUse");
        }
        other => panic!("expected BadSchema, got {other:?}"),
    }
    // Original file untouched.
    let after = read_settings(&paths.claude_settings);
    assert_eq!(after["hooks"]["PreToolUse"], "broken-string-value");
}

#[test]
fn install_refuses_when_top_level_is_not_object() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
    fs::write(&paths.claude_settings, b"[1, 2, 3]").unwrap();
    let err = install(&paths).expect_err("install should refuse non-object top-level");
    assert!(matches!(err, InstallerError::BadSchema { .. }));
}

#[test]
fn install_refuses_malformed_json() {
    let home = TempDir::new().unwrap();
    let paths = paths_in(&home);
    fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
    fs::write(&paths.claude_settings, b"{ malformed json").unwrap();
    let err = install(&paths).expect_err("install should refuse unparseable JSON");
    assert!(matches!(err, InstallerError::Json(_)));
}
