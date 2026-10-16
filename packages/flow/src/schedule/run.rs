//! One run end to end: take the working directory, drive the graph, record
//! how it ended, give the directory back.

use super::{RunInputs, RunOutcome, RunReport, run_flow};
use crate::error::{FlowIoError, IoSite};
use crate::event::{FlowEvent, RunEnd, RunNotice, Sequenced, emit};
use crate::lock::{LockError, RunLock};
use crate::marker::{DEFAULT_KEEP_RUNS, sweep_old_runs, write_marker};
use crate::model::Flow;
use crate::request::RunRequest;
use crate::run_log::line::EndLine;
use crate::run_log::machine::FlowRunEvent;
use crate::run_log::reader::Replay;
use crate::run_log::store::FlowRunStore;
use crate::runner::{CancelToken, NodeRunner};
use daruda_acp::LaunchSpec;
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::Path;

/// The resolved spec, left in the run directory as a flow file that would
/// produce the same run again.
const RUN_YAML: &str = "run.yaml";
const WRITE_SPEC: &str = "recording the resolved spec";

/// What the run did, left beside the marker that says it is over.
use crate::record::RUN_REPORT_FILE as RUN_MD;
const WRITE_RECORD: &str = "recording what the run did";

/// Keeps every run's artifacts out of the user's `git status`. The ignore
/// file lives inside the runs directory rather than at `.daruda/`, so that the
/// visibility of the `task-*.md` files already living there is not silently
/// changed.
const GITIGNORE: &str = ".gitignore";
const GITIGNORE_BODY: &str = "*\n";
const WRITE_GITIGNORE: &str = "hiding the run directory from git";
const SWEEP_RUNS: &str = "clearing out old run directories";
const START_LOG: &str = "starting the run's progress log";
/// Resolving the working tree so its lock can be named. A tree that
/// cannot be resolved is one this run cannot be sure it excludes anything
/// in.
const RESOLVE_TREE: &str = "resolving the working tree";

/// The whole of one run: take the lock, drive the graph, record how it
/// ended, let go. `run_flow` is the middle third and stays callable on its
/// own — every scheduler test drives that directly, with no filesystem
/// ceremony around it.
///
/// Blocking rather than `async`: the drive future is `!Send`, so no host
/// executor can take it anyway, and the host runs this on a thread it owns.
pub fn execute(request: &RunRequest, runner: &dyn NodeRunner, cancel: &CancelToken) -> RunReport {
    execute_with(request, runner, cancel, &|id, _| {
        runner.prepare_agent(id, cancel)
    })
}

/// Preparing one agent's runtime, by catalog id and launch spec. Injected so
/// the tests below can state *which* agents get prepared and *when* without
/// performing it: the real one downloads a Node.js runtime on a cold cache,
/// and a test that did that would be neither fast nor honest.
type Provision<'a> = dyn Fn(&str, &LaunchSpec) -> Result<Vec<String>, String> + 'a;

