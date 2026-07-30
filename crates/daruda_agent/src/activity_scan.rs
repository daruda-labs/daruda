//! Shared incremental JSONL activity scanner.
//!
//! Providers differ in where their session logs live and how one JSON object
//! should be interpreted, but the mechanics around append-only JSONL files are
//! the same: keep a byte offset per file, parse only complete appended lines,
//! cache derivative per-file state, and aggregate it into [`ActivityStats`].

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::activity::{ActivityError, ActivityStats, DayActivity, SessionSummary};

pub(crate) trait SessionLogFormat {
    const CACHE_VERSION: u32;

    fn list_logs(root: &Path) -> Result<Vec<PathBuf>, ActivityError>;
    fn count_record(record: &Value, entry: &mut FileEntry);
}

#[derive(Debug, Serialize, Deserialize)]
struct ActivityCache {
    version: u32,
    /// Keyed by absolute file path.
    files: BTreeMap<String, FileEntry>,
}

impl ActivityCache {
    fn new<F: SessionLogFormat>() -> Self {
        Self {
            version: F::CACHE_VERSION,
            files: BTreeMap::new(),
        }
    }
}

/// Per-file parse state plus the per-day counts extracted from it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct FileEntry {
    /// Bytes already parsed. Always ends on a line boundary; an incomplete
    /// final line is left unconsumed and retried on the next call.
    consumed_offset: u64,
    /// File mtime (ms since epoch) at the last refresh. Detects the rare
    /// same-size in-place rewrite that the offset check misses.
    mtime_ms: u64,
    /// File size at the last refresh.
    size: u64,
    /// Counts keyed by local date `"YYYY-MM-DD"`.
    pub(crate) days: BTreeMap<String, FileDayCounts>,
    pub(crate) session_id: Option<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) title: Option<String>,
    pub(crate) title_is_custom: bool,
    pub(crate) prompt_preview: Option<String>,
    pub(crate) git_branch: Option<String>,
    /// Latest activity timestamp seen, epoch milliseconds.
    pub(crate) last_active_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct FileDayCounts {
    pub(crate) turns: u64,
    pub(crate) tokens: u64,
}

/// Refresh an activity cache against one provider's session-log root and
/// return aggregate stats. Per-provider behavior is supplied by `F`.
pub(crate) fn update_activity<F: SessionLogFormat>(
    root: &Path,
    cache_path: &Path,
) -> Result<ActivityStats, ActivityError> {
    let old = load_cache::<F>(cache_path);
    let mut cache = ActivityCache::new::<F>();
    for path in F::list_logs(root)? {
        let key = path.to_string_lossy().into_owned();
        if let Some(entry) = refresh_file::<F>(&path, old.files.get(&key))? {
            cache.files.insert(key, entry);
        }
    }
    save_cache(cache_path, &cache)?;
    Ok(aggregate(&cache))
}

/// Load the cache, treating every failure (missing file, malformed JSON,
/// version mismatch) as "no cache" so the caller falls back to a full rebuild.
fn load_cache<F: SessionLogFormat>(cache_path: &Path) -> ActivityCache {
    let Ok(bytes) = fs::read(cache_path) else {
        return ActivityCache::new::<F>();
    };
    match serde_json::from_slice::<ActivityCache>(&bytes) {
        Ok(cache) if cache.version == F::CACHE_VERSION => cache,
        _ => ActivityCache::new::<F>(),
    }
}

/// Bring one file's cache entry up to date. Returns `None` when the file
/// vanished between listing and reading, which drops it from the cache.
fn refresh_file<F: SessionLogFormat>(
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
        if let Ok(record) = serde_json::from_slice::<Value>(&line) {
            F::count_record(&record, &mut entry);
        }
    }

    entry.size = size;
    entry.mtime_ms = mtime_ms;
    Ok(Some(entry))
}

pub(crate) fn local_date(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Local).date_naive().to_string())
}

pub(crate) fn epoch_millis(timestamp: &str) -> Option<i64> {
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
                title: entry.title.clone(),
                prompt_preview: entry.prompt_preview.clone(),
                git_branch: entry.git_branch.clone(),
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
