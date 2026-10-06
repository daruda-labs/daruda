//! Local aggregation of Claude Code activity from session JSONL files.
//!
//! daruda deliberately does **not** read `~/.claude/stats-cache.json` —
//! that file is owned by Claude Code and its schema can change without
//! notice. Instead this module scans the session logs
//! (`<projects_root>/<encoded-project>/<session>.jsonl`) itself and
//! keeps an incremental cache at a caller-supplied path (in production
//! `~/.daruda/cache/activity.json`). Callers can pass explicit paths to
//! [`update_activity`] or use [`source_for`] for production path resolution.
//!
//! JSONL session logs are append-only, so the cache stores a consumed
//! byte offset per file and each [`update_activity`] call parses only
//! the appended tail. The first call after a cache wipe parses full
//! history (potentially hundreds of MB) — callers run it on a
//! background thread; parsing streams line-by-line and never loads a
//! whole file into memory.
//!
//! Counting semantics match the Übersicht `claude-usage` widget's
//! `daily` shape (`{date, turns, tokens}`):
//! - `turns`: `user` records, excluding tool-result feedback records
//!   (content array containing a `tool_result` block) — one turn per
//!   human prompt.
//! - `tokens`: `input_tokens + output_tokens` summed from every
//!   `assistant` record's `message.usage`.
//! - Day attribution: the record's UTC `timestamp` converted to the
//!   **local** date. Records without a parseable timestamp are skipped.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use daruda_store::accounts::AccountRecipeId;

use crate::activity_scan::{
    FileEntry, SessionLogFormat, epoch_millis, local_date, update_activity as update_jsonl_activity,
};

/// Aggregated activity for one local calendar day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayActivity {
    /// `"YYYY-MM-DD"` in the local timezone.
    pub date: String,
    pub turns: u64,
    pub tokens: u64,
}

/// One session found while scanning — enough to render a recent-sessions
/// row and restore it into a new pane. Built from whichever `user`/
/// `assistant` records a session's file actually carried, so a session
/// with no such record yet (freshly created, still empty) never appears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_id: String,
    /// The session's originating working directory, as Claude Code wrote
    /// it into the JSONL — an absolute path, not yet matched against any
    /// daruda Lane.
    pub cwd: PathBuf,
    /// Explicit `custom-title` first, else the most recent `ai-title`
    /// seen (Claude Code regenerates it as the conversation evolves),
    /// else `None`.
    pub title: Option<String>,
    /// One-line preview of the user's latest prompt. Used as a display
    /// fallback for providers that do not persist generated titles.
    pub prompt_preview: Option<String>,
    /// Last git branch reported by the session log, when available.
    pub git_branch: Option<String>,
    pub last_active: SystemTime,
}

impl SessionSummary {
    /// Best provider-neutral display title: explicit/generated session title
    /// first, then a normalized prompt preview. Path fallback remains a UI
    /// decision because the caller owns lane/project context.
    pub fn display_title(&self) -> Option<&str> {
        session_display_title(self.title.as_deref(), self.prompt_preview.as_deref())
    }
}

/// Provider-neutral display-title rule shared by agent activity producers and
/// UI projections: prefer a captured/generated title, then a normalized prompt
/// preview. Callers still own path/lane fallback because that context is not
/// part of every activity source.
pub fn session_display_title<'a>(
    title: Option<&'a str>,
    prompt_preview: Option<&'a str>,
) -> Option<&'a str> {
    title
        .and_then(non_blank_trimmed)
        .or_else(|| prompt_preview.and_then(non_blank_trimmed))
}

/// Aggregate of all activity found under the projects root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityStats {
    /// Ascending by date.
    pub daily: Vec<DayActivity>,
    /// Unsorted, uncapped — every session this scan resolved a `cwd` +
    /// `session_id` for. Filtering to a caller's open Lanes and capping
    /// to a display count is the caller's job; this module has no
    /// concept of a Lane.
    pub recent_sessions: Vec<SessionSummary>,
}