fn execute_with(
    request: &RunRequest,
    runner: &dyn NodeRunner,
    cancel: &CancelToken,
    provision: &Provision<'_>,
) -> RunReport {
    // Before anything is created or taken. The design calls this stage
    // "at submission", and a host is expected to run it and show the issues
    // — but nothing forced that, so a host that forgot got a run that took a
    // lock and built directories from paths it had already rejected.
    let issues = crate::request::validate_request(request);
    if !issues.is_empty() {
        return not_started(request, RunOutcome::Invalid { issues });
    }

    // Outside the working tree, under the host's lock root.
    //
    // INVARIANT: never inside the tree it guards. `git clean -fdx` deletes
    // ignored files, so an agent tidying its worktree could remove the lock
    // while the run still held it, and `take` succeeds on a missing file.
    // Resolved, because two spellings must not become two locks; an
    // unresolvable tree is refused rather than guessed at.
    let tree = match crate::lock::CanonicalTree::resolve(&request.cwd) {
        Ok(tree) => tree,
        Err(source) => {
            return not_started(
                request,
                RunOutcome::Io(FlowIoError {
                    site: IoSite::Run,
                    doing: RESOLVE_TREE,
                    path: request.cwd.clone(),
                    source,
                }),
            );
        }
    };
    let lock_dir = crate::lock::lock_dir_for(&request.lock_dir, &tree);
    let lock = match RunLock::acquire(&lock_dir, &run_id_of(&request.run_dir), &*request.is_alive) {
        Ok(lock) => HeldLock::new(lock),
        // Neither refusal took the directory, so neither writes a marker
        // and neither releases: the run that is going owns both.
        Err(LockError::Held(holder)) => {
            return not_started(request, RunOutcome::LockHeld { holder });
        }
        Err(LockError::Io(e)) => return not_started(request, RunOutcome::Io(e)),
    };

    // Past the lock, so no other run's directory is touched, and before the
    // first node, so a run that never finishes is still hidden from git and
    // still counted for retention.
    let mut setup_warnings = prepare_runs_dir(request.run_dir.parent());

    // Only past the lock is the log opened: a continuation reads what the
    // run had finished here, never before the lock (F14).
    let begun = match &request.resume {
        None => begin(request, &mut setup_warnings),
        Some(_) => continue_run(request, cancel, &mut setup_warnings),
    };
    let Begun {
        store,
        pinned,
        resume,
    } = match begun {
        Ok(begun) => begun,
        // Nothing was recorded, so the directory is left for the next try.
        Err(outcome) => {
            let mut report = not_started(request, outcome);
            report.warn_from_setup(setup_warnings);
            if let Err(e) = lock.release() {
                report.warn(e.to_string());
            }
            return report;
        }
    };

    // Last of the setup steps, so a run that cannot be provisioned still
    // leaves the spec that says what it was going to do — and so a download
    // does not delay hiding the directory from git.
    let mut report = match provision_agents(request, provision, cancel, &mut setup_warnings) {
        Ok(()) => smol::block_on(run_flow(
            RunInputs {
                loaded: &request.loaded,
                flow_dir: &request.flow_dir,
                cwd: &request.cwd,
                run_dir: &request.run_dir,
                cancel,
                budget: &request.budget,
                git_status: request.git_status.clone(),
                log: &store,
                ask: request.ask.as_ref(),
                until: request.effective_until().cloned(),
                pinned,
                resume,
            },
            runner,
        )),
        Err(outcome) => not_started(request, outcome),
    };

    // In front, because they happened first — all are setup steps that ran
    // before the first node. None of them touches `RunOutcome`: a run
    // directory that could not be tidied or audited still ran.
    report.warn_from_setup(setup_warnings);
    // The log first: the marker below is only its cache (F8).
    let ended = match smol::block_on(record_end(&store, &report.outcome)) {
        Ok(seq) => seq,
        Err(e) => {
            report.warn(e);
            None
        }
    };
    // Before the marker, and after the warning above so the record carries
    // it: the marker is the "it is all over" signal, and a reader that acts
    // on it must not find a finished run with no account of itself.
    if let Err(e) = write_run_md(&report.run_dir, &report) {
        report.warn(e.to_string());
    }
    // Both of these run on every exit path, and neither replaces the
    // outcome: the run has already ended and that is what the user needs.
    if let Err(e) = write_marker(&report.run_dir, &report.outcome) {
        report.warn(e.to_string());
    }
    // The committer drains and stops here, so nothing writes this run's
    // log once another process may take the lock.
    drop(store);
    // After the marker, never before: in the window between a freed lock
    // and an unwritten marker a reader sees neither and calls a finished
    // run `Unknown`. A leaked lock is recovered by the next run's reclaim.
    if let Err(e) = lock.release() {
        report.warn(e.to_string());
    }
    // Last, after the marker: a host that reacts to this by opening the run
    // directory would otherwise find no marker and read a finished run as
    // `Unknown`. The end carries why, not just that.
    let end = RunEnd::from(&report.outcome);
    emit(
        request.events.as_ref(),
        match ended {
            Some(seq) => RunNotice::Recorded(Sequenced {
                seq,
                event: FlowEvent::RunEnded { end },
            }),
            None => RunNotice::Unrecorded(end),
        },
    );
    report
}

