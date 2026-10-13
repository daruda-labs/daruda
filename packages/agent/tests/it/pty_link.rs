//! Tests for the public `daruda_agent::pty_link` API.

use daruda_agent::pty_link::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[test]
fn a_pid_holds_its_session_until_the_file_goes_or_names_another() {
    let dir = TempDir::new().unwrap();
    assert!(!pid_holds_session(dir.path(), 7, "a"));
    write_meta(
        dir.path(),
        7,
        json!({"pid": 7, "sessionId": "a", "cwd": "/w"}),
    );
    assert!(pid_holds_session(dir.path(), 7, "a"));
    assert!(!pid_holds_session(dir.path(), 7, "b"));
    std::fs::write(dir.path().join("7.json"), "{half").unwrap();
    assert!(pid_holds_session(dir.path(), 7, "b"));
}

fn write_meta(dir: &Path, pid: u32, json_obj: serde_json::Value) {
    let path = dir.join(format!("{pid}.json"));
    std::fs::write(&path, json_obj.to_string()).unwrap();
}

#[test]
fn parses_minimal_three_field_meta() {
    let dir = TempDir::new().unwrap();
    write_meta(
        dir.path(),
        1234,
        json!({
            "pid": 1234,
            "sessionId": "abc-123",
            "cwd": "/tmp/proj"
        }),
    );

    let meta = read_session_meta_in(dir.path(), 1234).unwrap();
    assert_eq!(meta.pid, 1234);
    assert_eq!(meta.session_id, "abc-123");
    assert_eq!(meta.cwd, PathBuf::from("/tmp/proj"));
}

#[test]
fn parses_full_real_world_meta() {
    // Extra keys present in real session files; daruda only consumes 3.
    let dir = TempDir::new().unwrap();
    write_meta(
        dir.path(),
        31134,
        json!({
            "pid": 31134,
            "sessionId": "fd56d7e4-53b4-46d6-b065-03ae50709aab",
            "cwd": "/home/user/project",
            "startedAt": 1777600781920_i64,
            "procStart": "Fri May  1 01:59:40 2026",
            "version": "2.1.123",
            "peerProtocol": 1,
            "kind": "interactive",
            "entrypoint": "cli",
            "status": "active",
            "updatedAt": 1777600782000_i64,
            "waitingFor": null,
        }),
    );

    let meta = read_session_meta_in(dir.path(), 31134).unwrap();
    assert_eq!(meta.session_id, "fd56d7e4-53b4-46d6-b065-03ae50709aab");
}

#[test]
fn missing_file_yields_none() {
    let dir = TempDir::new().unwrap();
    assert!(read_session_meta_in(dir.path(), 99999).is_none());
}

#[test]
fn malformed_json_yields_none() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("777.json"), b"{ not valid").unwrap();
    assert!(read_session_meta_in(dir.path(), 777).is_none());
}

#[test]
fn missing_required_field_yields_none() {
    let dir = TempDir::new().unwrap();
    // No `sessionId` — required by our subset.
    write_meta(
        dir.path(),
        42,
        json!({
            "pid": 42,
            "cwd": "/tmp"
        }),
    );
    assert!(read_session_meta_in(dir.path(), 42).is_none());
}

#[test]
fn default_dir_resolves_under_home() {
    // We don't assert the exact home, only the suffix shape.
    let dir = default_sessions_dir().expect("home dir should resolve");
    let s = dir.to_string_lossy();
    assert!(s.ends_with(".claude/sessions") || s.ends_with(".claude\\sessions"));
}
