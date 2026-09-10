//! One run end to end: take the working directory, drive the graph, record
//! how it ended, give the directory back.

use super::{RunInputs, RunOutcome, RunReport, run_flow};
use crate::error::{FlowIoError, IoSite};
use crate::event::{FlowEvent, RunEnd, emit};
use crate::lock::{LockError, RunLocks};
use crate::marker::{DEFAULT_KEEP_RUNS, sweep_old_runs, write_marker};
use crate::model::Flow;
use crate::request::RunRequest;
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
const MAKE_RUNS_DIR: &str = "making the runs directory";
const GITIGNORE: &str = ".gitignore";
const GITIGNORE_BODY: &str = "*\n";
const WRITE_GITIGNORE: &str = "hiding the run directory from git";
const SWEEP_RUNS: &str = "clearing out old run directories";
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
    execute_with(request, runner, cancel, &|_, launch| {
        crate::runner::acp::provision(launch, &request.node_install_dir)
    })
}

/// Preparing one agent's runtime, by catalog id and launch spec. Injected so
/// the tests below can state *which* agents get prepared and *when* without
/// performing it: the real one downloads a Node.js runtime on a cold cache,
/// and a test that did that would be neither fast nor honest.
type Provision<'a> = dyn Fn(&str, &LaunchSpec) -> Result<(), String> + 'a;

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
    // It used to sit in the runs directory, covered by the `.gitignore`
    // below. That put the one file this whole exclusion rests on inside the
    // tree the exclusion protects — and `git clean -fdx` deletes ignored
    // files, so an agent tidying its own worktree could remove the lock
    // while the run still held the tree. `take` succeeds on a missing file,
    // so the next run would walk straight in.
    //
    // The tree's own path resolved, because two spellings of one tree must
    // not become two locks. Unresolvable is not a tree this run can be sure
    // it excludes anything in, so it refuses rather than guessing — the same
    // call the batcher makes about a node's directory.
    let tree = match request.cwd.canonicalize() {
        Ok(tree) => tree,
        // The error as the filesystem gave it: a permission denied read as
        // "no such directory" sends the reader looking for the wrong thing.
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
    // MIGRATION(from v0.2.12): the legacy place too, for one release. An older
    // build looks only
    // there, so writing it is what stops that build starting a second run
    // in a tree this one holds; and `run_status` reads it so a run *it*
    // started stays resumable.
    //
    // The copy is inside the tree, so `git clean -fdx` can take it — and
    // then an older build sees a free tree and starts a second run in one
    // this run holds. That window is what the move closes for every build
    // that knows the new place, and all it leaves is the older one; before
    // the move the same `git clean` freed the tree for *any* build.
    // Watching for the deletion would buy back the rest, at the price of a
    // watcher living as long as the compatibility copy — which is one
    // release.
    let legacy = request.run_dir.parent().unwrap_or(&request.cwd);
    let lock_dirs = vec![
        crate::lock::lock_dir_for(&request.lock_dir, &tree),
        legacy.to_path_buf(),
    ];
    // Before the locks, because a lock is a file inside one. Making a
    // directory claims nothing, so there is no race to lose here.
    if let Err(source) = std::fs::create_dir_all(legacy) {
        return not_started(
            request,
            RunOutcome::Io(FlowIoError {
                site: IoSite::Run,
                doing: MAKE_RUNS_DIR,
                path: legacy.to_path_buf(),
                source,
            }),
        );
    }
    let lock = match RunLocks::acquire(&lock_dirs, &run_id_of(&request.run_dir), &*request.is_alive)
    {
        Ok(lock) => lock,
        // Neither refusal took the directory, so neither writes a marker
        // and neither releases: the run that is going owns both.
        Err(LockError::Held(holder)) => {
            return not_started(request, RunOutcome::LockHeld { holder });
        }
        Err(LockError::Io(e)) => return not_started(request, RunOutcome::Io(e)),
    };

    // Only past the lock is there a run to announce: the two refusals above
    // took nothing, so a host watching them would see a run start and end
    // that never existed.
    emit(
        request.events.as_ref(),
        FlowEvent::RunStarted {
            run_dir: request.run_dir.clone(),
            nodes: request.loaded.graph().topological_order(),
        },
    );

    // Both belong to whoever sets a run directory up, and both go here for
    // the same reason as `run.yaml` below: past the lock, so no other run's
    // directory is touched, and before the first node, so a run that never
    // finishes is still hidden from git and still counted for retention.
    let mut setup_warnings = prepare_runs_dir(request.run_dir.parent());

    let resume = request.resume.clone();

    // Only a fresh run copies: a continuation's pins were copied by the
    // process that started it, and the journal already lists them as passed.
    let (pinned_ids, pin_warnings) = if resume.is_none() {
        copy_pinned_outputs(request)
    } else {
        (Vec::new(), Vec::new())
    };
    setup_warnings.extend(pin_warnings);

    // A continuation writes neither setup file again: the spec already in
    // the directory is the authority it reads back, and the journal it is
    // about to append to already has its opening line. Rewriting either
    // would replace the record of what the run *is* with this process's
    // idea of it.
    let (spec_warning, journal_warning) = match &resume {
        Some(replay) => {
            // Whatever the interrupted node had half-written is evidence,
            // not a result — `judge` cannot tell the two apart, and left
            // live it would be accepted as that node's output.
            setup_warnings.extend(
                crate::journal::resumed(&request.run_dir, replay.passed.len())
                    .err()
                    .map(|e| format!("this run's progress could not be marked as continued: {e}")),
            );
            setup_warnings.extend(crate::resume::archive_unclaimed_outputs(
                &request.run_dir,
                &request.run_dir.join(crate::schedule::LOG_DIR_NAME),
                &crate::schedule::node_outputs(request.loaded.flow(), &request.run_dir),
                &replay.passed,
            ));
            (None, None)
        }
        None => (
            // After the lock and before the first node: earlier would write
            // into a directory another run owns, later would leave a
            // crashed run — the one whose settings someone needs — with no
            // spec at all.
            write_run_yaml(&request.run_dir, request.loaded.flow(), &request.flow_dir)
                .err()
                .map(|e| e.to_string()),
            // Beside `run.yaml` and for the same reason, plus one of its
            // own: its presence is what tells a later resume that the crash
            // was not in setup.
            crate::journal::start(
                &request.run_dir,
                request.loaded.flow().profile.as_deref(),
                request.until.as_ref(),
                &pinned_ids,
            )
            .err()
            .map(|e| {
                format!("this run's progress cannot be written, so it cannot be resumed: {e}")
            }),
        ),
    };

    // Last of the setup steps, so a run that cannot be provisioned still
    // leaves the spec that says what it was going to do — and so a download
    // does not delay hiding the directory from git.
    let mut report = match provision_agents(request, provision) {
        Ok(()) => smol::block_on(run_flow(
            RunInputs {
                loaded: &request.loaded,
                flow_dir: &request.flow_dir,
                cwd: &request.cwd,
                run_dir: &request.run_dir,
                cancel,
                budget: &request.budget,
                git_status: request
                    .git_status
                    .as_ref()
                    .map(|ask| &**ask as &dyn Fn(&std::path::Path) -> Option<String>),
                events: request.events.as_ref(),
                ask: request.ask.as_ref(),
                until: request.effective_until().cloned(),
                // A continuation reads its pins back from the journal — where
                // `absorb` kept them apart from `passed` precisely so the
                // record can still say which nodes were reused rather than run.
                pinned: match &resume {
                    Some(replay) => replay.pinned.clone(),
                    None => pinned_ids.clone(),
                },
                resume,
            },
            runner,
        )),
        Err(outcome) => not_started(request, outcome),
    };

    // In front, because they happened first — all three are setup steps that
    // ran before the first node. None of them touches `RunOutcome`: a run
    // directory that could not be tidied or audited still ran.
    setup_warnings.extend(spec_warning);
    setup_warnings.extend(journal_warning);
    report.warn_from_setup(setup_warnings);
    // Before the marker, and after the warning above so the record carries
    // it: the marker is the "it is all over" signal, and a reader that acts
    // on it must not find a finished run with no account of itself.
    if let Err(e) = write_run_md(&report.run_dir, &report) {
        report.warn(e.to_string());
    }
    // Both of these run on every exit path, and neither replaces the
    // outcome: the run has already ended and that is what the user needs.
    // A `?` here would skip the release while this process stays alive, and
    // `is_alive` would then wedge the directory until `STALE_AFTER`.
    if let Err(e) = write_marker(&report.run_dir, &report.outcome) {
        report.warn(e.to_string());
    }
    // After the marker, never before: in the window between a freed lock
    // and an unwritten marker a reader sees neither and calls a finished
    // run `Unknown`. A leaked lock is recovered by the next run's reclaim.
    if let Err(e) = lock.release() {
        report.warn(e.to_string());
    }
    // Last, after the marker: a host that reacts to this by opening the run
    // directory would otherwise find no marker and read a finished run as
    // `Unknown`. The end carries why, not just that — the marker folds
    // `Failed`, `BudgetExhausted` and `Io` into one word.
    emit(
        request.events.as_ref(),
        FlowEvent::RunEnded {
            end: RunEnd::from(&report.outcome),
        },
    );
    report
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
fn provision_agents(request: &RunRequest, provision: &Provision<'_>) -> Result<(), RunOutcome> {
    let mut prepared = HashSet::new();
    for id in request.selected_agents() {
        if !prepared.insert(id) {
            continue;
        }
        let Some(launch) = request.agents.get(id) else {
            continue;
        };
        if let Err(message) = provision(id, launch) {
            return Err(RunOutcome::Unprovisioned {
                agent: id.to_string(),
                message,
            });
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
    // that node — and the journal counts a pinned node as passed, so the next
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
