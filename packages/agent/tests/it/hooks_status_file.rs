//! Tests for the public `daruda_agent::hooks::status_file` API.

use chrono::Utc;
use daruda_agent::SessionStatus;
use daruda_agent::hooks::events::NotificationType;
use daruda_agent::hooks::status_file::*;
use std::fs;
use tempfile::TempDir;

fn sample() -> StatusFile {
    StatusFile::new_hook("sess-1", "/tmp/cwd", SessionStatus::Working, "PreToolUse")
}

#[test]
fn event_within_max_age_is_not_expired() {
    let mut f = sample();
    f.timestamp = Utc::now() - chrono::Duration::seconds(60);
    assert!(!f.event_expired(Utc::now(), std::time::Duration::from_secs(300)));
}

#[test]
fn event_past_max_age_is_expired() {
    let mut f = sample();
    f.timestamp = Utc::now() - chrono::Duration::seconds(600);
    assert!(f.event_expired(Utc::now(), std::time::Duration::from_secs(300)));
}

/// Boundary matches `cold_restore::classify`: exactly at the
/// threshold counts as expired.
#[test]
fn event_exactly_at_max_age_is_expired() {
    let now = Utc::now();
    let mut f = sample();
    f.timestamp = now - chrono::Duration::seconds(300);
    assert!(f.event_expired(now, std::time::Duration::from_secs(300)));
}

/// A clock-skewed future timestamp must not underflow — it is
/// simply "not expired".
#[test]
fn event_in_future_is_not_expired() {
    let now = Utc::now();
    let mut f = sample();
    f.timestamp = now + chrono::Duration::seconds(60);
    assert!(!f.event_expired(now, std::time::Duration::from_secs(300)));
}

#[test]
fn roundtrip_write_then_read() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "sess-1");
    write_atomic(&p, &sample()).unwrap();
    let back = read(&p).unwrap().unwrap();
    assert_eq!(back.session_id, "sess-1");
    assert_eq!(back.status, SessionStatus::Working);
    assert_eq!(back.last_event, "PreToolUse");
    assert_eq!(back.source, Source::Hook);
    assert_eq!(back.schema_version, SCHEMA_VERSION);
}

#[test]
fn read_missing_yields_none() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "absent");
    assert!(read(&p).unwrap().is_none());
}

#[test]
fn notification_subtype_round_trips() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "notif");
    let mut file = sample();
    file.notification = Some(NotificationType::PermissionPrompt);
    write_atomic(&p, &file).unwrap();
    let back = read(&p).unwrap().unwrap();
    assert_eq!(back.notification, Some(NotificationType::PermissionPrompt));
}

#[test]
fn notification_defaults_to_none_when_absent() {
    // Old status files (written before the field existed) must read
    // back with `notification: None` via serde default.
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "legacy");
    let raw = r#"{"schema_version":1,"session_id":"legacy","cwd":"/x","state":"Idle","last_event":"Stop","timestamp":"2026-05-01T12:00:00Z","source":"hook"}"#;
    fs::write(&p, raw).unwrap();
    let back = read(&p).unwrap().unwrap();
    assert_eq!(back.notification, None);
}

#[test]
fn lock_path_is_sibling_with_lock_extension() {
    let dir = TempDir::new().unwrap();
    let json = path_for(dir.path(), "sess-1");
    let lock = lock_path_for(dir.path(), "sess-1");
    assert_eq!(lock.parent(), json.parent());
    assert_eq!(lock.extension().unwrap(), "lock");
    assert_eq!(lock.file_stem().unwrap(), "sess-1");
}

#[test]
fn read_malformed_yields_none() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "broken");
    fs::write(&p, b"{ not valid json").unwrap();
    assert!(read(&p).unwrap().is_none());
}

#[test]
fn write_creates_parent_dir() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a").join("b").join("c");
    let p = path_for(&nested, "sess-1");
    write_atomic(&p, &sample()).unwrap();
    assert!(p.exists());
}

#[test]
fn delete_idempotent() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "x");
    // Missing file: ok.
    delete(&p).unwrap();
    // Present file: also ok.
    write_atomic(&p, &sample()).unwrap();
    delete(&p).unwrap();
    assert!(!p.exists());
    // Calling again: still ok.
    delete(&p).unwrap();
}

#[test]
fn list_dir_filters_to_json() {
    let dir = TempDir::new().unwrap();
    write_atomic(&path_for(dir.path(), "a"), &sample()).unwrap();
    write_atomic(&path_for(dir.path(), "b"), &sample()).unwrap();
    fs::write(dir.path().join("readme.txt"), "noise").unwrap();
    let entries = list_dir(dir.path()).unwrap();
    assert_eq!(entries.len(), 2);
    for e in &entries {
        assert!(e.path.extension().unwrap() == "json");
    }
}

#[test]
fn list_dir_missing_dir_yields_empty() {
    let dir = TempDir::new().unwrap();
    let nope = dir.path().join("does-not-exist");
    let entries = list_dir(&nope).unwrap();
    assert!(entries.is_empty());
}

#[test]
fn forward_compat_extra_keys_ignored_on_read() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "future");
    // Hand-rolled JSON with an extra field a future daruda version
    // might add. Read should still succeed.
    let raw = r#"{"schema_version":1,"session_id":"future","cwd":"/x","state":"Idle","last_event":"Stop","timestamp":"2026-05-01T12:00:00Z","source":"hook","new_field_for_phase_f":42}"#.to_string();
    fs::write(&p, raw).unwrap();
    let back = read(&p).unwrap().unwrap();
    assert_eq!(back.session_id, "future");
    assert_eq!(back.status, SessionStatus::Idle);
}

#[test]
fn atomic_rename_replaces_existing_file() {
    let dir = TempDir::new().unwrap();
    let p = path_for(dir.path(), "sess");
    let mut s = sample();
    s.status = SessionStatus::Working;
    write_atomic(&p, &s).unwrap();
    s.status = SessionStatus::Idle;
    s.last_event = "Stop".into();
    write_atomic(&p, &s).unwrap();
    let back = read(&p).unwrap().unwrap();
    assert_eq!(back.status, SessionStatus::Idle);
    assert_eq!(back.last_event, "Stop");
}