/// What a started run hands the scheduler.
struct Begun {
    store: FlowRunStore,
    /// Nodes reused rather than run.
    pinned: Vec<crate::NodeId>,
    /// What an earlier process finished, for a continuation.
    resume: Option<Replay>,
}

/// A fresh run: pins, spec and an empty log holding `RunStarted`.
fn begin(request: &RunRequest, warnings: &mut Vec<String>) -> Result<Begun, RunOutcome> {
    // First, before anything is written over: a directory that already
    // holds a run is that run's, and starting another there would mix them.
    if crate::run_log::reader::has_log(&request.run_dir) {
        return Err(RunOutcome::Io(FlowIoError {
            site: IoSite::Run,
            doing: START_LOG,
            path: crate::run_log::log_dir(&request.run_dir),
            source: std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "this run directory already holds a run",
            ),
        }));
    }
    let (pinned, pin_warnings) = copy_pinned_outputs(request);
    warnings.extend(pin_warnings);
    // After the lock and before the first node: earlier would write into a
    // directory another run owns, later would leave a crashed run — the one
    // whose settings someone needs — with no spec at all.
    if let Err(e) = write_run_yaml(&request.run_dir, request.loaded.flow(), &request.flow_dir) {
        warnings.push(e.to_string());
    }
    let log = crate::run_log::store::open_file_log(&request.run_dir)?;
    let store = FlowRunStore::spawn(log, &request.run_dir, request.events.clone())?;
    // Its presence is what tells a later resume that the crash was not in
    // setup. Narrated as `RunStarted`.
    let started = FlowRunEvent::RunStarted {
        nodes: crate::run_log::node_starts(&request.loaded),
        until: request.until.clone(),
        pinned: pinned.clone(),
        profile: request.loaded.flow().profile.clone(),
    };
    smol::block_on(store.commit(started)).map_err(|e| store.failure(IoSite::Run, e))?;
    Ok(Begun {
        store,
        pinned,
        resume: None,
    })
}

/// A continuation: the log replayed and tidied under the lock (§5.5).
fn continue_run(
    request: &RunRequest,
    cancel: &CancelToken,
    warnings: &mut Vec<String>,
) -> Result<Begun, RunOutcome> {
    let log = crate::resume::open_under_lock(&request.run_dir, &request.loaded)?;
    let settle = request
        .resume
        .as_ref()
        .map(|c| c.settle)
        .unwrap_or_default();
    crate::resume::settle_after_crash(&log, settle, request.events.as_ref(), cancel);
    let (store, replay) = crate::resume::pick_up(
        log,
        &request.loaded,
        &request.run_dir,
        request.events.clone(),
        warnings,
    )?;
    Ok(Begun {
        store,
        // Read back from the record, where pins are kept apart from passes so
        // it can still say which nodes were reused rather than run.
        pinned: replay.pinned.clone(),
        resume: Some(replay),
    })
}

/// Close what is still open and commit how the run ended. `None` for a run
/// whose end is not the log's to record; `Err` is a warning.
pub(super) async fn record_end(
    store: &FlowRunStore,
    outcome: &RunOutcome,
) -> Result<Option<u64>, String> {
    let Some(end) = EndLine::of(outcome) else {
        return Ok(None);
    };
    let unrecorded = |e: crate::run_log::store::FlowCommitError| {
        format!("how this run ended could not be recorded, so it may read as unfinished: {e}")
    };
    // A question whose wait was cut short can still be open; a run cannot
    // end with one (B5).
    let open = store.state().open_ask_ids();
    for id in open {
        let reason = crate::run_log::machine::AbandonReason::RunEnded;
        let abandoned = store
            .commit(FlowRunEvent::AskAbandoned { id, reason })
            .await;
        match abandoned {
            Ok(_) | Err(daruda_runlog::CommitError::Rejected(_)) => {}
            Err(e) => return Err(unrecorded(e)),
        }
    }
    store
        .commit(FlowRunEvent::RunEnded { end })
        .await
        .map(|committed| Some(committed.seq))
        .map_err(unrecorded)
}

