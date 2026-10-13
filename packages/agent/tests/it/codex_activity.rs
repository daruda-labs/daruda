//! Public API tests for codex activity.

use daruda_agent::codex_activity::*;
use serde_json::Value;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

const D1: &str = "2026-07-20T00:18:16.344Z";

const D2: &str = "2026-07-21T00:18:16.344Z";

fn expected_date(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .unwrap()
        .with_timezone(&chrono::Local)
        .date_naive()
        .to_string()
}

fn task_started_line(ts: &str) -> String {
    json!({"timestamp": ts, "type": "event_msg",
           "payload": {"type": "task_started", "turn_id": "t1"}})
    .to_string()
        + "\n"
}

fn token_count_line(ts: &str, delta: u64) -> String {
    json!({"timestamp": ts, "type": "event_msg",
           "payload": {"type": "token_count",
                       "info": {"last_token_usage": {"total_tokens": delta},
                                "total_token_usage": {"total_tokens": delta}}}})
    .to_string()
        + "\n"
}

fn other_line(ts: &str) -> String {
    json!({"timestamp": ts, "type": "response_item", "payload": {}}).to_string() + "\n"
}

fn session_meta_line(id: &str, cwd: &str) -> String {
    json!({"type": "session_meta", "payload": {"session_id": id, "cwd": cwd}}).to_string() + "\n"
}

fn legacy_id_session_meta_line(id: &str, cwd: &str) -> String {
    json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd}}).to_string() + "\n"
}

fn session_meta_line_with_branch(id: &str, cwd: &str, branch: &str) -> String {
    json!({"type": "session_meta", "payload": {
        "session_id": id,
        "cwd": cwd,
        "git": {"branch": branch}
    }})
    .to_string()
        + "\n"
}

fn user_message_line(ts: &str, message: &str) -> String {
    json!({"timestamp": ts, "type": "event_msg",
           "payload": {"type": "user_message", "message": message}})
    .to_string()
        + "\n"
}

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let sessions = tmp.path().join("sessions");
    let cache = tmp.path().join("cache").join("codex-activity.json");
    (tmp, sessions, cache)
}

fn write_log(root: &Path, y: &str, m: &str, d: &str, name: &str, content: &str) -> PathBuf {
    let dir = root.join(y).join(m).join(d);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn counts_task_started_as_turns_and_ignores_other_records() {
    let (_tmp, sessions, cache) = fixture();
    let body = task_started_line(D1) + &other_line(D1);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.daily.len(), 1);
    assert_eq!(stats.daily[0].turns, 1);
    assert_eq!(stats.daily[0].tokens, 0);
}

#[test]
fn sums_last_token_usage_delta_not_the_cumulative_total() {
    let (_tmp, sessions, cache) = fixture();
    // Two token_count events with growing cumulative totals but
    // per-call deltas of 100 and 50 — the sum must be the deltas (150),
    // not the final cumulative total (that would be a bug of summing
    // `total_token_usage` instead of `last_token_usage`).
    let body = token_count_line(D1, 100) + &token_count_line(D1, 50);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.daily[0].tokens, 150);
}

#[test]
fn walks_nested_year_month_day_directories() {
    let (_tmp, sessions, cache) = fixture();
    write_log(&sessions, "2026", "07", "20", "a", &task_started_line(D1));
    write_log(&sessions, "2026", "07", "21", "b", &task_started_line(D2));

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.daily.len(), 2);
    assert_eq!(stats.daily[0].date, expected_date(D1));
    assert_eq!(stats.daily[1].date, expected_date(D2));
}

#[test]
fn incremental_tail_parse_skips_already_consumed_bytes() {
    let (_tmp, sessions, cache) = fixture();
    let prefix = task_started_line(D1) + &task_started_line(D1);
    let path = write_log(&sessions, "2026", "07", "20", "a", &prefix);
    let first = update_activity(&sessions, &cache).unwrap();
    assert_eq!(first.daily[0].turns, 2);

    let junk = "x".repeat(prefix.len());
    fs::write(&path, junk + &task_started_line(D1)).unwrap();

    let second = update_activity(&sessions, &cache).unwrap();
    assert_eq!(second.daily[0].turns, 3);
}

