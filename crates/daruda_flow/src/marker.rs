//! How a run ended, written as a file, and read back as a status.
//!
//! A crash cannot write a record of itself, so `Crashed` is derived from a
//! missing marker plus a dead lock holder rather than from a marker of its
//! own.

use crate::error::{FlowIoError, IoSite};
use crate::lock::IsAlive;
use crate::schedule::RunOutcome;
use std::path::{Path, PathBuf};

/// The marker file names, which are also the vocabulary a user reading the
/// run directory sees.
const DONE: &str = "DONE";
const FAILED: &str = "FAILED";
const CANCELED: &str = "CANCELED";

const WRITE_MARKER: &str = "recording how the run ended";

/// The three a run can write about itself, plus the two that can only be
/// derived. `Crashed` has no marker because a crash cannot write one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Done,
    Failed,
    Canceled,
    Running,
    Crashed,
    Unknown,
}

/// Record how the run ended, in its own run directory.
///
/// Must happen before the lock is released: in the window between a freed
/// lock and an unwritten marker, a reader sees neither and calls a finished
/// run `Unknown`.
pub fn write_marker(run_dir: &Path, outcome: &RunOutcome) -> Result<(), FlowIoError> {
    let Some(name) = marker_name(outcome) else {
        return Ok(());
    };
    let path = run_dir.join(name);
    std::fs::create_dir_all(run_dir)
        // Empty: the name is the whole message, and why a run failed is
        // `run.md`'s (P2b-3) to say.
        .and_then(|()| std::fs::write(&path, ""))
        .map_err(|source| FlowIoError {
            site: IoSite::Run,
            doing: WRITE_MARKER,
            path,
            source,
        })
}

/// How a finished run classifies, coarsely. `Io` and `BudgetExhausted` are
/// `Failed` too — they did not succeed, and which of them it was is `RunEnd`'s
/// finer axis to carry. `None` is `LockHeld`: that run never took the
/// directory, so it has nothing to say about it.
///
/// The one place the six outcomes are folded into three. `event::RunEnd` does
/// not repeat this fold — it spreads the same outcomes out instead, so a
/// seventh variant is decided once on each axis rather than twice on one.
pub fn status_of(outcome: &RunOutcome) -> Option<RunStatus> {
    match outcome {
        RunOutcome::Done => Some(RunStatus::Done),
        RunOutcome::Failed { .. }
        | RunOutcome::BudgetExhausted { .. }
        | RunOutcome::Io(_)
        | RunOutcome::Unprovisioned { .. } => Some(RunStatus::Failed),
        RunOutcome::Canceled { .. } => Some(RunStatus::Canceled),
        // Neither took the directory, so neither has anything to say
        // about it.
        RunOutcome::LockHeld { .. } | RunOutcome::Invalid { .. } => None,
    }
}

/// The file name a status is written under. `Running`, `Crashed` and
/// `Unknown` have none — they are read from evidence, never written, so no
/// outcome produces them.
fn marker_name(outcome: &RunOutcome) -> Option<&'static str> {
    match status_of(outcome)? {
        RunStatus::Done => Some(DONE),
        RunStatus::Failed => Some(FAILED),
        RunStatus::Canceled => Some(CANCELED),
        RunStatus::Running | RunStatus::Crashed | RunStatus::Unknown => None,
    }
}

/// What state a run directory is in. With no marker the lock is the only
/// evidence there is, so where the lock lives is part of this answer — and
/// it is derived from `run_dir` rather than passed, because a caller that
/// named the wrong directory would see every live run as `Unknown` and
/// nothing would say so.
pub fn run_status(run_dir: &Path, is_alive: IsAlive<'_>) -> RunStatus {
    // The runs directory, which is where `schedule::execute` takes the
    // lock — one derivation, so the two cannot drift apart.
    let Some(lock_dir) = run_dir.parent() else {
        return RunStatus::Unknown;
    };
    if run_dir.join(DONE).is_file() {
        return RunStatus::Done;
    }
    if run_dir.join(FAILED).is_file() {
        return RunStatus::Failed;
    }
    if run_dir.join(CANCELED).is_file() {
        return RunStatus::Canceled;
    }
    // A lock naming a different run is evidence about that run, not this
    // one; an unreadable or absent one is no evidence at all. Either way
    // there is nothing here to call a crash without guessing.
    match crate::lock::read_holder(lock_dir) {
        Some(holder) if !holds_this_run(&holder.run_id, run_dir) => RunStatus::Unknown,
        Some(holder) if is_alive(holder.pid) => RunStatus::Running,
        Some(_) => RunStatus::Crashed,
        None => RunStatus::Unknown,
    }
}

/// The run's id is the name of its directory — the host builds
/// `<cwd>/.daruda/flow-runs/<run-id>/`, so there is nothing else to compare.
fn holds_this_run(run_id: &str, run_dir: &Path) -> bool {
    run_dir.file_name().is_some_and(|name| name == run_id)
}

