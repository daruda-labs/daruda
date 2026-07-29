//! Local aggregation of Codex CLI activity from its rollout session JSONL
//! files, mirroring [`crate::activity`]'s Claude aggregator but reading a
//! different on-disk layout: `<CODEX_HOME>/sessions/<YYYY>/<MM>/<DD>/*.jsonl`
//! instead of Claude's flat `<projects_root>/<project>/<session>.jsonl`.
//!
//! Counting semantics, matched against real Codex session files:
//! - `turns`: `event_msg` records whose `payload.type == "task_started"` —
//!   one per human prompt, mirroring Claude's non-tool-result `user` record.
//! - `tokens`: `event_msg` records whose `payload.type == "token_count"`,
//!   summing `payload.info.last_token_usage.total_tokens` — the *delta* for
//!   that one model call. (`total_token_usage` is the session's running
//!   cumulative total and must not be summed.)
//! - Day attribution: the record's UTC `timestamp` converted to the
//!   **local** date, same as Claude.
//!
//! No sqlite dependency: Codex's `state_*.sqlite` also carries token totals,
//! but the rollout JSONL already carries both turns and tokens, so the
//! incremental byte-offset scan used for Claude applies unchanged.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::activity::{ActivityError, ActivityStats, DayActivity, SessionSummary};

/// Bumped whenever the cache schema or counting semantics change.
///
/// v2: `FileEntry` gained `session_id`/`cwd`/`last_active_ms` for
/// [`ActivityStats::recent_sessions`] — a v1 cache has none of these, so it
/// must rebuild rather than silently report an empty recent-sessions list
/// forever.
const CACHE_VERSION: u32 = 2;

#[derive(Debug, Serialize, Deserialize)]
struct ActivityCache {
    version: u32,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FileEntry {
    consumed_offset: u64,
    mtime_ms: u64,
    size: u64,
    days: BTreeMap<String, FileDayCounts>,
    /// This file's session id, best-effort read from a `session_meta`
    /// record's payload. See [`count_record`]'s doc comment — the exact
    /// payload shape is unverified against a real Codex rollout file.
    session_id: Option<String>,
    /// This session's originating cwd, same best-effort source.
    cwd: Option<PathBuf>,
    /// Latest `event_msg` timestamp seen. No title source is known for
    /// Codex sessions yet, unlike Claude's `ai-title`/`custom-title`.
    last_active_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct FileDayCounts {
    turns: u64,
    tokens: u64,
}

/// Refresh the activity cache against the rollout session logs under
/// `sessions_root` (`<CODEX_HOME>/sessions`) and return the aggregated
/// stats. Same incremental-cache flow as [`crate::activity::update_activity`].
pub fn update_activity(
    sessions_root: &Path,
    cache_path: &Path,
) -> Result<ActivityStats, ActivityError> {
    let old = load_cache(cache_path);
    let mut cache = ActivityCache::new();
    for path in list_session_logs(sessions_root)? {
        let key = path.to_string_lossy().into_owned();
        if let Some(entry) = refresh_file(&path, old.files.get(&key))? {
            cache.files.insert(key, entry);
        }
    }
    save_cache(cache_path, &cache)?;
    Ok(aggregate(&cache))
}

fn load_cache(cache_path: &Path) -> ActivityCache {
    let Ok(bytes) = fs::read(cache_path) else {
        return ActivityCache::new();
    };
    match serde_json::from_slice::<ActivityCache>(&bytes) {
        Ok(cache) if cache.version == CACHE_VERSION => cache,
        _ => ActivityCache::new(),
    }
}

/// Recursively collect every `*.jsonl` under `sessions_root`. Codex nests
/// rollout files three levels deep (`YYYY/MM/DD/`), but this walks any depth
/// so a layout change upstream doesn't silently stop scanning. A missing
/// root is not an error — no Codex history yet, same as Claude's
/// `list_session_logs`.
fn list_session_logs(sessions_root: &Path) -> Result<Vec<PathBuf>, ActivityError> {
    let mut logs = Vec::new();
    match fs::read_dir(sessions_root) {
        Ok(_) => walk_dir(sessions_root, &mut logs),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(logs),
        Err(e) => {
            return Err(ActivityError::Scan {
                path: sessions_root.to_path_buf(),
                source: e,
            });
        }
    }
    Ok(logs)
}

/// Best-effort recursive walk. An unreadable subdirectory is skipped rather
/// than failing the whole scan — one broken day folder must not blank the
/// stats.
fn walk_dir(dir: &Path, logs: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, logs);
        } else if path.extension().is_some_and(|ext| ext == "jsonl") {
            logs.push(path);
        }
    }
}

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
            break;
        }
        entry.consumed_offset += read as u64;
        count_record(&line, &mut entry);
    }

    entry.size = size;
    entry.mtime_ms = mtime_ms;
    Ok(Some(entry))
}

