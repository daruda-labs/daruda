use super::*;
use serde_json::json;
use std::io::Write;
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

/// Noon UTC — far enough from both midnights that the local date
/// equals what chrono computes for every real-world timezone the
/// test host could run in.
const D1: &str = "2026-06-01T12:00:00Z";
const D2: &str = "2026-06-02T12:00:00Z";

/// What the production conversion should yield, computed
/// independently in the test so the assertion stays
/// timezone-portable.
fn expected_date(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .unwrap()
        .with_timezone(&chrono::Local)
        .date_naive()
        .to_string()
}

fn user_line(ts: &str, sid: &str) -> String {
    json!({"type": "user", "timestamp": ts, "sessionId": sid,
           "message": {"role": "user", "content": "hi"}})
    .to_string()
        + "\n"
}

fn tool_result_user_line(ts: &str, sid: &str) -> String {
    json!({"type": "user", "timestamp": ts, "sessionId": sid,
    "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}
    ]}})
    .to_string()
        + "\n"
}

fn assistant_line(ts: &str, input_tokens: u64, output_tokens: u64) -> String {
    json!({"type": "assistant", "timestamp": ts,
           "message": {"role": "assistant", "content": [{"type": "text", "text": "sure"}],
                       "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}}})
    .to_string()
        + "\n"
}

fn plain_line(kind: &str, ts: &str) -> String {
    json!({"type": kind, "timestamp": ts}).to_string() + "\n"
}

fn user_line_with_cwd(ts: &str, sid: &str, cwd: &str) -> String {
    user_line_with_cwd_and_message(ts, sid, cwd, "hi")
}

fn user_line_with_cwd_and_message(ts: &str, sid: &str, cwd: &str, message: &str) -> String {
    json!({"type": "user", "timestamp": ts, "sessionId": sid, "cwd": cwd,
           "message": {"role": "user", "content": message}})
    .to_string()
        + "\n"
}

fn user_line_with_cwd_and_branch(ts: &str, sid: &str, cwd: &str, branch: &str) -> String {
    json!({"type": "user", "timestamp": ts, "sessionId": sid, "cwd": cwd,
           "gitBranch": branch, "message": {"role": "user", "content": "hi"}})
    .to_string()
        + "\n"
}

/// `ai-title`/`custom-title` records carry no `timestamp` field on
/// real Claude Code session logs — the fixtures deliberately omit one
/// too, so a regression that requires a timestamp for these types is
/// caught.
fn ai_title_line(sid: &str, title: &str) -> String {
    json!({"type": "ai-title", "sessionId": sid, "aiTitle": title}).to_string() + "\n"
}

fn custom_title_line(sid: &str, title: &str) -> String {
    json!({"type": "custom-title", "sessionId": sid, "customTitle": title}).to_string() + "\n"
}

fn last_prompt_line(sid: &str, prompt: &str) -> String {
    json!({"type": "last-prompt", "sessionId": sid, "lastPrompt": prompt}).to_string() + "\n"
}

/// `projects_root/<project>/<name>.jsonl` ← `content`.
fn write_log(root: &Path, project: &str, name: &str, content: &str) -> PathBuf {
    let dir = root.join(project);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let projects = tmp.path().join("projects");
    let cache = tmp.path().join("cache").join("activity.json");
    (tmp, projects, cache)
}

#[test]
fn display_title_prefers_title_then_prompt_preview() {
    let mut session = SessionSummary {
        session_id: "s1".to_string(),
        cwd: PathBuf::from("/Users/x/proj"),
        title: Some("  Fix the bug  ".to_string()),
        prompt_preview: Some("Raw prompt".to_string()),
        git_branch: None,
        last_active: std::time::SystemTime::now(),
    };
    assert_eq!(session.display_title(), Some("Fix the bug"));

    session.title = Some("  ".to_string());
    assert_eq!(session.display_title(), Some("Raw prompt"));

    session.prompt_preview = Some("\n\t".to_string());
    assert_eq!(session.display_title(), None);
}

#[test]
fn activity_paths_for_scopes_projects_root_and_cache_to_config_dir() {
    let account_id = "alice-1234";
    let config_dir = Path::new("/data/claude-accounts").join(account_id);

    let (projects_root, cache_path) = claude_activity_paths(Some(&config_dir))
        .expect("claude_activity_paths should resolve for an explicit config_dir");

    assert_eq!(projects_root, config_dir.join("projects"));

    let cache_file_name = cache_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("cache path should have a file name");
    assert!(
        cache_file_name.contains(account_id),
        "cache file name {cache_file_name:?} should embed the account key {account_id:?}"
    );
    assert_ne!(
        cache_file_name, "activity.json",
        "account-scoped cache must not collide with the system-default cache file name"
    );
}

#[test]
fn counts_user_turns_but_not_other_record_types() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line(D1, "s1")
        + &plain_line("attachment", D1)
        + &plain_line("system", D1)
        + &plain_line("summary", D1)
        + &plain_line("queue-operation", D1);
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily.len(), 1);
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn tool_result_user_records_do_not_count_as_turns() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line(D1, "s1") + &tool_result_user_line(D1, "s2");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn assistant_usage_tokens_summed_per_day() {
    let (_tmp, projects, cache) = fixture();
    let body = assistant_line(D1, 10, 5) + &assistant_line(D1, 3, 2);
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].tokens, 20);
}

