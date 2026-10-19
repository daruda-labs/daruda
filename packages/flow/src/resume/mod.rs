//! Picking up a run that was killed, rather than starting it over.
//!
//! Everything a resume needs is in the run directory the first process left:
//! the resolved spec in `run.yaml`, the outputs it had produced, and the run
//! log saying what happened to each node (or, for a run an older build
//! started, the `progress.jsonl` journal).
//!
//! [`prepare`] reads only what was fixed at the start, and reads it before
//! any lock. Everything that changes as a run goes is read inside `execute`,
//! after the lock is taken.
//!
//! **Node-level, and only node-level.** Whatever was in flight is gone — an
//! ACP session cannot outlive the process that opened it. So the interrupted
//! node runs again from the start, as its next attempt, and its side effects
//! happen again. That is the honest bound on this feature.

mod import;
mod pick_up;
mod settle;

use std::path::Path;
use std::time::Duration;

use crate::NodeId;
use crate::error::FlowError;
use crate::load::LoadedFlow;
use crate::marker::RunStatus;
use crate::run_log::reader;

pub(crate) use import::open_under_lock;
pub(crate) use pick_up::pick_up;
pub(crate) use settle::{SETTLE_AFTER_CRASH, Settled, settle_after_crash};

/// The spec a run was executing, as the run itself recorded it.
///
/// Named here rather than inlined at the call site because *which* file a
/// resume reads matters: never the flow file on disk today. Editing it
/// between the crash and the resume would otherwise silently change what
/// the run is, halfway through.
pub const RUN_SPEC_FILE: &str = "run.yaml";

/// Whether a run may be picked up.
///
/// A run that stopped without deciding to: `Crashed` (no marker, holder
/// gone) and `Stalled` (nothing in flight, journal whole). Failed and
/// canceled ended the way their policy said to, and continuing one of
/// those is a different verb. One function so the engine and the host
/// cannot disagree about what the button means.
pub fn is_resumable(status: RunStatus) -> bool {
    matches!(status, RunStatus::Crashed | RunStatus::Stalled)
}

/// What a resumed run starts from, as read before its lock.
pub struct Resumed {
    /// The flow as the earlier process resolved it, read back from
    /// `run.yaml`.
    pub loaded: LoadedFlow,
    /// The node the run was started up to. Only facts fixed at the start
    /// are read before the lock; the progress is read under it.
    pub until: Option<NodeId>,
}

impl Resumed {
    /// What the request carries into `execute`.
    pub fn continuation(&self) -> Continuation {
        Continuation {
            until: self.until.clone(),
            settle: SETTLE_AFTER_CRASH,
        }
    }
}

/// The part of a run fixed when it started, which a request to continue it
/// needs before its lock: which nodes it reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Continuation {
    pub until: Option<NodeId>,
    /// [`SETTLE_AFTER_CRASH`] outside the tests, which resume at once.
    pub(crate) settle: Duration,
}

/// Why a run directory cannot be picked up.
#[derive(Debug)]
pub enum ResumeError {
    /// The run ended on purpose, or is still going.
    NotResumable(RunStatus),
    /// No record at all: the crash landed during setup, before the first
    /// node. There is nothing to continue — the run has to be started
    /// again from the flow file.
    NothingStarted,
    /// `run.yaml` is missing or unreadable — without the spec there is no
    /// way to know what the run was.
    Spec(String),
    /// The spec is there but no longer loads under this build.
    Load(FlowError),
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeError::NotResumable(status) => {
                write!(
                    f,
                    "this run is `{status:?}`, which is not a run to continue"
                )
            }
            ResumeError::NothingStarted => {
                write!(
                    f,
                    "this run stopped before its first node, so there is nothing to continue"
                )
            }
            ResumeError::Spec(detail) => {
                write!(f, "its `{RUN_SPEC_FILE}` cannot be read: {detail}")
            }
            ResumeError::Load(e) => write!(f, "its `{RUN_SPEC_FILE}` no longer loads: {e}"),
        }
    }
}

