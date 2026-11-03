//! Tests for the public `daruda_agent::store` API.

use chrono::{DateTime, Duration as CDuration, TimeZone, Utc};
use daruda_agent::SessionStatus;
use daruda_agent::hooks::status_file::StatusFile;
use daruda_agent::store::*;
use std::path::PathBuf;

/// Fixed reference time for tests so offsets are exact; `Utc::now()`
/// otherwise drifts microseconds between calls and breaks "same
/// timestamp" comparisons.
fn ref_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()
}

fn entry(session: &str, cwd: &str, status: SessionStatus, offset_secs: i64) -> StatusFile {
    StatusFile {
        schema_version: 1,
        session_id: session.into(),
        cwd: PathBuf::from(cwd),
        transcript_path: None,
        status,
        last_event: "test".into(),
        tool_name: None,
        tool_input: None,
        permission_mode: None,
        notification: None,
        timestamp: ref_time() + CDuration::seconds(offset_secs),
        terminal_id: None,
        harness: None,
        agent_pid: None,
    }
}

#[test]
fn upsert_inserts_and_replaces() {
    let mut s = ClaudeStatusStore::new();
    assert!(s.update(entry("a", "/x", SessionStatus::Working, 0)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Working);
    assert!(s.update(entry("a", "/x", SessionStatus::Idle, 1)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
}

#[test]
fn remove_returns_old() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Idle, 0));
    let old = s.remove("a").unwrap();
    assert_eq!(old.session_id, "a");
    assert!(s.get("a").is_none());
}

#[test]
fn aggregate_picks_highest_priority() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/wt", SessionStatus::Idle, 0));
    s.update(entry("b", "/wt", SessionStatus::Working, 0));
    s.update(entry("c", "/wt", SessionStatus::NeedsAttention, 0));
    s.update(entry("d", "/other", SessionStatus::Working, 0));

    assert_eq!(
        s.aggregate_for_cwd(&PathBuf::from("/wt")),
        Some(SessionStatus::NeedsAttention)
    );
    assert_eq!(
        s.aggregate_for_cwd(&PathBuf::from("/other")),
        Some(SessionStatus::Working)
    );
    assert!(s.aggregate_for_cwd(&PathBuf::from("/empty")).is_none());
}

#[test]
fn needs_attention_persists_regardless_of_age() {
    let mut store = ClaudeStatusStore::new();
    // A long-stale NeedsAttention must NOT decay — it stays until a
    // real clearing event (PreToolUse / Stop / SessionEnd) arrives.
    store.update(entry("a", "/wt", SessionStatus::NeedsAttention, 0));
    let path = PathBuf::from("/wt");
    assert_eq!(
        store.aggregate_for_cwd(&path),
        Some(SessionStatus::NeedsAttention)
    );
}

#[test]
fn needs_attention_still_wins_aggregate_when_old() {
    let mut store = ClaudeStatusStore::new();
    // An old NeedsAttention outranks a fresh Working — no age-based
    // demotion flips the winner anymore.
    store.update(entry("old", "/wt", SessionStatus::NeedsAttention, 0));
    store.update(entry("new", "/wt", SessionStatus::Working, 80));
    assert_eq!(
        store.aggregate_for_cwd(&PathBuf::from("/wt")),
        Some(SessionStatus::NeedsAttention)
    );
}

#[test]
fn load_initial_seeds_store() {
    let mut s = ClaudeStatusStore::new();
    s.load_initial(vec![
        entry("a", "/x", SessionStatus::Idle, 0),
        entry("b", "/x", SessionStatus::Working, 0),
    ]);
    assert_eq!(s.len(), 2);
}

#[test]
fn a_terminal_replaces_its_previous_session_and_rejects_stale_replays() {
    let mut store = ClaudeStatusStore::new();
    let mut first = entry("old", "/work", SessionStatus::Working, 0);
    first.terminal_id = Some(daruda_agent::hooks::route::TerminalId::parse("terminal-a").unwrap());
    let mut second = first.clone();
    second.session_id = "new".into();
    second.timestamp += CDuration::seconds(1);
    assert!(store.update(first.clone()));
    assert!(store.update(second));
    assert_eq!(store.len(), 1);
    assert!(store.get("old").is_none());
    assert!(!store.update(first));
    assert!(store.get("new").is_some());
}

#[test]
fn the_same_session_in_two_terminals_has_independent_snapshots() {
    let mut store = ClaudeStatusStore::new();
    let mut first = entry("shared", "/work", SessionStatus::Working, 10);
    first.terminal_id = daruda_agent::hooks::route::TerminalId::parse("terminal-a");
    let mut second = entry("shared", "/work", SessionStatus::Idle, 0);
    second.terminal_id = daruda_agent::hooks::route::TerminalId::parse("terminal-b");
    assert!(store.update(first));
    assert!(store.update(second));
    assert_eq!(store.len(), 2);
    assert_eq!(
        store.get_key("terminal-a").unwrap().status,
        SessionStatus::Working
    );
    assert_eq!(
        store.get_key("terminal-b").unwrap().status,
        SessionStatus::Idle
    );
    store.remove_key("terminal-b");
    assert_eq!(
        store.get_key("terminal-a").unwrap().status,
        SessionStatus::Working
    );
}