/// How many finished runs a sweep leaves behind when the host states no
/// preference. Design §10's default; the parameter is where a host that
/// gains a config knob expresses its own.
pub const DEFAULT_KEEP_RUNS: usize = 20;

/// Delete finished runs beyond `keep`, newest first, and report what went.
///
/// "Finished" means a completion marker is present — not `run_status`, which
/// also answers from the lock and would call a crashed run `Unknown` whenever
/// any other run is going. An unmarked directory is the only evidence a crash
/// ever happened (§10's status table derives `Crashed` from exactly that), so
/// it is never a candidate however old it is, and never counts against `keep`.
/// Reading no lock is why this takes no `is_alive`.
///
/// Newest is decided by directory name, which is chronological only because
/// the host names run directories with ULIDs (design §10 — the app makes the
/// run-id, the engine only receives `run_dir`). A host using a different id
/// scheme would silently have this delete the wrong ones.
pub fn sweep_old_runs(runs_dir: &Path, keep: usize) -> std::io::Result<Vec<PathBuf>> {
    let mut finished: Vec<PathBuf> = std::fs::read_dir(runs_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && has_marker(path))
        .collect();
    finished.sort();

    let over = finished.len().saturating_sub(keep);
    let mut removed = Vec::with_capacity(over);
    for run_dir in finished.into_iter().take(over) {
        std::fs::remove_dir_all(&run_dir)?;
        removed.push(run_dir);
    }
    Ok(removed)
}

