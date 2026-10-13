//! Cold restore — daruda startup pass over `~/.daruda/status/`.
//!
//! Age is measured from each file's recorded event `timestamp`, not its
//! filesystem mtime (see [`run`] for why). Three age buckets:
//!
//! - `age < stale_threshold` → load as-is.
//! - `stale_threshold <= age < file_ttl` → reset state to `Connecting`,
//!   write back, and load the reset version. Catches the case where a
//!   `SessionEnd` was missed but the session probably isn't really
//!   active any more.
//! - `age >= file_ttl` → delete the file. Catches orphans from crashes
//!   or `kill -9`.
//!
//! [`classify`] is pure (no IO, takes timestamps as args) so unit tests
//! cover every age boundary without monkey-patching the clock.
//! [`run`] is the IO wrapper used once at startup; [`load`] is what a
//! window opened later reads, touching nothing.

use std::path::Path;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;

use crate::SessionStatus;
use crate::hooks::status_file::{
    StatusFile, StatusFileError, delete, list_dir, read, write_atomic,
};

/// Age thresholds for [`classify`]. Wired up by the consumer from
/// `daruda_config::ClaudeStatusConfig::{stale_threshold_secs, file_ttl_days}`.
#[derive(Clone, Copy, Debug)]
pub struct ColdRestorePolicy {
    pub stale_threshold: Duration,
    pub file_ttl: Duration,
}

impl ColdRestorePolicy {
    pub fn from_config_secs(stale_threshold_secs: u64, file_ttl_days: u32) -> Self {
        Self {
            stale_threshold: Duration::from_secs(stale_threshold_secs),
            file_ttl: Duration::from_secs(u64::from(file_ttl_days) * 24 * 60 * 60),
        }
    }
}

/// What to do with one status file at startup.
#[derive(Clone, Debug, PartialEq)]
pub enum ColdRestoreAction {
    /// Load the file as-is into the in-memory store.
    Load,
    /// Reset its `state` to `Connecting`, write back, and load.
    Reset,
    /// Delete the file entirely.
    Delete,
}

/// Pure classification given a file's modified time and the current time.
pub fn classify(
    modified: SystemTime,
    now: SystemTime,
    policy: &ColdRestorePolicy,
) -> ColdRestoreAction {
    let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
    if age >= policy.file_ttl {
        ColdRestoreAction::Delete
    } else if age >= policy.stale_threshold {
        ColdRestoreAction::Reset
    } else {
        ColdRestoreAction::Load
    }
}

/// Convert a recorded UTC event time to a `SystemTime` for age
/// comparison. Whole-second resolution is plenty at TTL scale (minutes
/// to days); a pre-epoch timestamp (impossible in practice) clamps to
/// the epoch, yielding the maximum age.
fn system_time_from_utc(ts: DateTime<Utc>) -> SystemTime {
    let secs = ts.timestamp();
    if secs >= 0 {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        SystemTime::UNIX_EPOCH
    }
}

/// IO pass: enumerate `dir`, classify each file, perform the action,
/// return the entries that should be loaded into the in-memory store.
///
/// Files are aged by their recorded `timestamp` (last-event time), not
/// the file mtime: the `Reset` branch rewrites the file and so refreshes
/// its mtime, which would reset an mtime-based TTL clock on every
/// startup — a perpetually-idle session would then resurrect as
/// `Connecting` forever and never expire. The recorded `timestamp` is
/// immutable across resets, so it ages monotonically and the TTL sweep
/// actually fires.
///
/// Best-effort — individual file failures are skipped, not propagated,
/// because cold restore must not block daruda startup. Returns an
/// error only if the directory itself can't be enumerated for reasons
/// other than "doesn't exist" (which is normal on first run).
pub fn run(dir: &Path, policy: &ColdRestorePolicy) -> Result<Vec<StatusFile>, StatusFileError> {
    // Best-effort sweep of stale per-session lock files left behind
    // when `SessionEnd` fired and the hook handler couldn't safely
    // delete its own lock (it was still holding it). The status JSON
    // sweep below handles its own files; locks need a separate pass
    // because `list_dir` filters them out by extension.
    cleanup_orphan_locks(dir, policy);

    let now = SystemTime::now();
    let mut loaded = Vec::new();

    for entry in list_dir(dir)? {
        match read(&entry.path) {
            Ok(Some(mut file)) => match classify(system_time_from_utc(file.timestamp), now, policy)
            {
                ColdRestoreAction::Load => loaded.push(file),
                ColdRestoreAction::Reset => {
                    file.status = SessionStatus::Connecting;
                    // A blocking prompt recorded by a now-stale session is
                    // no longer actionable. Dropping the subtype keeps the
                    // rewrite (seen as a watcher `Changed` event by every
                    // running instance) from re-raising its desktop push.
                    file.notification = None;
                    if let Err(e) = write_atomic(&entry.path, &file) {
                        // Still load the in-memory copy so the user
                        // sees status; the disk lag is tolerable.
                        LogWriter::log(
                            ErrorReport::new("Claude session reset write failed")
                                .severity(ErrorSeverity::Warning)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("path", redact_home(&entry.path))
                                .dedup("claude.cold_restore.reset_write")
                                .build(),
                        );
                    }
                    loaded.push(file);
                }
                ColdRestoreAction::Delete => {
                    let _ = delete(&entry.path);
                }
            },
            // Malformed or mid-write — no usable event timestamp, so fall
            // back to the file mtime. A recent file is probably mid-write
            // (leave it; the next event overwrites). Anything past the
            // stale threshold is an orphan — drop it.
            Ok(None) => {
                if classify(entry.modified, now, policy) != ColdRestoreAction::Load {
                    let _ = delete(&entry.path);
                }
            }
            Err(_) => continue,
        }
    }
    Ok(loaded)
}

/// Read the status files as they stand, changing none of them — for a
/// window opened after [`run`] already swept the directory. Sweeping again
/// would rewrite the file of a session another window is following, and
/// its watcher would read that rewrite as the session's own news.
pub fn load(dir: &Path) -> Result<Vec<StatusFile>, StatusFileError> {
    Ok(list_dir(dir)?
        .into_iter()
        .filter_map(|entry| read(&entry.path).ok().flatten())
        .collect())
}

/// Delete `<dir>/*.lock` files older than `policy.file_ttl`. Silent
/// on errors — this is housekeeping, not correctness.
fn cleanup_orphan_locks(dir: &Path, policy: &ColdRestorePolicy) {
    let now = SystemTime::now();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "lock") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age >= policy.file_ttl {
            let _ = std::fs::remove_file(&path);
        }
    }
}
