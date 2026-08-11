//! One run end to end: take the working directory, drive the graph, record
//! how it ended, give the directory back.

use super::{RunInputs, RunOutcome, RunReport, run_flow};
use crate::error::{FlowIoError, IoSite};
use crate::event::{FlowEvent, RunEnd, emit};
use crate::lock::{LockError, RunLock};
use crate::marker::{DEFAULT_KEEP_RUNS, sweep_old_runs, write_marker};
use crate::model::{Flow, NodeKind};
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

/// Keeps every run's artifacts out of the user's `git status`. Design §10
/// puts it inside the runs directory rather than at `.daruda/`, so that the
/// visibility of the `task-*.md` files already living there is not silently
/// changed.
const MAKE_RUNS_DIR: &str = "making the runs directory";
const GITIGNORE: &str = ".gitignore";
const GITIGNORE_BODY: &str = "*\n";
const WRITE_GITIGNORE: &str = "hiding the run directory from git";
const SWEEP_RUNS: &str = "clearing out old run directories";

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

    // In the runs directory, not `cwd`. It excludes exactly the same thing
    // either way — there is one runs directory per working directory — but
    // only here is it covered by the `.gitignore` below. At `cwd` the lock
    // is a stray file in the user's repository root, which is the noise
    // that `.gitignore` exists to prevent.
    let lock_dir = request.run_dir.parent().unwrap_or(&request.cwd);
    // Before the lock, because the lock is a file inside it. Making a
    // directory claims nothing, so there is no race to lose here.
    if let Err(source) = std::fs::create_dir_all(lock_dir) {
        return not_started(
            request,
            RunOutcome::Io(FlowIoError {
                site: IoSite::Run,
                doing: MAKE_RUNS_DIR,
                path: lock_dir.to_path_buf(),
                source,
            }),
        );
    }
    let lock = match RunLock::acquire(lock_dir, &run_id_of(&request.run_dir), &*request.is_alive) {
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
            crate::journal::start(&request.run_dir, request.loaded.flow().profile.as_deref())
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
                    .map(|ask| &**ask as &dyn Fn() -> Option<String>),
                events: request.events.as_ref(),
                ask: request.ask.as_ref(),
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
    for id in agent_ids(request.loaded.flow()) {
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

/// Every agent this run could open a session as: one per agent node, then the
/// repair agent. With repeats — the caller dedupes.
fn agent_ids(flow: &Flow) -> impl Iterator<Item = &str> {
    flow.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            NodeKind::Agent { agent, .. } => Some(agent.id.as_str()),
            NodeKind::Command { .. } => None,
        })
        .chain(flow.default_agent.iter().map(|agent| agent.id.as_str()))
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
mod tests {
    use super::super::tests::{CHAIN, request_for};
    use super::*;
    use crate::marker::DEFAULT_KEEP_RUNS;
    use crate::testing::FakeRunner;
    use std::cell::RefCell;

    /// `review` overrides the flow's agent, so one run needs two runtimes —
    /// the case design §6 added provisioning for.
    const TWO_AGENTS: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
  - id: review
    kind: agent
    deps: [design]
    agent: { id: codex, mode: auto }
    output: review.md
    prompt: write
";

    /// No node names an agent, and yet a repair's `fix` would open a session
    /// as `defaults.agent`.
    const COMMAND_ONLY: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
";

    fn spec() -> daruda_acp::LaunchSpec {
        daruda_acp::LaunchSpec {
            command: "x".to_string(),
            strip_env: Vec::new(),
        }
    }

    fn runs_dir_of(run_dir: &Path) -> &Path {
        run_dir.parent().expect("a run directory has a parent")
    }

    fn finished_run_in(dir: &Path, run_id: &str) -> std::path::PathBuf {
        let run_dir = dir.join(run_id);
        std::fs::create_dir_all(&run_dir).expect("mkdir");
        std::fs::write(run_dir.join("DONE"), "").expect("marker");
        run_dir
    }

    /// Without this every run's artifacts show up in the user's `git status`
    /// — the repo's own `.gitignore` has no `.daruda` entry.
    #[test]
    fn the_first_run_hides_its_output_from_git() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report = execute(
            &request_for(CHAIN, dir.path()),
            &FakeRunner::new(),
            &CancelToken::default(),
        );

        let text = std::fs::read_to_string(runs_dir_of(&report.run_dir).join(".gitignore"))
            .expect("written");
        assert!(text.contains('*'), "{text}");
    }

    /// The user may have edited it. Rewriting on every run would undo that.
    #[test]
    fn a_later_run_does_not_rewrite_an_existing_gitignore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let request = request_for(CHAIN, dir.path());
        let runs_dir = runs_dir_of(&request.run_dir).to_path_buf();
        std::fs::create_dir_all(&runs_dir).expect("mkdir");
        std::fs::write(runs_dir.join(".gitignore"), "*\n!keep-me.md\n").expect("write");

        let _ = execute(&request, &FakeRunner::new(), &CancelToken::default());

        assert_eq!(
            std::fs::read_to_string(runs_dir.join(".gitignore")).expect("still there"),
            "*\n!keep-me.md\n"
        );
    }

    /// Retention only matters if a run actually performs it. Invisible to
    /// git is not invisible to the disk.
    #[test]
    fn a_run_sweeps_the_runs_directory_it_starts_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let request = request_for(CHAIN, dir.path());
        let runs_dir = runs_dir_of(&request.run_dir).to_path_buf();
        std::fs::create_dir_all(&runs_dir).expect("mkdir");
        let old: Vec<_> = (0..DEFAULT_KEEP_RUNS + 5)
            .map(|i| finished_run_in(&runs_dir, &format!("01A{i:02}")))
            .collect();

        let report = execute(&request, &FakeRunner::new(), &CancelToken::default());

        assert!(report.warnings().is_empty(), "{:?}", report.warnings());
        for gone in &old[..5] {
            assert!(!gone.exists(), "{} survived the sweep", gone.display());
        }
        assert!(old[5].is_dir(), "the newest 20 must stay");
        assert!(report.run_dir.join("DONE").is_file());
    }

    /// The reason this is not lazy: a first-run download inside a node's turn
    /// eats that node's timeout, and the node that pays is whichever happened
    /// to be first.
    #[test]
    fn every_distinct_agent_is_provisioned_before_the_first_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut request = request_for(TWO_AGENTS, dir.path());
        request.agents.insert("codex".to_string(), spec());
        let runner = FakeRunner::new();
        let provisioned = RefCell::new(Vec::new());

        let report = execute_with(&request, &runner, &CancelToken::default(), &|id, _| {
            assert!(
                runner.calls().is_empty(),
                "`{id}` was prepared after a node had already run"
            );
            provisioned.borrow_mut().push(id.to_string());
            Ok(())
        });

        assert!(
            matches!(report.outcome, RunOutcome::Done),
            "{:?}",
            report.outcome
        );
        let mut prepared = provisioned.into_inner();
        prepared.sort();
        assert_eq!(prepared, vec!["claude", "codex"]);
    }

    /// Two nodes naming the same agent must not provision twice.
    #[test]
    fn one_agent_named_by_many_nodes_is_provisioned_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();
        let provisioned = RefCell::new(Vec::new());

        let report = execute_with(
            &request_for(CHAIN, dir.path()),
            &runner,
            &CancelToken::default(),
            &|id, _| {
                provisioned.borrow_mut().push(id.to_string());
                Ok(())
            },
        );

        assert!(
            matches!(report.outcome, RunOutcome::Done),
            "{:?}",
            report.outcome
        );
        assert_eq!(provisioned.into_inner(), vec!["claude"]);
    }

    /// A repair's `fix` opens a real session as `defaults.agent`, so that
    /// agent needs a runtime too — in a flow where no node names one, nothing
    /// else would ever ask for it.
    #[test]
    fn the_repair_agent_is_provisioned_even_when_no_node_names_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let provisioned = RefCell::new(Vec::new());

        let report = execute_with(
            &request_for(COMMAND_ONLY, dir.path()),
            &FakeRunner::new(),
            &CancelToken::default(),
            &|id, _| {
                provisioned.borrow_mut().push(id.to_string());
                Ok(())
            },
        );

        assert!(
            matches!(report.outcome, RunOutcome::Done),
            "{:?}",
            report.outcome
        );
        assert_eq!(provisioned.into_inner(), vec!["claude"]);
    }

    /// Provisioning is what makes a run possible at all, so its failure is the
    /// run's — not a node's, since no node has run yet.
    #[test]
    fn a_provisioning_failure_stops_the_run_before_any_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runner = FakeRunner::new();

        let report = execute_with(
            &request_for(CHAIN, dir.path()),
            &runner,
            &CancelToken::default(),
            &|id, _| Err(format!("no runtime for `{id}`")),
        );

        match &report.outcome {
            RunOutcome::Unprovisioned { agent, message } => {
                assert_eq!(agent, "claude");
                assert!(message.contains("no runtime"), "{message}");
            }
            other => panic!("expected an unprovisioned run, got {other:?}"),
        }
        assert!(runner.calls().is_empty(), "no node may have run");
        // This run took the directory, so it still ends the way every started
        // run does — a reader acting on the marker needs the account beside it.
        assert!(report.run_dir.join("FAILED").is_file());
        assert!(report.run_dir.join(RUN_MD).is_file());
    }
}