/// The run's lock, given back however the run thread ends. A panic used to
/// leave it naming this live process until the app restarted (F5).
struct HeldLock(Option<RunLock>);

impl HeldLock {
    fn new(lock: RunLock) -> Self {
        Self(Some(lock))
    }

    fn release(mut self) -> Result<(), FlowIoError> {
        match self.0.take() {
            Some(lock) => lock.release(),
            None => Ok(()),
        }
    }
}

impl Drop for HeldLock {
    fn drop(&mut self) {
        if let Some(lock) = self.0.take() {
            // SILENT-OK: only reached while unwinding, where there is no
            // report left to carry the failure; a leaked lock is reclaimed.
            let _ = lock.release();
        }
    }
}

/// Prepare every runtime this run could need, before the first node — a
/// first-run download inside a node's turn eats that node's budget, and the
/// node that pays is whichever happened to be first.
///
/// Distinct by catalog id, in flow order, and `default_agent` counts: a repair
/// opens a real session with it, in flows where no node names an agent at all.
/// An id with no launch spec is left alone — `validate_request` already
/// rejects it, and failing the whole run here would stop a flow over a repair
/// that may never happen.
fn provision_agents(
    request: &RunRequest,
    provision: &Provision<'_>,
    cancel: &CancelToken,
    notices: &mut Vec<String>,
) -> Result<(), RunOutcome> {
    let mut prepared = HashSet::new();
    for id in request.selected_agents() {
        if cancel.is_canceled() {
            return Err(RunOutcome::Canceled { node: None });
        }
        if !prepared.insert(id) {
            continue;
        }
        let Some(launch) = request.agents.get(id) else {
            continue;
        };
        match provision(id, launch) {
            Ok(warnings) => notices.extend(warnings),
            Err(_) if cancel.is_canceled() => return Err(RunOutcome::Canceled { node: None }),
            Err(message) => {
                return Err(RunOutcome::Unprovisioned {
                    agent: id.to_string(),
                    message,
                });
            }
        }
    }
    Ok(())
}

/// Set the directory this run's siblings live in up: hide it from git, then
/// clear out the runs that have piled up in it. Both fail into warnings, in
/// the order they happened.
///
/// `None` is a run directory with no parent, which the host cannot produce —
/// it builds `<cwd>/.daruda/flow-runs/<run-id>/` — and which has no runs
/// directory to prepare.
fn prepare_runs_dir(runs_dir: Option<&Path>) -> Vec<String> {
    let Some(runs_dir) = runs_dir else {
        return Vec::new();
    };
    let mut warnings = Vec::new();
    if let Err(e) = write_gitignore(runs_dir) {
        warnings.push(e.to_string());
    }
    // Second, so the directory the sweep reads exists even on the first run.
    if let Err(source) = sweep_old_runs(runs_dir, DEFAULT_KEEP_RUNS) {
        warnings.push(
            FlowIoError {
                site: IoSite::Run,
                doing: SWEEP_RUNS,
                path: runs_dir.to_path_buf(),
                source,
            }
            .to_string(),
        );
    }
    warnings
}

/// Write the runs directory's `.gitignore`, once. An existing one is left
/// alone — the user may have edited it, and rewriting every run would undo
/// that.
fn write_gitignore(runs_dir: &Path) -> Result<(), FlowIoError> {
    let path = runs_dir.join(GITIGNORE);
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(runs_dir)
        .and_then(|()| std::fs::write(&path, GITIGNORE_BODY))
        .map_err(|source| FlowIoError {
            site: IoSite::Run,
            doing: WRITE_GITIGNORE,
            path,
            source,
        })
}