#[test]
fn utc_timestamps_attributed_to_local_dates_ascending() {
    let (_tmp, projects, cache) = fixture();
    // Written newest-first to prove the output is sorted by date,
    // not by file order.
    let body = user_line(D2, "s2") + &user_line(D1, "s1");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily.len(), 2);
    assert_eq!(stats.daily[0].date, expected_date(D1));
    assert_eq!(stats.daily[1].date, expected_date(D2));
}

#[test]
fn records_without_parseable_timestamp_skipped() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line(D1, "s1")
        + &(json!({"type": "user", "message": {"content": "no ts"}}).to_string() + "\n")
        + &(json!({"type": "user", "timestamp": "garbage"}).to_string() + "\n");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily.iter().map(|d| d.turns).sum::<u64>(), 1);
}

#[test]
fn malformed_lines_skipped_without_error() {
    let (_tmp, projects, cache) = fixture();
    let body = format!("{{ not json\n\n{}", user_line(D1, "s1"));
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn incremental_tail_parse_skips_already_consumed_bytes() {
    let (_tmp, projects, cache) = fixture();
    let prefix = user_line(D1, "s1") + &user_line(D1, "s1");
    let path = write_log(&projects, "p", "a", &prefix);
    let first = update_activity(&projects, &cache).unwrap();
    assert_eq!(first.daily[0].turns, 2);

    // Overwrite the consumed prefix with same-length junk, then
    // append one valid line. If the second call re-read from byte
    // 0 the junk would parse as nothing and the count would drop
    // to 1; staying at 3 proves only the tail was read.
    let junk = "x".repeat(prefix.len());
    fs::write(&path, junk + &user_line(D1, "s1")).unwrap();

    let second = update_activity(&projects, &cache).unwrap();
    assert_eq!(second.daily[0].turns, 3);
}

#[test]
fn truncated_file_reparsed_from_scratch() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line(D1, "s1") + &user_line(D1, "s1") + &user_line(D1, "s1");
    let path = write_log(&projects, "p", "a", &body);
    assert_eq!(
        update_activity(&projects, &cache).unwrap().daily[0].turns,
        3
    );

    // Shrink the file below the consumed offset.
    fs::write(&path, user_line(D1, "s9")).unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn incomplete_last_line_consumed_only_after_newline_lands() {
    let (_tmp, projects, cache) = fixture();
    let full = user_line(D1, "s1");
    let second = user_line(D1, "s2");
    let (head, tail) = second.split_at(second.len() / 2);
    let path = write_log(&projects, "p", "a", &(full.clone() + head));

    // Mid-append: the partial line must not count yet.
    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);

    // The writer finishes the line — only the completed remainder
    // is parsed on the next call.
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(tail.as_bytes()).unwrap();
    drop(f);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 2);
}

#[test]
fn corrupt_cache_triggers_full_rebuild() {
    let (_tmp, projects, cache) = fixture();
    write_log(&projects, "p", "a", &user_line(D1, "s1"));
    update_activity(&projects, &cache).unwrap();

    fs::write(&cache, "{ definitely not a cache").unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn cache_version_mismatch_triggers_full_rebuild() {
    let (_tmp, projects, cache) = fixture();
    write_log(&projects, "p", "a", &user_line(D1, "s1"));
    update_activity(&projects, &cache).unwrap();

    // Rewrite the cache as a future version with bogus contents —
    // it must be discarded, not trusted.
    let raw = fs::read_to_string(&cache).unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v["version"] = json!(CLAUDE_CACHE_VERSION + 1);
    v["files"] = json!({});
    fs::write(&cache, v.to_string()).unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn deleted_file_drops_out_of_stats() {
    let (_tmp, projects, cache) = fixture();
    let path_a = write_log(&projects, "p1", "a", &user_line(D1, "s1"));
    write_log(&projects, "p2", "b", &user_line(D1, "s2"));
    assert_eq!(
        update_activity(&projects, &cache).unwrap().daily[0].turns,
        2
    );

    fs::remove_file(&path_a).unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.daily[0].turns, 1);
}

#[test]
fn missing_projects_root_yields_empty_stats() {
    let (_tmp, projects, cache) = fixture();
    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats, ActivityStats::default());
    // The cache is still written so the next call starts warm.
    assert!(cache.exists());
}

