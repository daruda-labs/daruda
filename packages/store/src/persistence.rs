//! Shared JSON persistence with synced atomic replacement and a previous
//! readable generation. Reads distinguish missing records from corruption;
//! recovery preserves the damaged bytes before restoring the backup.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::storage::StorageLayout;
use serde::{Serialize, de::DeserializeOwned};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;

mod recovery;
pub use recovery::{delete_recoverable_json, load_recoverable_json, save_recoverable_json};

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
    /// helper logged the failure; the caller decides
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
/// Recover a valid prior generation while preserving damaged original bytes.
pub fn load_json_file<T: DeserializeOwned>(subsystem: &str, path: &Path) -> LoadOutcome<T> {
    match load_recoverable_json(subsystem, path) {
        Ok(Some(value)) => LoadOutcome::Parsed(value),
        Ok(None) => LoadOutcome::Missing,
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
            LoadOutcome::Corrupt
        }
    }
}

/// Atomic JSON write — pretty-print, write to a tempfile in the same
/// directory, then `persist` (rename) into place. Creates `dir` if
/// missing. Returns the kind of `io::Error` callers already handle.
pub fn save_json_atomic<T: Serialize>(dir: &Path, target: &Path, value: &T) -> std::io::Result<()> {
    recovery::save_json_with_backup(dir, target, value)
}

fn write_json_atomic<T: Serialize>(dir: &Path, target: &Path, value: &T) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    if json.len() > recovery::MAX_RECORD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "State record is too large",
        ));
    }
    tmp.write_all(json.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    for attempt in 0..4 {
        match tmp.persist(target) {
            Ok(_) => return Ok(()),
            Err(error) => {
                if !cfg!(windows)
                    || !matches!(error.error.raw_os_error(), Some(32 | 33))
                    || attempt == 3
                {
                    return Err(error.error);
                }
                tmp = error.file;
                std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
            }
        }
    }
    Ok(())
}

/// Compatibility root for configuration, accounts and existing feature data.
/// Workspace state is owned separately by `project::WorkspaceStore`.
/// Environment and profile are captured once for all storage consumers.
pub fn default_data_dir() -> PathBuf {
    let directory = StorageLayout::current().data();
    if directory.is_relative() {
        LogWriter::log(
            ErrorReport::new("Config directory unresolved — using ./daruda")
                .severity(ErrorSeverity::Warning)
                .message("OS config directory unavailable; using the compatibility fallback")
                .at(file!(), line!())
                .dedup("store.config_dir.fallback")
                .build(),
        );
    }
    directory
}

/// The active profile's suffix for non-path resources that need the same
/// release/debug/named-profile isolation [`default_data_dir`] gives
/// on-disk paths — e.g. a macOS Keychain service name, so a debug build
/// and a release install never share (and long-poll-conflict on) the
/// same stored secret. `None` for the release profile (keeps the
/// existing unsuffixed name); `Some(profile)` otherwise.
pub fn profile_suffix() -> Option<&'static str> {
    match crate::profile::active_profile() {
        crate::profile::RELEASE_PROFILE => None,
        other => Some(other),
    }
}

/// Directory holding the app-managed Node.js runtime for the ACP adapter.
///
/// Deliberately **profile-independent**: the runtime's version is pinned in
/// `daruda_acp`, so one install (tens of MB) serves every profile — release,
/// debug, and named — instead of being duplicated per profile like
/// [`default_data_dir`]. Honors `DARUDA_DATA_DIR` (so tests / portable installs
/// stay isolated), otherwise lands under the shared `daruda/node` regardless of
/// the active profile.
pub fn node_install_dir() -> PathBuf {
    StorageLayout::current().node_install()
}

/// Where flow run locks live: `<config>/daruda/flow-locks`.
///
/// Deliberately **profile-independent**, for the opposite reason to
/// [`default_data_dir`]. This is not daruda's own state — it is a mutex on
/// something every profile shares, the user's working tree. A release build
/// and a debug build running a flow in one checkout must exclude each
/// other, and a per-profile lock root would have each of them see a free
/// tree and put two agents in it. The same reasoning [`node_install_dir`]
/// gives for being shared, arrived at from the other side.
///
/// Honors `DARUDA_DATA_DIR` so tests and portable installs stay isolated —
/// two suites pointed at different data directories are not sharing a
/// working tree either.
pub fn flow_lock_root() -> PathBuf {
    StorageLayout::current().flow_locks()
}

/// Where the per-bot connection locks live: `<config>/daruda/remote-locks`.
///
/// Profile-**independent**, the third of the deliberate exceptions and for the
/// same reason as [`flow_lock_root`]: this is not daruda's own state but a
/// mutex on something outside it that every profile shares — the bot account
/// itself. Telegram serves `getUpdates` to one poller per token and Slack
/// hands each event to one of an app's open sockets, so a release install and
/// a debug build pointed at one bot have to exclude each other. A per-profile
/// lock root would have both see a free bot and split the user's replies
/// between them, which is the failure it exists to stop.
///
/// Honors `DARUDA_DATA_DIR` so tests and portable installs stay isolated —
/// two suites pointed at different data directories are not sharing a bot
/// either.
pub fn remote_lock_root() -> PathBuf {
    StorageLayout::current().remote_locks()
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