/// Leave the settings each node resolved to, which `defaults` merging has
/// already erased by this point. A serialization failure is folded into
/// `io::Error` so both ways of failing reach the caller as one warning.
fn write_run_yaml(run_dir: &Path, flow: &Flow, flow_dir: &Path) -> Result<(), FlowIoError> {
    let path = run_dir.join(RUN_YAML);
    yaml_serde::to_string(&crate::resolve::to_flow_file(flow, flow_dir))
        .map_err(std::io::Error::other)
        .and_then(|text| {
            std::fs::create_dir_all(run_dir).and_then(|()| std::fs::write(&path, text))
        })
        .map_err(|source| FlowIoError {
            site: IoSite::Run,
            doing: WRITE_SPEC,
            path,
            source,
        })
}

/// Leave the run's account of what it did. Fails the way `run.yaml` does —
/// into `warnings`, never into the outcome: a missing audit file is not a
/// reason to tell the user their run did not happen.
fn write_run_md(run_dir: &Path, report: &RunReport) -> Result<(), FlowIoError> {
    let path = run_dir.join(RUN_MD);
    std::fs::create_dir_all(run_dir)
        .and_then(|()| std::fs::write(&path, crate::record::render_run_md(report)))
        .map_err(|source| FlowIoError {
            site: IoSite::Run,
            doing: WRITE_RECORD,
            path,
            source,
        })
}

/// The report for a run that never reached its first node — refused the lock,
/// or left without a runtime to run with. Nothing ran, so there is nothing to
/// account for.
/// Put each pinned output where this run's nodes will look for it, and say
/// which pins took.
///
/// A copy that fails is a warning and not a refusal: the node then runs and
/// is paid for, which is what would have happened with no pin at all. A pin
/// whose source was never there is refused earlier, by `validate_request` —
/// that one is the user pointing at nothing.
fn copy_pinned_outputs(request: &RunRequest) -> (Vec<crate::NodeId>, Vec<String>) {
    let outputs: std::collections::HashMap<crate::NodeId, std::path::PathBuf> =
        crate::schedule::node_outputs(request.loaded.flow(), &request.run_dir)
            .into_iter()
            .collect();
    let mut took = Vec::new();
    let mut warnings = Vec::new();
    // A pin outside the selection is not this run's business. Copying it
    // anyway put a file in the run directory that no node here produced and
    // recorded it as reused, in a run whose own report says it stopped before
    // that node — and the run log counts a pinned node as passed, so the next
    // resume would skip it on the strength of an output it never saw made.
    let selected = crate::graph::Selection::of(request.loaded.flow(), request.effective_until());
    for pin in request
        .pinned
        .iter()
        .filter(|pin| selected.includes(&pin.node))
    {
        let Some(dest) = outputs.get(&pin.node) else {
            continue;
        };
        let made = dest
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::copy(&pin.from, dest));
        match made {
            Ok(_) => took.push(pin.node.clone()),
            Err(e) => warnings.push(format!(
                "the output pinned for `{}` could not be copied in, so it will be run: {e}",
                pin.node
            )),
        }
    }
    (took, warnings)
}

fn not_started(request: &RunRequest, outcome: RunOutcome) -> RunReport {
    RunReport::refused(request.run_dir.clone(), outcome)
}

/// The run's id is the name of its directory — the host builds
/// `<cwd>/.daruda/flow-runs/<run-id>/`, so there is nothing else to carry.
fn run_id_of(run_dir: &Path) -> Cow<'_, str> {
    run_dir
        .file_name()
        .unwrap_or(run_dir.as_os_str())
        .to_string_lossy()
}

#[cfg(test)]
mod tests;
