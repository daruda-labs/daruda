//! Whether a lane (or project) root directory can be read on disk.
//!
//! A lane's root can vanish or become unreadable between sessions
//! (deleted worktree, external `git worktree remove`, TCC-denied
//! `readdir`). This runtime-only flag lets the read side short-circuit
//! (skip the scan, tear down the watcher, suppress the toast) and the
//! UI render the lane as unavailable. Recomputed from the live
//! filesystem, never serialized.
//!
//! GPUI-free: pure `std::fs`. The app maps its file-tree load errors onto
//! [`LaneAvailability`] where those errors are defined.

use std::path::Path;

/// Read-availability of a lane or project root directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneAvailability {
    /// The directory exists and this process can read it.
    #[default]
    Present,
    /// The directory does not exist (deleted, moved) or is not a
    /// directory (replaced by a regular file).
    Missing,
    /// The directory exists but the process is denied read access
    /// (e.g. a macOS TCC-restricted folder without Full Disk Access).
    AccessDenied,
}

/// Classify a directory by attempting to read it. `read_dir` is the
/// same probe the app's file-tree scan uses, so a `Present` result here
/// means that scan will succeed.
pub fn classify_dir(path: &Path) -> LaneAvailability {
    match path.read_dir() {
        Ok(_) => LaneAvailability::Present,
        Err(e) => classify_dir_from_error(&e),
    }
}

/// Map a `read_dir` failure onto an availability. Only genuine
/// "gone / unusable" kinds flip the lane; everything else stays
/// `Present`. Split out so the kind-mapping is unit-testable without
/// fabricating a filesystem that yields each error kind.
fn classify_dir_from_error(e: &std::io::Error) -> LaneAvailability {
    match e.kind() {
        // Only genuine "gone / unusable" kinds justify tearing the lane
        // down and showing the inaccessible empty-state.
        std::io::ErrorKind::NotFound => LaneAvailability::Missing,
        std::io::ErrorKind::PermissionDenied => LaneAvailability::AccessDenied,
        std::io::ErrorKind::NotADirectory => LaneAvailability::Missing,
        // Every other kind is transient/unknown — the directory likely
        // still exists, so stay `Present` and let the caller surface it
        // as a normal error toast rather than triggering teardown.
        _ => LaneAvailability::Present,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_present_missing_notadir() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(classify_dir(d.path()), LaneAvailability::Present);
        assert_eq!(
            classify_dir(&d.path().join("nope")),
            LaneAvailability::Missing
        );
        let file = d.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        // A regular file is not a directory → read_dir fails → Missing.
        assert_eq!(classify_dir(&file), LaneAvailability::Missing);
    }

    #[test]
    fn classify_transient_io_error_stays_present() {
        // A transient/unknown I/O kind on a directory that may still
        // exist must NOT flip the lane to Missing (no teardown).
        let interrupted = std::io::Error::new(std::io::ErrorKind::Interrupted, "interrupted");
        assert_eq!(
            classify_dir_from_error(&interrupted),
            LaneAvailability::Present
        );
        let timed_out = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        assert_eq!(
            classify_dir_from_error(&timed_out),
            LaneAvailability::Present
        );
        let generic = std::io::Error::other("boom");
        assert_eq!(classify_dir_from_error(&generic), LaneAvailability::Present);
        // The genuine "gone / unusable" kinds still flip.
        let not_found = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            classify_dir_from_error(&not_found),
            LaneAvailability::Missing
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            classify_dir_from_error(&denied),
            LaneAvailability::AccessDenied
        );
        let not_a_dir = std::io::Error::from(std::io::ErrorKind::NotADirectory);
        assert_eq!(
            classify_dir_from_error(&not_a_dir),
            LaneAvailability::Missing
        );
    }
}
