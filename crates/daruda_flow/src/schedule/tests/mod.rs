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

fn run_with_budget(
    text: &str,
    runner: &FakeRunner,
    budget: Budget,
) -> (RunReport, tempfile::TempDir) {
    run_with(text, runner, &CancelToken::default(), budget)
}

fn run_with(
    text: &str,
    runner: &FakeRunner,
    cancel: &CancelToken,
    budget: Budget,
) -> (RunReport, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(text).expect("valid flow");
    let report = smol::block_on(run_flow(
        RunInputs {
            loaded: &loaded,
            flow_dir: dir.path(),
            cwd: dir.path(),
            run_dir: &dir.path().join("run"),
            cancel,
            budget: &budget,
            git_status: None,
            events: None,
        },
        runner,
    ));
    (report, dir)
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
    let loaded = load(text).expect("valid flow");
    crate::request::RunRequest {
        loaded,
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
mod lifecycle;
mod ordering;
mod records;
mod repair;
