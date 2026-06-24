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
//! [`run`] is the IO wrapper used at startup.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::status_file::{StatusFile, path_for, write_atomic};
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn policy() -> ColdRestorePolicy {
        ColdRestorePolicy::from_config_secs(300, 7)
    }

    fn fresh_file(state: SessionStatus) -> StatusFile {
        StatusFile::new_hook("s1", "/tmp/x", state, "Stop")
    }

    /// A status file whose recorded event `timestamp` is `age` in the
    /// past — what `run` now ages by, independent of the file mtime.
    fn file_aged(state: SessionStatus, age: Duration) -> StatusFile {
        let mut f = StatusFile::new_hook("s1", "/tmp/x", state, "Stop");
        f.timestamp = Utc::now() - chrono::Duration::from_std(age).unwrap();
        f
    }

    #[test]
    fn classify_fresh_loads() {
        let now = SystemTime::now();
        let modified = now - Duration::from_secs(60);
        assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Load);
    }

    #[test]
    fn classify_stale_resets() {
        let now = SystemTime::now();
        let modified = now - Duration::from_secs(600); // 10 min
        assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Reset);
    }

    #[test]
    fn classify_old_deletes() {
        let now = SystemTime::now();
        let modified = now - Duration::from_secs(8 * 24 * 60 * 60); // 8 days
        assert_eq!(
            classify(modified, now, &policy()),
            ColdRestoreAction::Delete
        );
    }

    #[test]
    fn classify_boundary_at_threshold_resets() {
        let now = SystemTime::now();
        let modified = now - Duration::from_secs(300);
        assert_eq!(classify(modified, now, &policy()), ColdRestoreAction::Reset);
    }

    #[test]
    fn classify_boundary_at_ttl_deletes() {
        let now = SystemTime::now();
        let modified = now - Duration::from_secs(7 * 24 * 60 * 60);
        assert_eq!(
            classify(modified, now, &policy()),
            ColdRestoreAction::Delete
        );
    }

    #[test]
    fn run_loads_fresh_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "fresh");
        write_atomic(&p, &fresh_file(SessionStatus::Working)).unwrap();

        let loaded = run(dir.path(), &policy()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, SessionStatus::Working);
        assert!(p.exists());
    }

    #[test]
    fn run_missing_dir_yields_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let nope = dir.path().join("not-there");
        let loaded = run(&nope, &policy()).unwrap();
        assert!(loaded.is_empty());
    }

    /// Force a file's mtime to a specific value via std `File::set_modified`.
    /// Returns `false` if the platform doesn't honour the change (rare on
    /// modern macOS / Linux); the caller treats that as test-skip.
    fn force_mtime(path: &std::path::Path, when: SystemTime) -> bool {
        let f = match fs::File::options().write(true).open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        if f.set_modified(when).is_err() {
            return false;
        }
        let read_back = fs::metadata(path).and_then(|m| m.modified()).ok();
        match read_back {
            Some(t) => {
                let drift = t
                    .duration_since(when)
                    .or_else(|_| when.duration_since(t))
                    .unwrap_or(Duration::ZERO);
                drift < Duration::from_secs(2)
            }
            None => false,
        }
    }

    #[test]
    fn run_resets_stale_and_keeps_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "stale");
        // Event 10 min ago: past the stale threshold, under the TTL.
        write_atomic(
            &p,
            &file_aged(SessionStatus::Working, Duration::from_secs(600)),
        )
        .unwrap();

        let loaded = run(dir.path(), &policy()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, SessionStatus::Connecting);
        // Reset writes the reset version back to disk.
        let on_disk = read(&p).unwrap().unwrap();
        assert_eq!(on_disk.status, SessionStatus::Connecting);
    }

    /// Regression: a stale file that recorded a blocking notification
    /// subtype (e.g. `idle_prompt`) must not carry it through `Reset`.
    /// The rewrite fires the status watcher in every running daruda
    /// instance, and a preserved subtype re-raises a desktop push for
    /// a prompt that is long gone.
    #[test]
    fn run_reset_clears_stale_notification() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "stale-notif");
        let mut f = file_aged(SessionStatus::Idle, Duration::from_secs(600));
        f.notification = Some(crate::hooks::events::NotificationType::IdlePrompt);
        write_atomic(&p, &f).unwrap();

        let loaded = run(dir.path(), &policy()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].notification, None);
        let on_disk = read(&p).unwrap().unwrap();
        assert_eq!(on_disk.notification, None);
    }

    #[test]
    fn run_deletes_old_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "old");
        // Event 8 days ago: past the 7-day TTL.
        write_atomic(
            &p,
            &file_aged(SessionStatus::Idle, Duration::from_secs(8 * 24 * 60 * 60)),
        )
        .unwrap();

        let loaded = run(dir.path(), &policy()).unwrap();
        assert!(loaded.is_empty());
        assert!(!p.exists());
    }

    /// Regression: a perpetually-idle session repeatedly reset to
    /// `Connecting` keeps a fresh mtime (every `Reset` rewrites it) but an
    /// old event timestamp. It must still age out by the event timestamp,
    /// not live forever because the rewrite touched the mtime.
    #[test]
    fn run_ages_by_event_timestamp_not_mtime() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "immortal");
        // 30-day-old event time, but `write_atomic` gives it a brand-new
        // mtime — the exact shape of an accumulated stale husk.
        write_atomic(
            &p,
            &file_aged(
                SessionStatus::Connecting,
                Duration::from_secs(30 * 24 * 60 * 60),
            ),
        )
        .unwrap();

        let loaded = run(dir.path(), &policy()).unwrap();
        assert!(
            loaded.is_empty(),
            "stale-by-timestamp file must be deleted regardless of fresh mtime"
        );
        assert!(!p.exists());
    }

    /// The inverse: a recently-active session with an ancient mtime (e.g.
    /// restored from a backup) is kept, proving the event timestamp — not
    /// the mtime — is the age basis.
    #[test]
    fn run_keeps_recent_event_even_with_old_mtime() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "recent");
        write_atomic(
            &p,
            &file_aged(SessionStatus::Working, Duration::from_secs(60)),
        )
        .unwrap();

        let old = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        if !force_mtime(&p, old) {
            return; // platform skipped
        }

        let loaded = run(dir.path(), &policy()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, SessionStatus::Working);
        assert!(p.exists());
    }

    #[test]
    fn run_skips_malformed_recent_file_in_place() {
        // A malformed file with a fresh mtime is left alone — could be
        // mid-write. Don't load it, but don't delete it either.
        let dir = tempfile::TempDir::new().unwrap();
        let p = path_for(dir.path(), "wip");
        fs::write(&p, b"{ partial").unwrap();
        let loaded = run(dir.path(), &policy()).unwrap();
        assert!(loaded.is_empty());
        assert!(p.exists());
    }

    #[test]
    fn run_sweeps_old_lock_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let stale_lock = dir.path().join("orphan.lock");
        let fresh_lock = dir.path().join("active.lock");
        fs::write(&stale_lock, b"").unwrap();
        fs::write(&fresh_lock, b"").unwrap();

        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 60 * 60);
        if !force_mtime(&stale_lock, old) {
            return; // platform skipped
        }

        run(dir.path(), &policy()).unwrap();
        assert!(
            !stale_lock.exists(),
            "stale lock file should have been deleted"
        );
        assert!(fresh_lock.exists(), "fresh lock should be left alone");
    }

    #[test]
    fn run_skips_non_lock_non_json_files() {
        // The sweep must not touch unrelated extensions. Status JSON
        // sweep already filters by `.json`, lock sweep by `.lock` —
        // a stray README or `.tmp` should survive both.
        let dir = tempfile::TempDir::new().unwrap();
        let stray = dir.path().join("README.md");
        fs::write(&stray, b"hello").unwrap();
        run(dir.path(), &policy()).unwrap();
        assert!(stray.exists());
    }
}
