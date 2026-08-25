//! Picking up a run that was killed, rather than starting it over.
//!
//! Everything a resume needs is in the run directory the first process left:
//! the resolved spec in `run.yaml`, the outputs it had produced, and the
//! journal saying which nodes those belong to. This module is what turns
//! that directory back into something [`crate::schedule::execute`] can be
//! handed.
//!
//! **Node-level, and only node-level.** Whatever was in flight is gone — an
//! ACP session cannot outlive the process that opened it, and a command
//! node's process group dies with it. So the interrupted node runs again
//! from the start, and its side effects happen again. That is the honest
//! bound on this feature, not an implementation gap to close later.

use std::path::{Path, PathBuf};

use crate::error::FlowError;
use crate::journal::{self, Replay};
use crate::load::LoadedFlow;
use crate::marker::RunStatus;

/// The spec a run was executing, as the run itself recorded it.
///
/// Named here rather than inlined at the call site because *which* file a
/// resume reads is the whole of decision ③: never the flow file on disk
/// today. Editing it between the crash and the resume would otherwise
/// silently change what the run is, halfway through.
pub const RUN_SPEC_FILE: &str = "run.yaml";

/// Whether a run may be picked up.
///
/// Only a run that was **killed**: no marker, and the lock's holder is
/// gone. A run that failed or was canceled ended the way its policy said
/// to, and continuing one of those is a different verb — offering both
/// behind one button would leave nobody able to predict which happened.
///
/// One function so the engine and the host cannot disagree about what the
/// button means, and so widening it later is one edit.
pub fn is_resumable(status: RunStatus) -> bool {
    matches!(status, RunStatus::Crashed)
}

/// What a resumed run starts from.
pub struct Resumed {
    /// The flow as the earlier process resolved it, read back from
    /// `run.yaml`.
    pub loaded: LoadedFlow,
    /// What that process had finished.
    pub replay: Replay,
}

/// Why a run directory cannot be picked up.
#[derive(Debug)]
pub enum ResumeError {
    /// The run ended on purpose, or is still going.
    NotResumable(RunStatus),
    /// No journal at all: the crash landed during setup, before the first
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
pub fn prepare(run_dir: &Path, is_alive: &dyn Fn(u32) -> bool) -> Result<Resumed, ResumeError> {
    let status = crate::marker::run_status(run_dir, is_alive);
    if !is_resumable(status) {
        return Err(ResumeError::NotResumable(status));
    }
    if !journal::exists(run_dir) {
        return Err(ResumeError::NothingStarted);
    }
    let text = std::fs::read_to_string(run_dir.join(RUN_SPEC_FILE))
        .map_err(|e| ResumeError::Spec(e.to_string()))?;
    // `None`: the profile was folded in when this spec was written, and
    // `run.yaml` deliberately carries no profile list to choose from. The
    // name the run went by comes back from the journal instead.
    let loaded = crate::load::load(&text, None).map_err(ResumeError::Load)?;
    Ok(Resumed {
        loaded,
        replay: journal::read(run_dir),
    })
}

/// Move aside whatever the interrupted node had written.
///
/// A node that did not settle may have left a half-written output, and
/// `judge` cannot tell that from a good one — it only asks whether the file
/// is there and non-empty. Every live output that no journal line claims is
/// therefore evidence, not a result.
///
/// Best-effort, and it reports rather than fails: the paths it could not
/// move become warnings, because a resume that refuses over a stale file is
/// worse than one that says which file is stale.
pub(crate) fn archive_unclaimed_outputs(
    run_dir: &Path,
    log_dir: &Path,
    outputs: &[(crate::NodeId, PathBuf)],
    passed: &[crate::NodeId],
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, output) in outputs {
        // Asked without following links, so all three link shapes reach the
        // archive's refusal: `is_file` is false for a dangling one and for
        // one to a directory, which would skip both without a word.
        if passed.contains(id) || std::fs::symlink_metadata(output).is_err() {
            continue;
        }
        match crate::archive::archive_canceled(log_dir, id, output) {
            Ok(_) => {}
            Err(e) => warnings.push(format!(
                "`{id}` left a partial output that could not be moved aside, so it may be \
                 mistaken for a finished one: {e}"
            )),
        }
    }
    let _ = run_dir;
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three statuses that are not a killed run, and the one that is.
    /// Stated as a table because the whole of decision ① is which of these
    /// gets a button.
    #[test]
    fn only_a_killed_run_is_one_to_continue() {
        assert!(is_resumable(RunStatus::Crashed));
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
            prepare(&run_dir, &|_| true),
            Err(ResumeError::NotResumable(RunStatus::Running))
        ));
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
            prepare(&run_dir, &|_| false),
            Err(ResumeError::NothingStarted)
        ));
    }

    /// The output of a node that never settled is moved aside. Left live,
    /// `judge` would accept a half-written file as that node's result — the
    /// one way a resume can produce a wrong answer rather than a slow one.
    #[test]
    fn a_partial_output_from_the_interrupted_node_is_moved_aside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let done = dir.path().join("design.md");
        let partial = dir.path().join("half.md");
        std::fs::write(&done, "finished\n").expect("write");
        std::fs::write(&partial, "half a th").expect("write");

        let warnings = archive_unclaimed_outputs(
            dir.path(),
            &log_dir,
            &[
                ("design".into(), done.clone()),
                ("half".into(), partial.clone()),
            ],
            &["design".into()],
        );

        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(done.is_file(), "a passed node's output was taken away");
        assert!(!partial.exists(), "the partial output is still live");
    }

    /// A link where an output belongs is residue too, and every shape of one
    /// has to say so: on `is_file` a dangling link and one to a directory are
    /// both indistinguishable from nothing written, so the sweep passed over
    /// them and left them live for `judge` to read.
    #[test]
    fn every_shape_of_link_left_behind_is_reported_rather_than_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_dir = dir.path().join("logs");
        let elsewhere = dir.path().join("elsewhere.md");
        std::fs::write(&elsewhere, "someone else's work\n").expect("write");
        let a_directory = dir.path().join("a-directory");
        std::fs::create_dir_all(&a_directory).expect("mkdir");

        for (name, target) in [
            ("to_a_file", elsewhere.clone()),
            ("dangling", dir.path().join("never-written.md")),
            ("to_a_directory", a_directory.clone()),
        ] {
            let output = dir.path().join(format!("{name}.md"));
            std::os::unix::fs::symlink(&target, &output).expect("symlink");

            let warnings = archive_unclaimed_outputs(
                dir.path(),
                &log_dir,
                &[(name.into(), output.clone())],
                &[],
            );

            assert_eq!(warnings.len(), 1, "{name}: {warnings:?}");
            assert!(warnings[0].contains(name), "{name}: {warnings:?}");
            assert!(
                std::fs::symlink_metadata(&output).is_ok(),
                "{name}: a refused link is left where a reader can see it"
            );
            std::fs::remove_file(&output).expect("unlink");
        }
    }
}
