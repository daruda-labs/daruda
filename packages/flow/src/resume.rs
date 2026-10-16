//! Picking up a run that was killed, rather than starting it over.
//!
//! Everything a resume needs is in the run directory the first process left:
//! the resolved spec in `run.yaml`, the outputs it had produced, and the run
//! log saying what happened to each node (or, for a run an older build
//! started, the `progress.jsonl` journal).
//!
//! [`prepare`] reads only what was fixed at the start, and reads it before
//! any lock. Everything that changes as a run goes is read inside `execute`,
//! after the lock is taken (F14).
//!
//! **Node-level, and only node-level.** Whatever was in flight is gone — an
//! ACP session cannot outlive the process that opened it. So the interrupted
//! node runs again from the start, as its next attempt, and its side effects
//! happen again. That is the honest bound on this feature.

use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_runlog::{LogBackend, RunLog};

use crate::NodeId;
use crate::error::{FlowError, FlowIoError, IoSite};
use crate::load::LoadedFlow;
use crate::marker::RunStatus;
use crate::run_log::line::AttemptLine;
use crate::run_log::machine::{
    AbandonReason, AttemptState, FlowRunEvent, FlowRunMachine, FlowRunState, Imported,
    ImportedAttempt, ImportedLast, ImportedNode, ImportedOutcome, Produces,
};
use crate::run_log::reader::{self, Replay};
use crate::run_log::store::FlowRunStore;
use crate::runner::OutputContract;
use crate::schedule::RunOutcome;

/// The spec a run was executing, as the run itself recorded it.
///
/// Named here rather than inlined at the call site because *which* file a
/// resume reads is the whole of decision ③: never the flow file on disk
/// today. Editing it between the crash and the resume would otherwise
/// silently change what the run is, halfway through.
pub const RUN_SPEC_FILE: &str = "run.yaml";

const RESUME: &str = "continuing the run";
const IMPORT: &str = "taking over the run's progress journal";

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
    /// What the run directory said it had finished when this was read. For
    /// display: the run itself reads it again once it holds the lock.
    pub replay: Replay,
}

impl Resumed {
    /// What the request carries into `execute`.
    pub fn continuation(&self) -> Continuation {
        Continuation {
            until: self.replay.until.clone(),
            settle: SETTLE_AFTER_CRASH,
        }
    }
}

/// How long after a dead run's last record a resume waits before it starts.
/// Measured: after the host is killed, an agent's CLI lingers as an orphan
/// for 3–10 s before it exits, and a resume should not start beside it.
pub(crate) const SETTLE_AFTER_CRASH: Duration = Duration::from_secs(10);

/// The part of a run fixed when it started, which a request to continue it
/// needs before its lock: which nodes it reaches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Continuation {
    pub until: Option<NodeId>,
    /// [`SETTLE_AFTER_CRASH`] outside the tests, which resume at once.
    pub(crate) settle: Duration,
}

/// What is left of `settle` after a record written at `last_ms`, at `now_ms`.
/// A record dated after now (a clock set back) waits the whole window, never
/// longer.
pub(crate) fn settle_wait(last_ms: u64, now_ms: u64, settle: Duration) -> Duration {
    let since = Duration::from_millis(now_ms.saturating_sub(last_ms));
    settle.saturating_sub(since)
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
        replay: reader::read(run_dir),
    })
}

/// Whether a resume may go on once the settle window is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    Go,
    /// Stopped before the run was taken. Nothing was locked or written, so
    /// the run is left resumable exactly as it was.
    Stopped,
}

/// Wait out what is left of the settle window after a dead run's last
/// record, so its agent processes exit before the run starts beside them.
/// Only for a run that stopped without ending; a stop cuts the wait short.
///
/// Before the lock, and reading only when the log was last written: a
/// stop during the wait must leave no trace. Under the lock it would have
/// to give the lock back, and a crashed run whose lock is gone reads as
/// `Unknown`, which nobody can resume.
pub(crate) fn settle_after_crash(
    run_dir: &Path,
    settle: Duration,
    events: Option<&smol::channel::Sender<crate::event::RunNotice>>,
    cancel: &crate::runner::CancelToken,
) -> Settled {
    if let Some(last_ms) = reader::last_written_while_running(run_dir) {
        wait_out(last_ms, settle, events, cancel);
    }
    // Also with no wait at all: a stop that lands before the run is taken
    // must not end, in its log, a run that never ran again.
    if cancel.is_canceled() {
        Settled::Stopped
    } else {
        Settled::Go
    }
}