/// Whether the run got to say how it ended. The one question retention asks
/// — and the only one it can answer without a lock.
///
/// This is also what keeps a killed run around long enough to be resumed:
/// it never wrote a marker, so the sweep never reaches it. See
/// [`crate::resume`].
fn has_marker(run_dir: &Path) -> bool {
    [DONE, FAILED, CANCELED]
        .iter()
        .any(|name| run_dir.join(name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockHolder;
    use crate::runner::NodeFailure;
    use crate::schedule::{BudgetLimit, RunOutcome};
    use std::path::{Path, PathBuf};

    fn run_dir_in(dir: &Path, run_id: &str) -> PathBuf {
        let run_dir = dir.join(run_id);
        std::fs::create_dir_all(&run_dir).expect("mkdir");
        run_dir
    }

    /// Plant a `.lock` naming a holder this process is not — the only way
    /// to stage a dead pid without killing anything.
    fn write_lock(dir: &Path, pid: u32, run_id: &str) {
        let holder = LockHolder {
            pid,
            run_id: run_id.to_string(),
            started_unix_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        std::fs::write(
            dir.join(".lock"),
            yaml_serde::to_string(&holder).expect("serialise"),
        )
        .expect("write");
    }

    /// The three a run can state about itself. Every non-success shares one
    /// marker: `run.md` (P2b-3) is where the reason belongs.
    #[test]
    fn each_outcome_a_run_can_record_writes_its_own_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cases: [(RunOutcome, &str, RunStatus); 5] = [
            (RunOutcome::Done, "DONE", RunStatus::Done),
            (
                RunOutcome::Failed {
                    node: "design".into(),
                    failure: NodeFailure::Exit { code: Some(1) },
                },
                "FAILED",
                RunStatus::Failed,
            ),
            (
                RunOutcome::BudgetExhausted {
                    limit: BudgetLimit::NodeRuns,
                },
                "FAILED",
                RunStatus::Failed,
            ),
            (
                RunOutcome::Canceled {
                    node: Some("design".into()),
                },
                "CANCELED",
                RunStatus::Canceled,
            ),
            (
                RunOutcome::Canceled { node: None },
                "CANCELED",
                RunStatus::Canceled,
            ),
        ];
        for (outcome, name, status) in cases {
            let run_dir = run_dir_in(dir.path(), name);
            write_marker(&run_dir, &outcome).expect("write");
            assert!(run_dir.join(name).is_file(), "{outcome:?}");
            assert_eq!(run_status(&run_dir, &|_| true), status);
            std::fs::remove_file(run_dir.join(name)).expect("clean");
        }
    }

    /// A run that never took the directory has nothing to say about it —
    /// stamping one would overwrite the status of the run that is going.
    #[test]
    fn a_run_that_never_took_the_lock_writes_no_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        write_marker(
            &run_dir,
            &RunOutcome::LockHeld {
                holder: LockHolder {
                    pid: 4242,
                    run_id: "other".to_string(),
                    started_unix_secs: 0,
                },
            },
        )
        .expect("write");
        assert_eq!(std::fs::read_dir(&run_dir).expect("read").count(), 0);
    }

    /// The marker's directory may not exist yet: a command-only flow writes
    /// no output, so nothing else has created it.
    #[test]
    fn a_marker_creates_its_run_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = dir.path().join("never-created");
        write_marker(&run_dir, &RunOutcome::Done).expect("write");
        assert!(run_dir.join("DONE").is_file());
    }

    #[test]
    fn no_marker_and_a_live_holder_reads_as_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        write_lock(dir.path(), 4242, "01J");
        assert_eq!(run_status(&run_dir, &|_| true), RunStatus::Running);
    }

    /// The row that cannot be reached by writing a marker: a crash leaves no
    /// record of itself, so the absence of one plus a dead holder is the only
    /// evidence there is.
    #[test]
    fn no_marker_and_a_dead_holder_reads_as_crashed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        write_lock(dir.path(), 4242, "01J");
        assert_eq!(run_status(&run_dir, &|_| false), RunStatus::Crashed);
    }

    /// Not the same as crashed. Without a lock there is no evidence either
    /// way, and saying "crashed" would be a guess presented as a fact.
    #[test]
    fn no_marker_and_no_lock_reads_as_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        assert_eq!(run_status(&run_dir, &|_| true), RunStatus::Unknown);
    }

    /// A live lock naming a different run is evidence about that run, not
    /// this one. Reading it as `Running` would report every unmarked run in
    /// the history as live for as long as any run is going.
    #[test]
    fn a_lock_held_by_another_run_says_nothing_about_this_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        write_lock(dir.path(), 4242, "01K");
        assert_eq!(run_status(&run_dir, &|_| true), RunStatus::Unknown);
    }

    /// A run that ended and said so. The id doubles as the sort key, the way
    /// a ULID does.
    fn finished_run_in(dir: &Path, run_id: &str) -> PathBuf {
        let run_dir = run_dir_in(dir, run_id);
        std::fs::write(run_dir.join(DONE), "").expect("marker");
        run_dir
    }

    /// The directory names left behind, sorted — what a user would see.
    fn surviving(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The rule that makes retention safe: a directory with no marker is the
    /// only evidence a crash ever happened.
    ///
    /// The counts matter. 25 marked — five over the cap — proves the cap is
    /// enforced at all; with exactly 20 a sweep that never ran would pass.
    /// The 3 unmarked are older than every marked one, so surviving proves
    /// they are exempt rather than merely fitting under the cap.
    #[test]
    fn a_sweep_keeps_unmarked_runs_however_old_they_are() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..3 {
            run_dir_in(dir.path(), &format!("01A{i:02}"));
        }
        for i in 0..25 {
            finished_run_in(dir.path(), &format!("01B{i:02}"));
        }

        let removed = sweep_old_runs(dir.path(), 20).expect("sweep");

        assert_eq!(removed.len(), 5, "{removed:?}");
        let left = surviving(dir.path());
        assert_eq!(left.len(), 23, "{left:?}");
        for i in 0..3 {
            let unmarked = format!("01A{i:02}");
            assert!(left.contains(&unmarked), "{unmarked} was swept: {left:?}");
        }
    }

    /// The case that catches a sweep keyed on `run_status` instead of the
    /// marker: with a live lock naming some other run, an unmarked directory
    /// reads as `Unknown` rather than `Crashed` — and any run at all being in
    /// progress is enough to produce that. The evidence of the crash must
    /// survive regardless.
    #[test]
    fn a_sweep_keeps_an_unmarked_run_while_another_run_holds_the_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let crashed = run_dir_in(dir.path(), "01A00");
        for i in 0..25 {
            finished_run_in(dir.path(), &format!("01B{i:02}"));
        }
        // Beside the run directories, which is where a `run_status`-keyed
        // sweep would look — it has only `runs_dir` to offer as the lock's
        // directory. This one reads no lock at all.
        write_lock(dir.path(), 4242, "other");
        assert_eq!(
            run_status(&crashed, &|_| true),
            RunStatus::Unknown,
            "the misreading this test exists to survive"
        );

        let removed = sweep_old_runs(dir.path(), 20).expect("sweep");

        assert!(crashed.is_dir(), "the only evidence of a crash was swept");
        assert!(!removed.contains(&crashed), "{removed:?}");
    }

    /// Newest kept, oldest dropped — and the ordering is by run-id, because
    /// the design's run-ids are ULIDs, which sort by time.
    #[test]
    fn a_sweep_keeps_the_newest_and_drops_the_oldest() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..22 {
            finished_run_in(dir.path(), &format!("01B{i:02}"));
        }

        let removed = sweep_old_runs(dir.path(), 20).expect("sweep");

        let gone: Vec<String> = removed
            .iter()
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(gone, vec!["01B00".to_string(), "01B01".to_string()]);
        assert_eq!(surviving(dir.path()).len(), 20);
        assert!(dir.path().join("01B21").is_dir(), "the newest must stay");
    }

    /// A truncated or garbage lock is not evidence of a live run — the same
    /// reading `RunLock::acquire` makes when it reclaims one.
    #[test]
    fn an_unparseable_lock_reads_as_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let run_dir = run_dir_in(dir.path(), "01J");
        std::fs::write(dir.path().join(".lock"), "{ truncated").expect("write");
        assert_eq!(run_status(&run_dir, &|_| true), RunStatus::Unknown);
    }
}
