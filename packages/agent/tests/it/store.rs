//! Tests for the public `daruda_agent::store` API.

use chrono::{DateTime, Duration as CDuration, TimeZone, Utc};
use daruda_agent::SessionStatus;
use daruda_agent::hooks::status_file::{Source, StatusFile};
use daruda_agent::store::*;
use std::path::PathBuf;

/// Fixed reference time for tests so offsets are exact; `Utc::now()`
/// otherwise drifts microseconds between calls and breaks "same
/// timestamp" comparisons.
fn ref_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()
}

fn entry(
    session: &str,
    cwd: &str,
    status: SessionStatus,
    source: Source,
    offset_secs: i64,
) -> StatusFile {
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
        source,
    }
}

#[test]
fn upsert_inserts_and_replaces() {
    let mut s = ClaudeStatusStore::new();
    assert!(s.update(entry("a", "/x", SessionStatus::Working, Source::Hook, 0)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Working);
    assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 1)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
}

#[test]
fn remove_returns_old() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 0));
    let old = s.remove("a").unwrap();
    assert_eq!(old.session_id, "a");
    assert!(s.get("a").is_none());
}

#[test]
fn hook_always_wins_over_existing_jsonl() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 0));
    // Hook arriving even with same timestamp wins.
    assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 0)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    assert_eq!(s.get("a").unwrap().source, Source::Hook);
}

#[test]
fn jsonl_loses_to_existing_hook_at_same_or_older_time() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 10));
    // Older jsonl: rejected.
    assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 5)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    // Same-timestamp jsonl: rejected (hook keeps slot).
    assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 10)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
}

#[test]
fn jsonl_strictly_newer_overrides_hook() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Working, Source::Hook, 0));
    // jsonl 30s newer → wins (e.g. user uninstalled hooks; jsonl
    // catches up).
    assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Jsonl, 30)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    assert_eq!(s.get("a").unwrap().source, Source::Jsonl);
}

#[test]
fn jsonl_vs_jsonl_takes_newest() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 0));
    // Newer wins.
    assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Jsonl, 1)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    // Older rejected.
    assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, -1)));
    assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
}

#[test]
fn aggregate_picks_highest_priority() {
    let mut s = ClaudeStatusStore::new();
    s.update(entry("a", "/wt", SessionStatus::Idle, Source::Hook, 0));
    s.update(entry("b", "/wt", SessionStatus::Working, Source::Hook, 0));
    s.update(entry(
        "c",
        "/wt",
        SessionStatus::NeedsAttention,
        Source::Hook,
        0,
    ));
    s.update(entry(
        "d",
        "/other",
        SessionStatus::Working,
        Source::Hook,
        0,
    ));

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
    store.update(entry(
        "a",
        "/wt",
        SessionStatus::NeedsAttention,
        Source::Hook,
        0,
    ));
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
    store.update(entry(
        "old",
        "/wt",
        SessionStatus::NeedsAttention,
        Source::Hook,
        0,
    ));
    store.update(entry(
        "new",
        "/wt",
        SessionStatus::Working,
        Source::Hook,
        80,
    ));
    assert_eq!(
        store.aggregate_for_cwd(&PathBuf::from("/wt")),
        Some(SessionStatus::NeedsAttention)
    );
}

#[test]
fn load_initial_seeds_store() {
    let mut s = ClaudeStatusStore::new();
    s.load_initial(vec![
        entry("a", "/x", SessionStatus::Idle, Source::Hook, 0),
        entry("b", "/x", SessionStatus::Working, Source::Hook, 0),
    ]);
    assert_eq!(s.len(), 2);
}
