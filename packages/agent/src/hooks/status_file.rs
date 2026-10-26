//! Status file persistence — `~/.daruda/status/<session_id>.json`.
//!
//! daruda's hook handler ([`crate::hooks::fsm`]) writes one of these per
//! Claude session. The main daruda process watches the directory with
//! `notify` and reads files back into the in-memory store.
//!
//! Writes are atomic (tempfile in the same directory + rename). Reads of
//! missing or malformed files are graceful (`Ok(None)` rather than an
//! error) so the hook handler can treat absent state as "first event for
//! this session".

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::SessionStatus;
use crate::hooks::events::{NotificationType, PermissionMode};

pub(super) mod lock;

/// Bumped only when a structural change can't be absorbed by
/// `#[serde(default)]` on optional fields.
pub const SCHEMA_VERSION: u32 = 1;

/// Which channel produced this status entry. Hook is authoritative;
/// JSONL is used as a fallback when hooks are unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Hook,
    Jsonl,
}

/// On-disk schema for `~/.daruda/status/<session_id>.json`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StatusFile {
    pub schema_version: u32,
    pub session_id: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    #[serde(rename = "state")]
    pub status: SessionStatus,
    pub last_event: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Set only on `Notification` events — carries the subtype so the
    /// app-side ingest can decide whether to raise a transient desktop
    /// push (blocking subtypes) without re-parsing the hook payload.
    /// `None` for every other event.
    #[serde(default)]
    pub notification: Option<NotificationType>,
    pub timestamp: DateTime<Utc>,
    pub source: Source,
}

impl StatusFile {
    /// Build a current-time hook-source entry for the given session.
    pub fn new_hook(
        session_id: impl Into<String>,
        cwd: impl Into<PathBuf>,
        status: SessionStatus,
        last_event: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id: session_id.into(),
            cwd: cwd.into(),
            transcript_path: None,
            status,
            last_event: last_event.into(),
            tool_name: None,
            tool_input: None,
            permission_mode: None,
            notification: None,
            timestamp: Utc::now(),
            source: Source::Hook,
        }
    }

    /// `true` when the recorded event time is at least `max_age` in the
    /// past. Push gate for blocking notifications: an entry re-delivered
    /// long after it fired (cold-restore rewrite, watcher replay) is no
    /// longer actionable and must not raise a desktop push. The boundary
    /// matches `cold_restore::classify` (`>=` counts as expired); a
    /// future timestamp (clock skew) is never expired.
    pub fn event_expired(&self, now: DateTime<Utc>, max_age: std::time::Duration) -> bool {
        (now - self.timestamp)
            .to_std()
            .is_ok_and(|age| age >= max_age)
    }
}

/// Errors from status file IO.
#[derive(Debug)]
pub enum StatusFileError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// `default_dir()` (via `daruda_store::persistence::default_data_dir`)
    /// has no unresolvable-home code path today; this variant exists so a
    /// future resolver change has somewhere to signal one without a
    /// breaking enum change.
    NoHome,
}

impl std::fmt::Display for StatusFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "status file io: {e}"),
            Self::Json(e) => write!(f, "status file json: {e}"),
            Self::NoHome => f.write_str("could not resolve user home directory"),
        }
    }
}

impl std::error::Error for StatusFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::NoHome => None,
        }
    }
}

impl From<std::io::Error> for StatusFileError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for StatusFileError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Default status directory: `<profile-scoped data dir>/status/` — same
/// base as `daruda_store::persistence::default_data_dir` (logs,
/// workspaces, panels), so a debug build's hook writes never land in
/// (or get read back by) a real release install's status directory.
/// Release keeps the pre-existing unsuffixed path.
pub fn default_dir() -> Result<PathBuf, StatusFileError> {
    Ok(daruda_store::persistence::default_data_dir().join("status"))
}

/// Path for a single session's status file inside `dir`.
pub fn path_for(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.json"))
}

/// Path for a single session's advisory lock file inside `dir`. The
/// hook handler and startup maintenance lock this during read-modify-write.
/// The pathname stays in place: unlinking it can let a second writer lock
/// a new file while a previous writer still owns the old one.
pub fn lock_path_for(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.lock"))
}

/// Read a status file. Returns `Ok(None)` if the file does not exist
/// or fails to parse — the caller treats both as "no prior state".
pub fn read(path: &Path) -> Result<Option<StatusFile>, StatusFileError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_slice::<StatusFile>(&bytes) {
        Ok(s) => Ok(Some(s)),
        // Malformed (e.g. half-written by an interrupted writer, or
        // schema drift in a future daruda version) — treat as missing.
        Err(_) => Ok(None),
    }
}

/// Atomic write: serialize to a tempfile in the same directory, then
/// `rename(2)` into place. POSIX guarantees the rename is atomic on
/// the same filesystem (HFS+/APFS qualifies).
pub fn write_atomic(path: &Path, value: &StatusFile) -> Result<(), StatusFileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;

    let mut tmp = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| std::io::Error::other("status file path has no parent"))?,
    )?;
    tmp.write_all(&bytes)?;
    tmp.flush()?;
    tmp.persist(path)
        .map_err(|e| StatusFileError::Io(e.error))?;
    Ok(())
}

/// Remove a status file. Missing-file is not an error.
pub fn delete(path: &Path) -> Result<(), StatusFileError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Listing entry for cold-restore — full path + last-modified time.
pub struct StatusFileEntry {
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// Enumerate every `*.json` in `dir`. Used by cold restore.
/// Files that fail to stat are skipped silently (best-effort).
pub fn list_dir(dir: &Path) -> Result<Vec<StatusFileEntry>, StatusFileError> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        out.push(StatusFileEntry { path, modified });
    }
    Ok(out)
}