fn wait_out(
    last_ms: u64,
    settle: Duration,
    events: Option<&smol::channel::Sender<crate::event::RunNotice>>,
    cancel: &crate::runner::CancelToken,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        });
    let wait = settle_wait(last_ms, now_ms, settle);
    if wait.is_zero() {
        return;
    }
    crate::event::emit(events, crate::event::RunNotice::Settling { wait });
    let until = std::time::Instant::now() + wait;
    while !cancel.is_canceled() {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}

/// A refusal found once the lock is held: nothing was recorded, so the run
/// directory is left as it was.
fn refused(run_dir: &Path, error: ResumeError) -> RunOutcome {
    RunOutcome::Io(FlowIoError {
        site: IoSite::Run,
        doing: RESUME,
        path: run_dir.to_path_buf(),
        source: std::io::Error::other(error.to_string()),
    })
}

/// Open the run's log under the lock (§5.5 steps 1–2), taking over the
/// journal of a run an older build started.
pub(crate) fn open_under_lock(
    run_dir: &Path,
    loaded: &LoadedFlow,
) -> Result<RunLog<FlowRunMachine, daruda_runlog::FileBackend>, RunOutcome> {
    if !reader::started(run_dir) {
        return Err(refused(run_dir, ResumeError::NothingStarted));
    }
    let mut log = crate::run_log::store::open_file_log(run_dir)?;
    // Decided on what the opened log holds, not on whether it exists: a
    // crash between opening it and committing the import leaves a log with
    // nothing in it, and the journal is still the run's record (O3).
    if log.state().phase() != crate::run_log::machine::RunPhase::Fresh
        || !crate::journal::exists(run_dir)
    {
        return Ok(log);
    }
    log.commit(import_journal(run_dir, loaded)).map_err(|e| {
        RunOutcome::Io(FlowIoError {
            site: IoSite::Run,
            doing: IMPORT,
            path: crate::run_log::log_dir(run_dir),
            source: std::io::Error::other(e.to_string()),
        })
    })?;
    Ok(log)
}

/// What the old journal knew, as one record. Only into an empty log.
fn import_journal(run_dir: &Path, loaded: &LoadedFlow) -> FlowRunEvent {
    let replay = crate::journal::read(run_dir);
    let records = replay
        .records
        .iter()
        .flat_map(|node| {
            node.attempts.iter().map(|attempt| ImportedAttempt {
                node: node.id.clone(),
                outcome: match &attempt.outcome {
                    crate::record::AttemptOutcome::Passed => ImportedOutcome::Passed,
                    crate::record::AttemptOutcome::Canceled => ImportedOutcome::Canceled,
                    crate::record::AttemptOutcome::Reported(reason) => ImportedOutcome::Reported {
                        reason: reason.clone(),
                    },
                    crate::record::AttemptOutcome::Failed(failure) => ImportedOutcome::Reported {
                        reason: format!("failed: {failure}"),
                    },
                },
                record: AttemptLine::new(attempt, &Default::default()),
            })
        })
        .collect();
    FlowRunEvent::Imported(Box::new(Imported {
        nodes: crate::run_log::node_starts(loaded),
        until: replay.until.clone(),
        passed: replay
            .passed
            .iter()
            .filter(|id| !replay.pinned.contains(id))
            .cloned()
            .collect(),
        pinned: replay.pinned.clone(),
        profile: replay.profile.clone(),
        next_evidence: replay.next_seq.max(1),
        spent: (&replay.spent).into(),
        records,
        carried: carried(&replay.records, &replay.passed),
    }))
}

/// Each unpassed node's settled attempts since a repair last invalidated
/// it. Counted, not read off attempt numbers: an older build's resume
/// restarted those at 1, so the numbers alone undercount.
fn carried(records: &[crate::record::NodeRecord], passed: &[NodeId]) -> Vec<ImportedNode> {
    use crate::record::AttemptOutcome;
    // The evidence number is run-wide and only rises, so an invalidation
    // splits a node's attempts into the generation before it and after.
    let mut boundary = std::collections::BTreeMap::<&NodeId, u32>::new();
    for attempt in records.iter().flat_map(|node| &node.attempts) {
        for id in &attempt.invalidated.nodes {
            let seen = boundary.entry(id).or_default();
            *seen = (*seen).max(attempt.evidence_seq);
        }
    }
    records
        .iter()
        .filter(|node| !passed.contains(&node.id))
        .filter_map(|node| {
            let since = boundary.get(&node.id).copied();
            let current: Vec<_> = node
                .attempts
                .iter()
                .filter(|attempt| since.is_none_or(|seq| attempt.evidence_seq > seq))
                .collect();
            let last = match &current.iter().max_by_key(|a| a.evidence_seq)?.outcome {
                AttemptOutcome::Canceled => ImportedLast::Canceled,
                AttemptOutcome::Reported(reason) => ImportedLast::Failed {
                    reason: reason.clone(),
                },
                AttemptOutcome::Failed(failure) => ImportedLast::Failed {
                    reason: failure.to_string(),
                },
                AttemptOutcome::Passed => return None,
            };
            Some(ImportedNode {
                node: node.id.clone(),
                attempts: u32::try_from(current.len()).unwrap_or(u32::MAX),
                last,
            })
        })
        .collect()
}

