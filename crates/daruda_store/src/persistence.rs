//! Shared file-based persistence helpers for `tasks.json`, `panels.json`,
//! and `recent.json` / per-project `state.json`.
//!
//! All three share the same atomic-rename write strategy and exhibit the
//! same operational failure mode: a parse error currently looks
//! identical to "file missing" because both return `None`. That hides
//! data corruption from the user and makes silent partial restores
//! debugging-hostile. This module factors the read/parse path so every
//! caller logs the same way and treats the same failures consistently.
//!
//! Atomicity note: writes use `tempfile::NamedTempFile::new_in(dir)?`
//! followed by `persist(target)`, which is a same-FS rename — atomic on
//! POSIX. There is no window where a reader can see a truncated file:
//! the kernel's `rename(2)` either flips the directory entry to the new
//! inode or it doesn't. Concurrent readers either see the old contents
//! (full file) or the new contents (full file). The "corruption"
//! failure mode this module addresses is therefore not a write race —
//! it's a user (or external tool) hand-editing the file into invalid
//! JSON, or a power loss during a non-atomic third-party write.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;

/// Outcome of [`load_json_file`]. `Missing` and `Parsed` are the
/// callers' two normal paths; `Corrupt` is surfaced separately so a
/// caller that wants to differentiate "blank slate" from "file present
/// but unreadable" can.
pub enum LoadOutcome<T> {
    /// File doesn't exist on disk. Caller should fall back to defaults
    /// or seed a fresh state.
    Missing,
    /// File present and parsed successfully.
    Parsed(T),
    /// File present but unreadable (I/O error or invalid JSON). The
    /// helper logged a single line to stderr; the caller decides
    /// whether to treat this like `Missing` or surface it to the user.
    Corrupt,
}

impl<T> LoadOutcome<T> {
    /// Convenience: `Missing` and `Corrupt` collapse to `None`. Use this
    /// at call sites that already discard the distinction.
    pub fn into_option(self) -> Option<T> {
        match self {
            LoadOutcome::Parsed(t) => Some(t),
            LoadOutcome::Missing | LoadOutcome::Corrupt => None,
        }
    }
}

/// Load + parse a JSON file under `path`. Logs once on parse / I/O
/// failure (tagged with `subsystem` so users can grep `daruda` logs).
///
/// Retries the read once on a transient I/O error other than NotFound:
/// rare, but cheap insurance against the brief window where another
/// process is mid-rename across the same target. The retry is a single
/// 50 ms sleep — not a backoff loop.
pub fn load_json_file<T: DeserializeOwned>(subsystem: &str, path: &Path) -> LoadOutcome<T> {
    let json = match read_with_one_retry(path) {
        Ok(Some(s)) => s,
        Ok(None) => return LoadOutcome::Missing,
        Err(e) => {
            LogWriter::log(
                ErrorReport::new(format!("Failed to read {subsystem} state"))
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("subsystem", subsystem)
                    .with_context("path", redact_home(path))
                    .dedup(format!("store.{subsystem}.read"))
                    .build(),
            );
            return LoadOutcome::Corrupt;
        }
    };

    match serde_json::from_str::<T>(&json) {
        Ok(t) => LoadOutcome::Parsed(t),
        Err(e) => {
            LogWriter::log(
                ErrorReport::new(format!("Failed to parse {subsystem} state"))
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("subsystem", subsystem)
                    .with_context("path", redact_home(path))
                    .dedup(format!("store.{subsystem}.parse"))
                    .build(),
            );
            LoadOutcome::Corrupt
        }
    }
}

/// `Ok(Some(_))` = read succeeded. `Ok(None)` = file does not exist.
/// `Err(_)` = I/O error other than NotFound, after one retry.
fn read_with_one_retry(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => {
            std::thread::sleep(std::time::Duration::from_millis(50));
            match std::fs::read_to_string(path) {
                Ok(s) => Ok(Some(s)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

/// Atomic JSON write — pretty-print, write to a tempfile in the same
/// directory, then `persist` (rename) into place. Creates `dir` if
/// missing. Returns the kind of `io::Error` callers already handle.
pub fn save_json_atomic<T: Serialize>(dir: &Path, target: &Path, value: &T) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    tmp.write_all(json.as_bytes())?;
    tmp.flush()?;
    tmp.persist(target).map_err(|e| e.error)?;
    Ok(())
}

/// Default daruda data directory under the platform config dir. All
/// three subsystems point here so a single env override (future) flips
/// every read/write at once.
pub fn default_data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| {
            LogWriter::log(
                ErrorReport::new("Config directory unresolved — using ./daruda")
                    .severity(ErrorSeverity::Warning)
                    .message("dirs::config_dir() returned None; falling back to ./daruda")
                    .at(file!(), line!())
                    .dedup("store.config_dir.fallback")
                    .build(),
            );
            PathBuf::from(".")
        })
        .join("daruda")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Sample {
        x: i32,
    }

    #[test]
    fn missing_file_returns_missing_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nope.json");
        let result: LoadOutcome<Sample> = load_json_file("test", &path);
        assert!(matches!(result, LoadOutcome::Missing));
    }

    #[test]
    fn corrupt_json_returns_corrupt_outcome_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let result: LoadOutcome<Sample> = load_json_file("test", &path);
        assert!(
            matches!(result, LoadOutcome::Corrupt),
            "expected Corrupt, got something else"
        );
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("good.json");
        save_json_atomic(dir.path(), &path, &Sample { x: 42 }).unwrap();
        let result: LoadOutcome<Sample> = load_json_file("test", &path);
        match result {
            LoadOutcome::Parsed(s) => assert_eq!(s, Sample { x: 42 }),
            _ => panic!("expected Parsed"),
        }
    }

    #[test]
    fn load_outcome_into_option_collapses_missing_and_corrupt() {
        let m: LoadOutcome<Sample> = LoadOutcome::Missing;
        assert!(m.into_option().is_none());
        let c: LoadOutcome<Sample> = LoadOutcome::Corrupt;
        assert!(c.into_option().is_none());
        let p: LoadOutcome<Sample> = LoadOutcome::Parsed(Sample { x: 7 });
        assert_eq!(p.into_option(), Some(Sample { x: 7 }));
    }
}
