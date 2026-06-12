//! Local aggregation of Claude Code activity from session JSONL files.
//!
//! daruda deliberately does **not** read `~/.claude/stats-cache.json` —
//! that file is owned by Claude Code and its schema can change without
//! notice. Instead this module scans the session logs
//! (`<projects_root>/<encoded-project>/<session>.jsonl`) itself and
//! keeps an incremental cache at a caller-supplied path (in production
//! `~/.daruda/cache/activity.json`; path resolution is the app layer's
//! job — this module is path-agnostic and GPUI-free).
//!
//! JSONL session logs are append-only, so the cache stores a consumed
//! byte offset per file and each [`update_activity`] call parses only
//! the appended tail. The first call after a cache wipe parses full
//! history (potentially hundreds of MB) — callers run it on a
//! background thread; parsing streams line-by-line and never loads a
//! whole file into memory.
//!
//! Counting semantics match the Übersicht `claude-usage` widget (and
//! thereby Claude Code's own `stats-cache.json` message counts):
//! - `messages`: records of type `user` / `assistant` / `attachment` /
//!   `system`.
//! - `tool_calls`: `tool_use` blocks inside `assistant` message content.
//! - `sessions`: distinct `sessionId`s on `user` records, excluding
//!   tool-result feedback records (content array containing a
//!   `tool_result` block).
//! - Day attribution: the record's UTC `timestamp` converted to the
//!   **local** date. Records without a parseable timestamp are skipped.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Aggregated activity for one local calendar day.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayActivity {
    /// `"YYYY-MM-DD"` in the local timezone.
    pub date: String,
    pub messages: u64,
    /// Distinct session ids seen that day (unioned across files).
    pub sessions: u64,
    pub tool_calls: u64,
}

/// Aggregate of all activity found under the projects root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActivityStats {
    /// Ascending by date.
    pub daily: Vec<DayActivity>,
    pub total_messages: u64,
    /// Distinct session ids across all days and files.
    pub total_sessions: u64,
    /// Days with at least one counted message.
    pub active_days: u64,
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

/// Bumped whenever the cache schema or the counting semantics change.
/// A loaded cache with any other version is discarded and rebuilt
/// from the JSONL files, so a version bump is always safe.
const CACHE_VERSION: u32 = 1;

/// On-disk incremental cache. Plain serde JSON; corruption or version
/// mismatch is never an error — the cache is a pure derivative of the
/// JSONL files and can always be rebuilt.
#[derive(Debug, Serialize, Deserialize)]
struct ActivityCache {
    version: u32,
    /// Keyed by absolute file path.
    files: BTreeMap<String, FileEntry>,
}

impl ActivityCache {
    fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            files: BTreeMap::new(),
        }
    }
}

/// Per-file parse state plus the per-day counts extracted from it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FileEntry {
    /// Bytes already parsed. Always ends on a line boundary — an
    /// incomplete final line is left unconsumed and retried on the
    /// next call.
    consumed_offset: u64,
    /// File mtime (ms since epoch) at the last refresh. Detects the
    /// rare same-size in-place rewrite that the offset check misses.
    mtime_ms: u64,
    /// File size at the last refresh.
    size: u64,
    /// Counts keyed by local date `"YYYY-MM-DD"`.
    days: BTreeMap<String, FileDayCounts>,
}

/// One file's contribution to one day.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FileDayCounts {
    messages: u64,
    tool_calls: u64,
    /// Kept as a set (not a count) so the aggregation step can union
    /// session ids across files — one session frequently spans the
    /// main session log and sidechain logs.
    session_ids: BTreeSet<String>,
}

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
    let old = load_cache(cache_path);
    let mut cache = ActivityCache::new();
    for path in list_session_logs(projects_root)? {
        let key = path.to_string_lossy().into_owned();
        if let Some(entry) = refresh_file(&path, old.files.get(&key))? {
            cache.files.insert(key, entry);
        }
    }
    save_cache(cache_path, &cache)?;
    Ok(aggregate(&cache))
}