/// Classify one rollout JSONL line. `event_msg` records with a
/// `task_started` or `token_count` payload contribute turns/tokens and
/// roll `last_active_ms` forward; every other `event_msg` payload only
/// rolls `last_active_ms` forward. `session_meta` best-effort captures
/// `cwd`/`session_id` (see its own comment below) but carries no counts.
/// Every other record kind (`response_item`, `world_state`,
/// `turn_context`) is a silent no-op.
fn count_record(line: &[u8], entry: &mut FileEntry) {
    let Ok(record) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let Some(kind) = record.get("type").and_then(Value::as_str) else {
        return;
    };

    if kind == "session_meta" {
        // Best-effort: the exact payload shape is not verified against a
        // real Codex rollout file (none was available on the machine this
        // was written on). A shape mismatch just leaves `cwd`/`session_id`
        // unset — the session then simply doesn't contribute to
        // `recent_sessions` — rather than erroring.
        if entry.session_id.is_none()
            && let Some(id) = record.pointer("/payload/id").and_then(Value::as_str)
        {
            entry.session_id = Some(id.to_string());
        }
        if entry.cwd.is_none()
            && let Some(cwd) = record.pointer("/payload/cwd").and_then(Value::as_str)
        {
            entry.cwd = Some(PathBuf::from(cwd));
        }
        return;
    }

    if kind != "event_msg" {
        return;
    }
    let Some(timestamp) = record.get("timestamp").and_then(Value::as_str) else {
        return;
    };
    let Some(date) = local_date(timestamp) else {
        return;
    };
    if let Some(ms) = epoch_millis(timestamp) {
        entry.last_active_ms = Some(entry.last_active_ms.map_or(ms, |prev| prev.max(ms)));
    }
    let Some(payload_type) = record.pointer("/payload/type").and_then(Value::as_str) else {
        return;
    };

    match payload_type {
        "task_started" => {
            entry.days.entry(date).or_default().turns += 1;
        }
        "token_count" => {
            let delta = record
                .pointer("/payload/info/last_token_usage/total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if delta > 0 {
                entry.days.entry(date).or_default().tokens += delta;
            }
        }
        _ => {}
    }
}

/// RFC 3339 UTC timestamp → local `"YYYY-MM-DD"`. Codex stamps
/// `"2026-07-20T00:18:16.344Z"` (millisecond-precision, `Z`-suffixed), which
/// `parse_from_rfc3339` accepts directly.
fn local_date(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Local).date_naive().to_string())
}

/// RFC 3339 timestamp → epoch milliseconds, for [`FileEntry::last_active_ms`].
fn epoch_millis(timestamp: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn mtime_millis(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

#[derive(Default)]
struct DayAccum {
    turns: u64,
    tokens: u64,
}

fn aggregate(cache: &ActivityCache) -> ActivityStats {
    let mut per_day: BTreeMap<&str, DayAccum> = BTreeMap::new();
    let mut recent_sessions = Vec::new();
    for entry in cache.files.values() {
        for (date, counts) in &entry.days {
            let day = per_day.entry(date.as_str()).or_default();
            day.turns += counts.turns;
            day.tokens += counts.tokens;
        }
        if let (Some(session_id), Some(cwd), Some(last_active_ms)) =
            (&entry.session_id, &entry.cwd, entry.last_active_ms)
        {
            recent_sessions.push(SessionSummary {
                session_id: session_id.clone(),
                cwd: cwd.clone(),
                title: None,
                last_active: UNIX_EPOCH + Duration::from_millis(last_active_ms.max(0) as u64),
            });
        }
    }
    let daily: Vec<DayActivity> = per_day
        .into_iter()
        .map(|(date, accum)| DayActivity {
            date: date.to_string(),
            turns: accum.turns,
            tokens: accum.tokens,
        })
        .collect();
    ActivityStats {
        daily,
        recent_sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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

    /// The `session_meta` shape assumed by `count_record` — unverified
    /// against a real Codex rollout file, so these fixtures double as the
    /// executable spec of that assumption.
    fn session_meta_line(id: &str, cwd: &str) -> String {
        json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd}}).to_string() + "\n"
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
    fn missing_sessions_root_yields_empty_stats() {
        let (_tmp, sessions, cache) = fixture();
        let stats = update_activity(&sessions, &cache).unwrap();
        assert_eq!(stats, ActivityStats::default());
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
    }

    #[test]
    fn a_session_meta_missing_the_assumed_fields_yields_no_recent_session() {
        let (_tmp, sessions, cache) = fixture();
        // The assumed `payload.id`/`payload.cwd` shape is absent — this
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
        let body = session_meta_line("s1", "/Users/x/proj")
            + &task_started_line(D1)
            + &task_started_line(D2);
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

        // Simulate a stale v1 cache (no session_id/cwd/last_active_ms
        // fields) at the current file's offset.
        let raw = fs::read_to_string(&cache).unwrap();
        let mut v: Value = serde_json::from_str(&raw).unwrap();
        v["version"] = json!(1);
        fs::write(&cache, v.to_string()).unwrap();

        let stats = update_activity(&sessions, &cache).unwrap();
        assert_eq!(stats.recent_sessions.len(), 1);
    }
}