/// Read a killed run's directory back into something runnable.
///
/// `is_alive` is the host's, the same predicate the lock and the panel use
/// — the engine never asks the OS about a pid.
///
/// `lock_dir` is where this tree's lock lives ([`crate::lock::lock_dir_for`]).
/// Naming the wrong one makes every run look `Unknown`, which is to say
/// unresumable — see [`crate::marker::run_status`].
pub fn prepare(
    run_dir: &Path,
    lock_dir: Option<&Path>,
    is_alive: &dyn Fn(u32) -> bool,
) -> Result<Resumed, ResumeError> {
    let status = crate::marker::run_status(run_dir, lock_dir, is_alive);
    if !is_resumable(status) {
        return Err(ResumeError::NotResumable(status));
    }
    if !reader::started(run_dir) {
        return Err(ResumeError::NothingStarted);
    }
    let text = std::fs::read_to_string(run_dir.join(RUN_SPEC_FILE))
        .map_err(|e| ResumeError::Spec(e.to_string()))?;
    // `None`: the profile was folded in when this spec was written, and
    // `run.yaml` deliberately carries no profile list to choose from. The
    // name the run went by comes back from its record instead.
    let loaded = crate::load::load(&text, None).map_err(ResumeError::Load)?;
    Ok(Resumed {
        loaded,
        until: reader::selection(run_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two statuses a run can be picked up from, and the five it
    /// cannot. `Stalled` is the one worth spelling out — the only *written*
    /// marker on the resumable side, so sorting by "did it record an
    /// ending" takes the button away from a run that lost nothing.
    #[test]
    fn a_killed_or_stalled_run_is_one_to_continue() {
        for pickable in [RunStatus::Crashed, RunStatus::Stalled] {
            assert!(is_resumable(pickable), "{pickable:?}");
        }
        for ended in [
            RunStatus::Done,
            RunStatus::Failed,
            RunStatus::Canceled,
            RunStatus::Running,
            RunStatus::Unknown,
        ] {
            assert!(!is_resumable(ended), "{ended:?}");
        }
    }

    /// A run directory with a live lock and no journal is not resumable for
    /// two separate reasons; the status is checked first because "it is
    /// still going" is the more useful thing to say.
    #[test]
    fn a_run_that_is_still_going_is_refused_before_its_journal_is_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runs = dir.path().join("runs");
        let run_dir = runs.join("01J");
        std::fs::create_dir_all(&run_dir).expect("mkdir");
        crate::lock::RunLock::acquire(&runs, "01J", &|_| true).expect("lock");

        assert!(matches!(
            prepare(&run_dir, Some(&runs), &|_| true),
            Err(ResumeError::NotResumable(RunStatus::Running))
        ));
    }

    /// **The guard for the lock's move out of the working tree.**
    ///
    /// Asserts the whole path a resume takes — the lock where the engine
    /// now puts it, read back through `prepare`. Nothing else catches a
    /// caller handing `run_status` the wrong directory.
    #[test]
    fn a_crashed_run_is_still_resumable_with_the_lock_outside_the_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tree = dir.path().join("tree");
        let run_dir = tree.join(".daruda/flow-runs/01J");
        std::fs::create_dir_all(&run_dir).expect("mkdir");
        let tree = crate::lock::CanonicalTree::resolve(&tree).expect("the tree resolves");
        let lock_dir = crate::lock::lock_dir_for(&dir.path().join("locks"), &tree);
        std::fs::create_dir_all(&lock_dir).expect("mkdir");
        // Left behind by a process that is gone, the way `RunLock` writes it.
        std::mem::forget(crate::lock::RunLock::acquire(&lock_dir, "01J", &|_| true).expect("lock"));

        assert!(
            matches!(
                prepare(&run_dir, Some(&lock_dir), &|_| false),
                Err(ResumeError::NothingStarted)
            ),
            "a crashed run must reach the journal check, not stop at `Unknown`"
        );
        assert_eq!(
            crate::marker::run_status(&run_dir, Some(&lock_dir), &|_| false),
            RunStatus::Crashed,
            "the lock outside the tree is what says the run crashed"
        );
    }

    /// A crash during setup leaves a lock and no journal. There is nothing
    /// to continue, and saying so is different from saying the run ended.
    #[test]
    fn a_crash_before_the_first_node_has_nothing_to_continue() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runs = dir.path().join("runs");
        let run_dir = runs.join("01J");
        std::fs::create_dir_all(&run_dir).expect("mkdir");
        // The lock file a killed process leaves behind, written the way
        // `RunLock` writes it — the holder is this test's own pid, which
        // the `is_alive` below then reports as gone.
        std::mem::forget(crate::lock::RunLock::acquire(&runs, "01J", &|_| true).expect("lock"));

        assert!(matches!(
            prepare(&run_dir, Some(&runs), &|_| false),
            Err(ResumeError::NothingStarted)
        ));
    }
}