/// Load the cache, treating every failure (missing file, malformed
/// JSON, version mismatch) as "no cache" so the caller falls back to
/// a full rebuild.
fn load_cache(cache_path: &Path) -> ActivityCache {
    let Ok(bytes) = fs::read(cache_path) else {
        return ActivityCache::new();
    };
    match serde_json::from_slice::<ActivityCache>(&bytes) {
        Ok(cache) if cache.version == CACHE_VERSION => cache,
        _ => ActivityCache::new(),
    }
}

/// List `projects_root/*/*.jsonl`. A missing root is not an error —
/// the machine simply has no Claude Code history yet — but any other
/// listing failure on the root is surfaced. Unreadable child entries
/// are skipped: one broken project dir must not blank out the stats.
fn list_session_logs(projects_root: &Path) -> Result<Vec<PathBuf>, ActivityError> {
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

/// Bring one file's cache entry up to date. Returns `None` when the
/// file vanished between listing and reading (drop it from the cache).
///
/// Change detection, in order:
/// - fully consumed and mtime unchanged → reuse the entry untouched;
/// - grown past `consumed_offset` → append-only growth, parse the tail;
/// - anything else (shrunk = truncate/rewrite, or same size with a
///   changed mtime = in-place rewrite, or no cache entry) → reparse
///   from byte 0.
fn refresh_file(
    path: &Path,
    cached: Option<&FileEntry>,
) -> Result<Option<FileEntry>, ActivityError> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(None);
    };
    let size = meta.len();
    let mtime_ms = mtime_millis(&meta);

    let mut entry = match cached {
        Some(e) if e.consumed_offset == size && e.mtime_ms == mtime_ms => {
            return Ok(Some(e.clone()));
        }
        Some(e) if size > e.consumed_offset => e.clone(),
        _ => FileEntry::default(),
    };

    let Ok(file) = fs::File::open(path) else {
        return Ok(None);
    };
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(entry.consumed_offset))
        .map_err(|e| ActivityError::Read {
            path: path.to_path_buf(),
            source: e,
        })?;

    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| ActivityError::Read {
                path: path.to_path_buf(),
                source: e,
            })?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            // Incomplete final line — the writer is mid-append. Leave
            // it unconsumed so the next call retries once the rest of
            // the line (and its newline) has landed.
            break;
        }
        entry.consumed_offset += read as u64;
        count_record(&line, &mut entry.days);
    }

    entry.size = size;
    entry.mtime_ms = mtime_ms;
    Ok(Some(entry))
}

/// Classify one JSONL line and fold it into the per-day counts.
/// Malformed JSON and records without a parseable timestamp are
/// skipped silently — session logs routinely contain entry kinds we
/// don't model.
fn count_record(line: &[u8], days: &mut BTreeMap<String, FileDayCounts>) {
    let Ok(record) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let Some(date) = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(local_date)
    else {
        return;
    };
    let Some(kind) = record.get("type").and_then(Value::as_str) else {
        return;
    };
    if !matches!(kind, "user" | "assistant" | "attachment" | "system") {
        return;
    }

    let day = days.entry(date).or_default();
    day.messages += 1;

    let content = record.pointer("/message/content");
    match kind {
        "user" => {
            // A user record whose content array carries a tool_result
            // block is Claude Code feeding a tool output back in — not
            // a human turn, so it must not mark the session active.
            let is_tool_result = content.and_then(Value::as_array).is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            });
            if !is_tool_result
                && let Some(sid) = record.get("sessionId").and_then(Value::as_str)
                && !sid.is_empty()
            {
                day.session_ids.insert(sid.to_string());
            }
        }
        "assistant" => {
            if let Some(blocks) = content.and_then(Value::as_array) {
                day.tool_calls += blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                    .count() as u64;
            }
        }
        _ => {}
    }
}

/// RFC 3339 UTC timestamp → local `"YYYY-MM-DD"`. `None` on any parse
/// failure so the record is skipped rather than misattributed.
fn local_date(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Local).date_naive().to_string())
}