/// The rest of §5.5 once the log is open: steps 3–7. Returns the store and
/// what the scheduler starts from (step 8).
pub(crate) fn pick_up<B: LogBackend + Send + 'static>(
    log: RunLog<FlowRunMachine, B>,
    loaded: &LoadedFlow,
    run_dir: &Path,
    events: Option<smol::channel::Sender<crate::event::RunNotice>>,
    warnings: &mut Vec<String>,
) -> Result<(FlowRunStore, Replay), RunOutcome> {
    if !log.state().is_resumable() {
        let why = match log.state().phase() {
            crate::run_log::machine::RunPhase::Fresh => ResumeError::NothingStarted,
            _ => ResumeError::NotResumable(crate::marker::run_status(run_dir, None, &|_| false)),
        };
        return Err(refused(run_dir, why));
    }
    // Step 3. A marker left in place would keep saying how the run last
    // ended while it runs again (F8).
    if let Err(e) = crate::marker::clear_markers(run_dir) {
        warnings.push(e.to_string());
    }
    // Step 4. Before `Resumed` turns these attempts into interrupted ones
    // and forgets their groups.
    warnings.extend(end_orphaned_commands(log.state()));
    let store = FlowRunStore::spawn(log, run_dir, events)?;
    smol::block_on(resume_steps(&store, loaded, run_dir, warnings))?;
    let history = store.events().map_err(|e| store.failure(IoSite::Run, e))?;
    Ok((store, reader::replay_events(&history)))
}

async fn resume_steps(
    store: &FlowRunStore,
    loaded: &LoadedFlow,
    run_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), RunOutcome> {
    let record = |e| store.failure(IoSite::Run, e);
    // Step 5. The reply channels died with the process, so nothing can
    // answer these; left open they would keep the run from ever ending (B5).
    let open = store.state().open_ask_ids();
    for id in open {
        let reason = AbandonReason::AppRestart;
        store
            .commit(FlowRunEvent::AskAbandoned { id, reason })
            .await
            .map_err(record)?;
    }
    let carried = store.state().finished().len();
    store
        .commit(FlowRunEvent::Resumed { carried })
        .await
        .map_err(record)?;

    // Step 6. A pass is only as good as the file it left; directory entries
    // may not survive a power cut, so check (F4). Commands and pins leave
    // nothing to check.
    let outputs = crate::schedule::node_outputs(loaded.flow(), run_dir);
    for node in &loaded.flow().nodes {
        if !output_lost(store, run_dir, node) {
            continue;
        }
        store
            .commit(FlowRunEvent::OutputLost {
                node: node.id.clone(),
            })
            .await
            .map_err(record)?;
    }

    // Step 7. Whatever an unfinished or invalidated node left live is
    // evidence, not a result (F3).
    let passed: Vec<NodeId> = store.state().finished().into_iter().collect();
    let resumes = store.state().resumes;
    let log_dir = run_dir.join(crate::schedule::LOG_DIR_NAME);
    warnings.extend(archive_unclaimed_outputs(
        &log_dir, &outputs, &passed, resumes,
    ));
    Ok(())
}

/// End the command groups a dead process left running (F7). A command
/// survives its host in a group of its own, and the retry would otherwise
/// run beside it. A pid whose identity no longer matches is someone else's.
fn end_orphaned_commands(state: &FlowRunState) -> Vec<String> {
    state
        .unsettled_processes()
        .into_iter()
        .filter_map(|(node, process)| {
            let identity = daruda_core::process::ProcessIdentity::from_raw(process.identity);
            daruda_core::process::end_recorded_group(process.leader, identity)
                .err()
                .map(|e| {
                    format!(
                        "`{node}`'s command from before the crash could not be stopped, so it \
                         may still be running beside its retry: {e}"
                    )
                })
        })
        .collect()
}

/// Whether a passed agent node's output no longer meets its contract.
fn output_lost(store: &FlowRunStore, run_dir: &Path, node: &crate::model::Node) -> bool {
    let passed = store.state().node(&node.id).is_some_and(|state| {
        state.produces == Produces::File
            && !state.invalidated
            && matches!(state.last, AttemptState::Passed { .. })
    });
    if !passed {
        return false;
    }
    let Some(output) = crate::schedule::node_output(node, run_dir) else {
        return false;
    };
    let contract = crate::contract::file::FileContract::new(
        run_dir,
        &output,
        node.kind.output_schema(),
        node.kind.continue_until(),
    );
    contract.check().is_err()
}

