//! Scheduler behaviour, driven end to end through the scripted runner.
//!
//! This file is the harness the sibling modules share — the four ways to
//! start a run, the gated flow the repair tests are all written against,
//! and the request shape a host submits. The tests themselves are split by
//! the fixture they need, which is the axis they actually differ on: a
//! budget test hands over a ceiling nothing else does, a lifecycle test
//! plants a lock nothing else does.

use super::*;
use crate::error::IoSite;
use crate::event::{FlowEvent, RunEnd};
use crate::load::load;
use crate::lock::RunLock;
use crate::record::AttemptOutcome;
use crate::request::{Budget, CostLimit};
use crate::runner::{CancelToken, NodeFailure};
use crate::testing::{FakeRunner, Step};
use std::time::Duration;

pub(super) const CHAIN: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
  - id: test
    kind: command
    deps: [design]
    run: \"true\"
  - id: review
    kind: agent
    deps: [test]
    output: review.md
    prompt: write
";

fn run(text: &str, runner: &FakeRunner) -> (RunReport, tempfile::TempDir) {
    run_with(text, runner, &CancelToken::default(), Budget::unlimited())
}

fn run_with_cancel(
    text: &str,
    runner: &FakeRunner,
    cancel: &CancelToken,
) -> (RunReport, tempfile::TempDir) {
    run_with(text, runner, cancel, Budget::unlimited())
}

/// `dyn` because a handful of tests wrap the fake in a runner of their own
/// — a link planter, a lock stealer, two calls in flight at once.
fn run_with_budget(
    text: &str,
    runner: &dyn NodeRunner,
    budget: Budget,
) -> (RunReport, tempfile::TempDir) {
    run_with(text, runner, &CancelToken::default(), budget)
}

/// Run with `node` pinned, its output already sitting where the copy step
/// would have put it — so this level tests the skip and what downstream sees,
/// not the copy itself (that is `schedule::run`'s, tested there).
fn run_pinned(
    text: &str,
    runner: &dyn NodeRunner,
    node: &str,
    output: &str,
    body: &str,
) -> (RunReport, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(run_dir.join(output), body).expect("place the pinned output");
    let loaded = load(text, None).expect("valid flow");
    let report = smol::block_on(run_flow(
        RunInputs {
            until: None,
            pinned: vec![crate::NodeId::from(node)],
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &run_dir,
            cancel: &CancelToken::default(),
            budget: &Budget::unlimited(),
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        },
        runner,
    ));
    (report, dir)
}

/// Run no further than `target`, which is the whole point of the selection
/// axis — every other helper passes `None`.
fn run_until(text: &str, runner: &dyn NodeRunner, target: &str) -> (RunReport, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(text, None).expect("valid flow");
    let report = smol::block_on(run_flow(
        RunInputs {
            pinned: Vec::new(),
            until: Some(crate::NodeId::from(target)),
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &dir.path().join("run"),
            cancel: &CancelToken::default(),
            budget: &Budget::unlimited(),
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        },
        runner,
    ));
    (report, dir)
}

fn run_with(
    text: &str,
    runner: &dyn NodeRunner,
    cancel: &CancelToken,
    budget: Budget,
) -> (RunReport, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(text, None).expect("valid flow");
    let report = smol::block_on(run_flow(
        RunInputs {
            pinned: Vec::new(),
            until: None,
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &dir.path().join("run"),
            cancel,
            budget: &budget,
            git_status: None,
            events: None,
            ask: None,
            resume: None,
        },
        runner,
    ));
    (report, dir)
}

/// The contract block the scheduler appends, spelled out rather than asked
/// of the code under test — a test that composed it the same way would pass
/// whatever the wording became. `output` is relative, the way a flow
/// declares it; the block must state the absolute path.
pub(super) fn expected_contract(run_dir: &Path, output: &str) -> String {
    format!(
        "OUTPUT CONTRACT (machine-validated):\n\
         When you are done, write your result to {}.\n\
         The file must exist and be non-empty; a symlink is refused.",
        run_dir.join(output).display()
    )
}

const GATED: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: implement
    kind: agent
    output: implement.md
    prompt: write
  - id: review
    kind: agent
    deps: [implement]
    output: review.md
    prompt: review
  - id: gate
    kind: command
    deps: [review]
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        rerun: [review]
        max_attempts: 2
        wait: 0s
";

/// A run as the host submits it: the lock lives beside the working
/// directory, and the run id is the run directory's own name.
pub(super) fn request_for(text: &str, dir: &std::path::Path) -> crate::request::RunRequest {
    request_for_profile(text, None, dir)
}

/// The same request, run under a named profile.
pub(super) fn request_for_profile(
    text: &str,
    profile: Option<&str>,
    dir: &std::path::Path,
) -> crate::request::RunRequest {
    let loaded = load(text, profile).expect("valid flow");
    crate::request::RunRequest {
        loaded,
        until: None,
        pinned: Vec::new(),
        cwd: dir.to_path_buf(),
        run_dir: dir.join(".daruda/flow-runs/01J"),
        flow_dir: dir.to_path_buf(),
        agents: std::collections::HashMap::from([(
            "claude".to_string(),
            daruda_acp::LaunchSpec {
                command: "x".to_string(),
                strip_env: Vec::new(),
            },
        )]),
        node_install_dir: dir.to_path_buf(),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| true),
        git_status: None,
        events: None,
        ask: None,
        resume: None,
    }
}

/// Replaces the lock mid-run with another run's, the way a mistaken
/// reclaim would. Nothing else puts a foreign holder under a live run.
struct LockStealer(FakeRunner);

/// Put a different run's lock in place, atomically enough for a test.
///
/// In the runs directory, where `execute` takes it — under `cwd` this would
/// write a file the engine never reads, and the test would pass whatever
/// `release` did.
fn steal(run_dir: &Path) {
    let Some(runs_dir) = run_dir.parent() else {
        return;
    };
    let _ = std::fs::write(
        runs_dir.join(".lock"),
        "pid: 999999\nrun_id: someone-else\nstarted_unix_secs: 1\n",
    );
}

impl NodeRunner for LockStealer {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        steal(ctx.run_dir);
        self.0.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        steal(ctx.run_dir);
        self.0.run_command(ctx, run)
    }
}

/// Removes the lock mid-run, the way a stray cleanup would.
struct LockLoser(FakeRunner);

impl NodeRunner for LockLoser {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        let _ = ctx
            .run_dir
            .parent()
            .map(|d| std::fs::remove_file(d.join(".lock")));
        self.0.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        let _ = ctx
            .run_dir
            .parent()
            .map(|d| std::fs::remove_file(d.join(".lock")));
        self.0.run_command(ctx, run)
    }
}

mod artifacts;
mod budgets;
mod cancel;
mod contract;
mod lifecycle;
mod ordering;
mod parallel;
mod prompts;
mod records;
mod repair;
mod resuming;
mod working_dirs;