/// Failure surface for [`update_activity`]. Per-file races (a session
/// log deleted between listing and reading) are not errors — the file
/// simply drops out of the stats. Errors are reserved for problems
/// the user can act on: an unreadable projects root, an I/O failure
/// mid-read, or a cache that cannot be written.
#[derive(Debug, thiserror::Error)]
pub enum ActivityError {
    #[error("failed to scan projects root {}: {source}", path.display())]
    Scan {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read session log {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write activity cache {}: {source}", path.display())]
    CacheWrite {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Where one auth domain's local activity comes from. One implementation per
/// domain, resolved through [`source_for`] so app call sites stay free of
/// provider-specific session-log branching.
pub trait ActivitySource: Send + Sync {
    /// Fetch and parse this domain's local activity. `config_dir` is a managed
    /// account's isolated home, `None` the ambient system login. Blocking disk
    /// I/O; call from a background thread.
    fn fetch(&self, config_dir: Option<&Path>) -> Option<ActivityStats>;
}

/// The activity source for `id`. Total, like `usage::source_for`.
pub fn source_for(id: AccountRecipeId) -> &'static dyn ActivitySource {
    crate::providers::integration_for(id).activity
}

pub struct ClaudeActivity;

impl ActivitySource for ClaudeActivity {
    fn fetch(&self, config_dir: Option<&Path>) -> Option<ActivityStats> {
        let (projects_root, cache_path) = claude_activity_paths(config_dir)?;
        update_activity(&projects_root, &cache_path).ok()
    }
}

/// Resolve Claude's session-log root and profile-scoped cache path.
///
/// `None` reads the system default `~/.claude/projects`; `Some(config_dir)`
/// reads a managed account's isolated `CLAUDE_CONFIG_DIR/projects`. The cache
/// always lives under daruda's profile-scoped data dir so debug/test/release
/// profiles do not overwrite each other's derived activity cache.
fn claude_activity_paths(config_dir: Option<&Path>) -> Option<(PathBuf, PathBuf)> {
    let (projects_root, cache_file_name) = match config_dir {
        Some(dir) => {
            let key = dir.file_name().and_then(|name| name.to_str());
            (
                dir.join("projects"),
                key.map(|k| format!("activity-{k}.json"))
                    .unwrap_or_else(|| "activity-managed.json".to_string()),
            )
        }
        None => {
            let home = dirs::home_dir()?;
            (
                home.join(".claude").join("projects"),
                "activity.json".to_string(),
            )
        }
    };
    let cache_path = daruda_store::persistence::default_data_dir()
        .join("cache")
        .join(cache_file_name);
    Some((projects_root, cache_path))
}

/// Bumped whenever the cache schema or the counting semantics change.
/// A loaded cache with any other version is discarded and rebuilt
/// from the JSONL files, so a version bump is always safe.
///
/// v2: `FileDayCounts` switched from `{messages, tool_calls, session_ids}`
/// to `{turns, tokens}` — an old v1 cache would deserialize with the wrong
/// field names, so the version bump forces a full rebuild instead.
///
/// v3: `FileEntry` gained `session_id`/`cwd`/`title`/`title_is_custom`/
/// `last_active_ms` for [`ActivityStats::recent_sessions`] — a v2 cache
/// has none of these, so it must rebuild rather than silently report an
/// empty recent-sessions list forever.
///
/// v4: recent sessions gained `prompt_preview` and `git_branch`, so cached
/// v3 file entries need a rebuild to populate the new display fields.
const CLAUDE_CACHE_VERSION: u32 = 4;

/// Refresh the activity cache against the session logs under
/// `projects_root` and return the aggregated stats.
///
/// Flow: load cache → list `projects_root/*/*.jsonl` → incrementally
/// parse changed files → atomically save the cache → aggregate.
/// Files present in the cache but gone from disk drop out — the stats
/// always reflect what is on disk right now.
pub fn update_activity(
    projects_root: &Path,
    cache_path: &Path,
) -> Result<ActivityStats, ActivityError> {
    update_jsonl_activity::<ClaudeLogFormat>(projects_root, cache_path)
}

struct ClaudeLogFormat;

impl SessionLogFormat for ClaudeLogFormat {
    const CACHE_VERSION: u32 = CLAUDE_CACHE_VERSION;

    /// List `projects_root/*/*.jsonl`. A missing root is not an error — the
    /// machine simply has no Claude Code history yet — but any other listing
    /// failure on the root is surfaced. Unreadable child entries are skipped:
    /// one broken project dir must not blank out the stats.
    fn list_logs(projects_root: &Path) -> Result<Vec<PathBuf>, ActivityError> {
        let mut logs = Vec::new();
        let dirs = match fs::read_dir(projects_root) {
            Ok(dirs) => dirs,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
            Err(e) => {
                return Err(ActivityError::Scan {
                    path: projects_root.to_path_buf(),
                    source: e,
                });
            }
        };
        for project_dir in dirs.flatten() {
            let dir_path = project_dir.path();
            if !dir_path.is_dir() {
                continue;
            }
            let Ok(entries) = fs::read_dir(&dir_path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file() {
                    logs.push(path);
                }
            }
        }
        Ok(logs)
    }

    /// Classify one JSONL object and fold it into `entry`. `ai-title`/
    /// `custom-title`/`last-prompt` records carry no `timestamp` field at all,
    /// so title and preview capture runs before the timestamp gate below.
    fn count_record(record: &Value, entry: &mut FileEntry) {
        let Some(kind) = record.get("type").and_then(Value::as_str) else {
            return;
        };

        match kind {
            "ai-title" => {
                if !entry.title_is_custom
                    && let Some(title) = record.get("aiTitle").and_then(Value::as_str)
                {
                    entry.title = Some(title.to_string());
                }
                return;
            }
            "custom-title" => {
                if let Some(title) = record.get("customTitle").and_then(Value::as_str) {
                    entry.title = Some(title.to_string());
                    entry.title_is_custom = true;
                }
                return;
            }
            "last-prompt" => {
                if let Some(preview) = record
                    .get("lastPrompt")
                    .and_then(Value::as_str)
                    .and_then(session_prompt_preview)
                {
                    entry.prompt_preview = Some(preview);
                }
                return;
            }
            _ => {}
        }

        let Some(timestamp) = record.get("timestamp").and_then(Value::as_str) else {
            return;
        };
        let Some(date) = local_date(timestamp) else {
            return;
        };

        if let Some(branch) = record
            .get("gitBranch")
            .and_then(Value::as_str)
            .and_then(non_blank_owned)
        {
            entry.git_branch = Some(branch);
        }

        if matches!(kind, "user" | "assistant") {
            if entry.session_id.is_none()
                && let Some(sid) = record.get("sessionId").and_then(Value::as_str)
            {
                entry.session_id = Some(sid.to_string());
            }
            if entry.cwd.is_none()
                && let Some(cwd) = record.get("cwd").and_then(Value::as_str)
            {
                entry.cwd = Some(PathBuf::from(cwd));
            }
            if let Some(ms) = epoch_millis(timestamp) {
                entry.last_active_ms = Some(entry.last_active_ms.map_or(ms, |prev| prev.max(ms)));
            }
        }

        match kind {
            "user" => {
                // A user record whose content array carries a tool_result
                // block is Claude Code feeding a tool output back in — not
                // a human turn.
                let is_tool_result = record
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks
                            .iter()
                            .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                    });
                if !is_tool_result {
                    entry.days.entry(date).or_default().turns += 1;
                    if let Some(preview) = record
                        .pointer("/message/content")
                        .and_then(prompt_preview_from_message_content)
                    {
                        entry.prompt_preview = Some(preview);
                    }
                }
            }
            "assistant" => {
                let usage = record.pointer("/message/usage");
                let input = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let output = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if input + output > 0 {
                    entry.days.entry(date).or_default().tokens += input + output;
                }
            }
            _ => {}
        }
    }
}

const SESSION_PROMPT_PREVIEW_MAX_CHARS: usize = 240;

pub(crate) fn session_prompt_preview(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut pending_space = false;
    let mut chars = 0;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(ch);
        chars += 1;
        if chars >= SESSION_PROMPT_PREVIEW_MAX_CHARS {
            break;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn prompt_preview_from_message_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => session_prompt_preview(s),
        Value::Array(blocks) => {
            let mut text = String::new();
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("text") {
                    continue;
                }
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(part);
                }
            }
            session_prompt_preview(&text)
        }
        _ => None,
    }
}

fn non_blank_owned(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn non_blank_trimmed(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

#[cfg(test)]
mod tests {
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
        let body =
            user_line_with_cwd(D1, "s1", "/Users/x/proj") + &ai_title_line("s1", "Fix the bug");
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
}