/// Move aside whatever an interrupted or invalidated node had written.
///
/// A node that did not settle may have left a half-written output, and
/// `judge` cannot tell that from a good one — it only asks whether the file
/// is there and non-empty. Every live output that no pass claims is
/// therefore evidence, not a result.
///
/// Best-effort, and it reports rather than fails: the paths it could not
/// move become warnings, because a resume that refuses over a stale file is
/// worse than one that says which file is stale.
pub(crate) fn archive_unclaimed_outputs(
    log_dir: &Path,
    outputs: &[(NodeId, PathBuf)],
    passed: &[NodeId],
    resume: u32,
) -> Vec<String> {
    let mut warnings = Vec::new();
    for (id, output) in outputs {
        // Asked without following links, so all three link shapes reach the
        // archive's refusal: `is_file` is false for a dangling one and for
        // one to a directory, which would skip both without a word.
        if passed.contains(id) || std::fs::symlink_metadata(output).is_err() {
            continue;
        }
        let stamp = crate::archive::CancelStamp::Resume(resume);
        match crate::archive::archive_canceled(log_dir, id, output, stamp) {
            Ok(_) => {}
            Err(e) => warnings.push(format!(
                "`{id}` left a partial output that could not be moved aside, so it may be \
                 mistaken for a finished one: {e}"
            )),
        }
    }
    warnings
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

    /// The wait is what is left of the window after the dead run's last
    /// record; a clock set back never stretches it past the window.
    #[test]
    fn the_settle_wait_is_what_is_left_of_the_window() {
        let window = Duration::from_secs(10);
        assert_eq!(settle_wait(1_000, 4_000, window), Duration::from_secs(7));
        assert_eq!(settle_wait(1_000, 11_000, window), Duration::ZERO);
        assert_eq!(settle_wait(1_000, 90_000, window), Duration::ZERO);
        assert_eq!(settle_wait(9_000, 1_000, window), window);
        assert_eq!(settle_wait(1_000, 4_000, Duration::ZERO), Duration::ZERO);
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
            &log_dir,
            &[
                ("design".into(), done.clone()),
                ("half".into(), partial.clone()),
            ],
            &["design".into()],
            1,
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
            daruda_core::path::symlink(&target, &output).expect("symlink");

            let warnings =
                archive_unclaimed_outputs(&log_dir, &[(name.into(), output.clone())], &[], 1);

            assert_eq!(warnings.len(), 1, "{name}: {warnings:?}");
            assert!(warnings[0].contains(name), "{name}: {warnings:?}");
            assert!(
                std::fs::symlink_metadata(&output).is_ok(),
                "{name}: a refused link is left where a reader can see it"
            );
            daruda_core::path::remove_symlink(&output).expect("unlink");
        }
    }

    fn journaled(
        seq: u32,
        outcome: crate::record::AttemptOutcome,
        invalidated: &[&str],
    ) -> crate::record::AttemptRecord {
        crate::record::AttemptRecord {
            attempt: 1,
            evidence_seq: seq,
            outcome,
            invalidated: crate::record::Invalidation {
                nodes: invalidated.iter().map(|id| NodeId::from(*id)).collect(),
                archived: Vec::new(),
            },
            git_status: None,
            at: std::time::UNIX_EPOCH,
            took: Duration::ZERO,
            waited: crate::runner::Waiting::default(),
            tools: Vec::new(),
            turns: 1,
            usage: None,
        }
    }

    /// A repair's invalidation starts the member's count over: only the
    /// attempts after it are carried, and a passed node carries nothing.
    #[test]
    fn only_the_current_generations_attempts_are_carried() {
        use crate::record::{AttemptOutcome, NodeRecord};
        let failed = || AttemptOutcome::Reported("exited with status 1".into());
        let records = vec![
            NodeRecord {
                id: "review".into(),
                attempts: vec![
                    journaled(1, failed(), &[]),
                    journaled(4, AttemptOutcome::Canceled, &[]),
                ],
            },
            NodeRecord {
                id: "gate".into(),
                attempts: vec![journaled(3, failed(), &["review", "gate"])],
            },
            NodeRecord {
                id: "design".into(),
                attempts: vec![journaled(2, AttemptOutcome::Passed, &[])],
            },
        ];

        let carried = carried(&records, &["design".into()]);

        assert_eq!(
            carried,
            vec![ImportedNode {
                node: "review".into(),
                attempts: 1,
                last: ImportedLast::Canceled,
            }]
        );
    }
}