fn mtime_millis(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Atomic cache write: serialize, write to a temp file in the same
/// directory, then rename over the target. A crash mid-write leaves
/// the previous cache intact (or none — both rebuild cleanly).
fn save_cache(cache_path: &Path, cache: &ActivityCache) -> Result<(), ActivityError> {
    let write_err = |source: std::io::Error| ActivityError::CacheWrite {
        path: cache_path.to_path_buf(),
        source,
    };
    let parent = cache_path
        .parent()
        .ok_or_else(|| write_err(std::io::Error::other("cache path has no parent directory")))?;
    fs::create_dir_all(parent).map_err(write_err)?;
    let bytes = serde_json::to_vec(cache).map_err(|e| write_err(e.into()))?;

    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(write_err)?;
    tmp.write_all(&bytes).map_err(write_err)?;
    tmp.flush().map_err(write_err)?;
    tmp.persist(cache_path).map_err(|e| write_err(e.error))?;
    Ok(())
}

/// Accumulator for one day across all files.
#[derive(Default)]
struct DayAccum<'a> {
    messages: u64,
    tool_calls: u64,
    sessions: BTreeSet<&'a str>,
}

/// Fold the per-file day counts into the public stats. Session ids
/// are unioned per day and globally; messages and tool calls sum.
fn aggregate(cache: &ActivityCache) -> ActivityStats {
    let mut per_day: BTreeMap<&str, DayAccum<'_>> = BTreeMap::new();
    let mut all_sessions: BTreeSet<&str> = BTreeSet::new();
    for entry in cache.files.values() {
        for (date, counts) in &entry.days {
            let day = per_day.entry(date.as_str()).or_default();
            day.messages += counts.messages;
            day.tool_calls += counts.tool_calls;
            day.sessions
                .extend(counts.session_ids.iter().map(String::as_str));
            all_sessions.extend(counts.session_ids.iter().map(String::as_str));
        }
    }

    // BTreeMap iterates keys in lexicographic order, which for
    // "YYYY-MM-DD" strings is chronological order.
    let daily: Vec<DayActivity> = per_day
        .into_iter()
        .map(|(date, accum)| DayActivity {
            date: date.to_string(),
            messages: accum.messages,
            sessions: accum.sessions.len() as u64,
            tool_calls: accum.tool_calls,
        })
        .collect();

    ActivityStats {
        total_messages: daily.iter().map(|d| d.messages).sum(),
        total_sessions: all_sessions.len() as u64,
        active_days: daily.iter().filter(|d| d.messages > 0).count() as u64,
        daily,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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

    fn assistant_line(ts: &str, tool_uses: usize) -> String {
        let mut content = vec![json!({"type": "text", "text": "sure"})];
        for i in 0..tool_uses {
            content.push(json!({"type": "tool_use", "id": format!("t{i}"),
                                "name": "Bash", "input": {}}));
        }
        json!({"type": "assistant", "timestamp": ts,
               "message": {"role": "assistant", "content": content}})
        .to_string()
            + "\n"
    }

    fn plain_line(kind: &str, ts: &str) -> String {
        json!({"type": kind, "timestamp": ts}).to_string() + "\n"
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
    fn counts_all_four_message_record_types() {
        let (_tmp, projects, cache) = fixture();
        let body = user_line(D1, "s1")
            + &assistant_line(D1, 0)
            + &plain_line("attachment", D1)
            + &plain_line("system", D1)
            + &plain_line("summary", D1)
            + &plain_line("queue-operation", D1);
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 4);
        assert_eq!(stats.daily.len(), 1);
        assert_eq!(stats.daily[0].messages, 4);
        assert_eq!(stats.active_days, 1);
    }

    #[test]
    fn tool_result_user_records_count_as_messages_but_not_sessions() {
        let (_tmp, projects, cache) = fixture();
        let body = user_line(D1, "s1") + &tool_result_user_line(D1, "s2");
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.daily[0].messages, 2);
        // s2 only ever appears on a tool_result feedback record.
        assert_eq!(stats.daily[0].sessions, 1);
        assert_eq!(stats.total_sessions, 1);
    }

    #[test]
    fn tool_use_blocks_counted_per_assistant_record() {
        let (_tmp, projects, cache) = fixture();
        let body = assistant_line(D1, 2) + &assistant_line(D1, 1) + &assistant_line(D1, 0);
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.daily[0].tool_calls, 3);
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
        assert_eq!(stats.active_days, 2);
    }

    #[test]
    fn records_without_parseable_timestamp_skipped() {
        let (_tmp, projects, cache) = fixture();
        let body = user_line(D1, "s1")
            + &(json!({"type": "user", "message": {"content": "no ts"}}).to_string() + "\n")
            + &(json!({"type": "user", "timestamp": "garbage"}).to_string() + "\n");
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
    }

    #[test]
    fn malformed_lines_skipped_without_error() {
        let (_tmp, projects, cache) = fixture();
        let body = format!("{{ not json\n\n{}", user_line(D1, "s1"));
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
    }

    #[test]
    fn incremental_tail_parse_skips_already_consumed_bytes() {
        let (_tmp, projects, cache) = fixture();
        let prefix = user_line(D1, "s1") + &user_line(D1, "s1");
        let path = write_log(&projects, "p", "a", &prefix);
        let first = update_activity(&projects, &cache).unwrap();
        assert_eq!(first.total_messages, 2);

        // Overwrite the consumed prefix with same-length junk, then
        // append one valid line. If the second call re-read from byte
        // 0 the junk would parse as nothing and the count would drop
        // to 1; staying at 3 proves only the tail was read.
        let junk = "x".repeat(prefix.len());
        fs::write(&path, junk + &user_line(D1, "s1")).unwrap();

        let second = update_activity(&projects, &cache).unwrap();
        assert_eq!(second.total_messages, 3);
    }

    #[test]
    fn truncated_file_reparsed_from_scratch() {
        let (_tmp, projects, cache) = fixture();
        let body = user_line(D1, "s1") + &user_line(D1, "s1") + &user_line(D1, "s1");
        let path = write_log(&projects, "p", "a", &body);
        assert_eq!(
            update_activity(&projects, &cache).unwrap().total_messages,
            3
        );

        // Shrink the file below the consumed offset.
        fs::write(&path, user_line(D1, "s9")).unwrap();

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
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
        assert_eq!(stats.total_messages, 1);
        assert_eq!(stats.total_sessions, 1);

        // The writer finishes the line — only the completed remainder
        // is parsed on the next call.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(tail.as_bytes()).unwrap();
        drop(f);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 2);
        assert_eq!(stats.total_sessions, 2);
    }

    #[test]
    fn corrupt_cache_triggers_full_rebuild() {
        let (_tmp, projects, cache) = fixture();
        write_log(&projects, "p", "a", &user_line(D1, "s1"));
        update_activity(&projects, &cache).unwrap();

        fs::write(&cache, "{ definitely not a cache").unwrap();

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
        assert_eq!(stats.total_sessions, 1);
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
        v["version"] = json!(CACHE_VERSION + 1);
        v["files"] = json!({});
        fs::write(&cache, v.to_string()).unwrap();

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
    }

    #[test]
    fn deleted_file_drops_out_of_stats() {
        let (_tmp, projects, cache) = fixture();
        let path_a = write_log(&projects, "p1", "a", &user_line(D1, "s1"));
        write_log(&projects, "p2", "b", &user_line(D1, "s2"));
        assert_eq!(
            update_activity(&projects, &cache).unwrap().total_messages,
            2
        );

        fs::remove_file(&path_a).unwrap();

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.total_messages, 1);
        assert_eq!(stats.total_sessions, 1);
    }

    #[test]
    fn same_day_sessions_unioned_across_files() {
        let (_tmp, projects, cache) = fixture();
        // One session spans two files (main log + sidechain); a second
        // session appears in only one of them.
        write_log(
            &projects,
            "p1",
            "a",
            &(user_line(D1, "shared") + &user_line(D1, "solo")),
        );
        write_log(&projects, "p2", "b", &user_line(D1, "shared"));

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.daily.len(), 1);
        assert_eq!(stats.daily[0].sessions, 2);
        assert_eq!(stats.total_sessions, 2);
        assert_eq!(stats.daily[0].messages, 3);
    }

    #[test]
    fn session_spanning_two_days_counts_once_in_total() {
        let (_tmp, projects, cache) = fixture();
        let body = user_line(D1, "s1") + &user_line(D2, "s1");
        write_log(&projects, "p", "a", &body);

        let stats = update_activity(&projects, &cache).unwrap();
        assert_eq!(stats.daily.len(), 2);
        assert_eq!(stats.daily[0].sessions, 1);
        assert_eq!(stats.daily[1].sessions, 1);
        assert_eq!(stats.total_sessions, 1);
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
}