#[test]
fn malformed_lines_skipped_without_error() {
    let (_tmp, sessions, cache) = fixture();
    let body = format!("{{ not json\n\n{}", task_started_line(D1));
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn session_meta_cwd_and_id_populate_a_recent_session() {
    let (_tmp, sessions, cache) = fixture();
    let body = session_meta_line("s1", "/Users/x/proj") + &task_started_line(D1);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.recent_sessions.len(), 1);
    let s = &stats.recent_sessions[0];
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.cwd, PathBuf::from("/Users/x/proj"));
    assert_eq!(s.title, None, "no title source is known for Codex yet");
    assert_eq!(s.prompt_preview, None);
    assert_eq!(s.git_branch, None);
}

#[test]
fn legacy_session_meta_id_still_populates_a_recent_session() {
    let (_tmp, sessions, cache) = fixture();
    let body = legacy_id_session_meta_line("s1", "/Users/x/proj") + &task_started_line(D1);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.recent_sessions[0].session_id, "s1");
}

#[test]
fn user_message_populates_the_prompt_preview() {
    let (_tmp, sessions, cache) = fixture();
    let body = session_meta_line("s1", "/Users/x/proj")
        + &task_started_line(D1)
        + &user_message_line(D1, "  improve\n\nrecent session rows  ");
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].prompt_preview.as_deref(),
        Some("improve recent session rows")
    );
}

#[test]
fn session_meta_git_branch_populates_the_recent_session_branch() {
    let (_tmp, sessions, cache) = fixture();
    let body = session_meta_line_with_branch("s1", "/Users/x/proj", "feature/recent-sessions")
        + &task_started_line(D1);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].git_branch.as_deref(),
        Some("feature/recent-sessions")
    );
}

#[test]
fn a_session_meta_missing_the_assumed_fields_yields_no_recent_session() {
    let (_tmp, sessions, cache) = fixture();
    // The expected `payload.session_id`/`payload.cwd` shape is absent — this
    // must degrade to "no recent session for this file", not an error.
    let body = json!({"type": "session_meta", "payload": {"other_field": true}}).to_string()
        + "\n"
        + &task_started_line(D1);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    assert!(stats.recent_sessions.is_empty());
    // The turn count is unaffected — the two concerns are independent.
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn last_active_tracks_the_latest_event_msg_timestamp() {
    let (_tmp, sessions, cache) = fixture();
    let body =
        session_meta_line("s1", "/Users/x/proj") + &task_started_line(D1) + &task_started_line(D2);
    write_log(&sessions, "2026", "07", "20", "a", &body);

    let stats = update_activity(&sessions, &cache).unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339(D2)
        .unwrap()
        .timestamp_millis();
    let actual = stats.recent_sessions[0]
        .last_active
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    assert_eq!(actual, expected);
}

#[test]
fn cache_version_bump_forces_a_recent_sessions_rebuild() {
    let (_tmp, sessions, cache) = fixture();
    write_log(
        &sessions,
        "2026",
        "07",
        "20",
        "a",
        &(session_meta_line("s1", "/Users/x/proj") + &task_started_line(D1)),
    );
    update_activity(&sessions, &cache).unwrap();

    // Simulate a stale v2 cache (no prompt_preview/git_branch
    // fields) at the current file's offset.
    let raw = fs::read_to_string(&cache).unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v["version"] = json!(2);
    fs::write(&cache, v.to_string()).unwrap();

    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats.recent_sessions.len(), 1);
}

use daruda_agent::ActivityStats;

#[test]
fn missing_sessions_root_yields_empty_stats() {
    let (_tmp, sessions, cache) = fixture();
    let stats = update_activity(&sessions, &cache).unwrap();
    assert_eq!(stats, ActivityStats::default());
}
