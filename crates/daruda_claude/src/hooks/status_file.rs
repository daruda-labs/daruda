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
use crate::hooks::events::PermissionMode;

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
            timestamp: Utc::now(),
            source: Source::Hook,
        }
    }
}

/// Errors from status file IO.
#[derive(Debug)]
pub enum StatusFileError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// `dirs::home_dir()` returned `None` — extremely rare on a real
    /// macOS install, but the hook handler still needs to exit
    /// gracefully so this is a recoverable signal.
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

/// Default status directory: `~/.daruda/status/`.
pub fn default_dir() -> Result<PathBuf, StatusFileError> {
    let home = dirs::home_dir().ok_or(StatusFileError::NoHome)?;
    Ok(home.join(".daruda").join("status"))
}

/// Path for a single session's status file inside `dir`.
pub fn path_for(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(format!("{session_id}.json"))
}

/// Path for a single session's advisory lock file inside `dir`. The
/// hook handler flocks this during its read-modify-write window; cold
/// restore and dead-session pruning sweep it. Centralized here so the
/// `.lock` suffix has a single definition.
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

/// Enumerate every `*.json` in `dir`. Used by cold restore (A-9).
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> StatusFile {
        StatusFile::new_hook("sess-1", "/tmp/cwd", SessionStatus::Working, "PreToolUse")
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
}