#[test]
fn unchanged_file_repeats_identical_stats() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line(D1, "s1");
    let path = write_log(&projects, "p", "a", &body);
    let first = update_activity(&projects, &cache).unwrap();

    // The whole file was consumed — the cache offset sits at EOF,
    // so the next call takes the unchanged fast path.
    let v: Value = serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
    let entry = &v["files"][path.to_string_lossy().as_ref()];
    assert_eq!(
        entry["consumed_offset"].as_u64().unwrap(),
        body.len() as u64
    );
    assert_eq!(entry["size"].as_u64().unwrap(), body.len() as u64);

    let second = update_activity(&projects, &cache).unwrap();
    assert_eq!(first, second);
}

#[test]
fn error_display_includes_path() {
    let err = ActivityError::Scan {
        path: PathBuf::from("/nope/projects"),
        source: std::io::Error::other("denied"),
    };
    assert!(err.to_string().contains("/nope/projects"));
    assert!(err.to_string().contains("denied"));
}

#[test]
fn a_session_with_no_user_or_assistant_record_has_no_recent_session() {
    let (_tmp, projects, cache) = fixture();
    // Only a title record, no cwd/session_id ever gets captured since
    // that only happens on `user`/`assistant` lines.
    write_log(&projects, "p", "a", &ai_title_line("s1", "Some title"));

    let stats = update_activity(&projects, &cache).unwrap();
    assert!(stats.recent_sessions.is_empty());
}

#[test]
fn a_session_with_cwd_and_session_id_appears_in_recent_sessions() {
    let (_tmp, projects, cache) = fixture();
    write_log(
        &projects,
        "p",
        "a",
        &user_line_with_cwd(D1, "s1", "/Users/x/proj"),
    );

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.recent_sessions.len(), 1);
    let s = &stats.recent_sessions[0];
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.cwd, PathBuf::from("/Users/x/proj"));
    assert_eq!(s.title, None);
    assert_eq!(s.prompt_preview.as_deref(), Some("hi"));
    assert_eq!(s.git_branch, None);
}

#[test]
fn ai_title_sets_the_session_title() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line_with_cwd(D1, "s1", "/Users/x/proj") + &ai_title_line("s1", "Fix the bug");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].title.as_deref(),
        Some("Fix the bug")
    );
}

#[test]
fn a_later_ai_title_overwrites_an_earlier_one() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line_with_cwd(D1, "s1", "/Users/x/proj")
        + &ai_title_line("s1", "First guess")
        + &ai_title_line("s1", "Better guess");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].title.as_deref(),
        Some("Better guess")
    );
}

#[test]
fn an_explicit_custom_title_wins_over_a_later_ai_title() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line_with_cwd(D1, "s1", "/Users/x/proj")
        + &custom_title_line("s1", "My chosen title")
        + &ai_title_line("s1", "Auto-generated guess");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].title.as_deref(),
        Some("My chosen title")
    );
}

#[test]
fn last_prompt_sets_the_session_prompt_preview() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line_with_cwd(D1, "s1", "/Users/x/proj")
        + &last_prompt_line("s1", "  fix\n\nrecent session labels  ");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].prompt_preview.as_deref(),
        Some("fix recent session labels")
    );
}

#[test]
fn git_branch_sets_the_session_branch() {
    let (_tmp, projects, cache) = fixture();
    write_log(
        &projects,
        "p",
        "a",
        &user_line_with_cwd_and_branch(D1, "s1", "/Users/x/proj", "feature/recent-sessions"),
    );

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(
        stats.recent_sessions[0].git_branch.as_deref(),
        Some("feature/recent-sessions")
    );
}

#[test]
fn last_active_tracks_the_latest_user_or_assistant_timestamp() {
    let (_tmp, projects, cache) = fixture();
    let body = user_line_with_cwd(D1, "s1", "/Users/x/proj")
        + &user_line_with_cwd(D2, "s1", "/Users/x/proj");
    write_log(&projects, "p", "a", &body);

    let stats = update_activity(&projects, &cache).unwrap();
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
    let (_tmp, projects, cache) = fixture();
    write_log(
        &projects,
        "p",
        "a",
        &user_line_with_cwd(D1, "s1", "/Users/x/proj"),
    );
    update_activity(&projects, &cache).unwrap();

    // Simulate a stale v2 cache (no session_id/cwd/title fields at
    // all) sitting at the current file's offset — the version gate
    // must discard it wholesale rather than deserialize partially.
    let raw = fs::read_to_string(&cache).unwrap();
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    v["version"] = json!(2);
    fs::write(&cache, v.to_string()).unwrap();

    let stats = update_activity(&projects, &cache).unwrap();
    assert_eq!(stats.recent_sessions.len(), 1);
}
