//! A quit while a wave is in flight. Only what the quit cut short is left to
//! resume; an ending a sibling reached on its own is recorded as the run's.

use super::*;
use crate::marker::{RunStatus, run_status};

/// Two agent nodes in one wave, each in its own directory. `first` is
/// declared before `second`, so it names the run's ending when both stop.
fn two_in_a_wave(first: &str, second: &str) -> String {
    format!(
        "\
version: 1
defaults:
  agent: {{ id: claude }}
  parallel: 2
nodes:
  - id: {first}
    kind: agent
    cwd: {first}
    output: {first}.md
    prompt: write
  - id: {second}
    kind: agent
    cwd: {second}
    output: {second}.md
    prompt: write
"
    )
}

/// The run ended `Failed` on `failed`, recorded in its log and marker, the
/// lock given back, and nothing left to resume.
fn assert_ended_failed_on(report: &RunReport, request: &crate::request::RunRequest, failed: &str) {
    assert!(
        matches!(&report.outcome, RunOutcome::Failed { node, .. } if node == failed),
        "{:?}",
        report.outcome
    );
    let state = crate::run_log::reader::state(&report.run_dir).expect("a log");
    assert_eq!(
        state.phase(),
        crate::run_log::machine::RunPhase::Ended(crate::run_log::machine::EndKind::Failed)
    );
    assert!(report.run_dir.join("FAILED").is_file());
    let lock_dir = super::lock_dir_of(request);
    assert!(
        crate::lock::read_holder(&lock_dir).is_none(),
        "the lock was kept"
    );
    let status = run_status(&report.run_dir, Some(&lock_dir), &|_| false);
    assert_eq!(status, RunStatus::Failed);
    assert!(matches!(
        crate::resume::prepare(&report.run_dir, Some(&lock_dir), &|_| false),
        Err(crate::resume::ResumeError::NotResumable(RunStatus::Failed))
    ));
}

/// A node that failed for good before the app quit ended the run; the quit
/// only cut its sibling short, and must not reopen a failed run.
#[test]
fn a_quit_beside_a_terminal_failure_ends_the_run_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let request = request_for(&two_in_a_wave("a", "b"), dir.path());
    let runner = FakeRunner::new()
        .script("a", vec![Step::fail(NodeFailure::Exit { code: Some(1) })])
        .shuts_down_at("b", 1);

    let report = execute(&request, &runner, &CancelToken::default());

    assert_eq!(runner.ids(), vec![NodeId::from("a"), NodeId::from("b")]);
    assert_ended_failed_on(&report, &request, "a");
}

/// The same when the quitting node is declared first: a refusal its
/// sibling met before any session outranks the quit, whatever the order.
#[test]
fn a_quit_beside_a_refused_node_ends_the_run_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let request = request_for(&two_in_a_wave("b", "a"), dir.path());
    // A link where `a` writes: its preflight refuses it, and no session runs.
    std::fs::create_dir_all(&request.run_dir).expect("mkdir");
    let elsewhere = dir.path().join("elsewhere.md");
    std::fs::write(&elsewhere, "not the run's\n").expect("write");
    daruda_core::path::symlink(&elsewhere, request.run_dir.join("a.md")).expect("symlink");
    let runner = FakeRunner::new().shuts_down_at("b", 1);

    let report = execute(&request, &runner, &CancelToken::default());

    assert_eq!(runner.ids(), vec![NodeId::from("b")]);
    assert_ended_failed_on(&report, &request, "a");
}

/// A quit with nothing else ending the run leaves it to resume, as before.
#[test]
fn a_quit_alone_still_leaves_the_run_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let request = request_for(&two_in_a_wave("a", "b"), dir.path());
    let report = execute(
        &request,
        &FakeRunner::new().shuts_down_at("b", 1),
        &CancelToken::default(),
    );

    assert!(
        matches!(report.outcome, RunOutcome::ShutDown),
        "{:?}",
        report.outcome
    );
    let lock_dir = super::lock_dir_of(&request);
    assert_eq!(
        run_status(&report.run_dir, Some(&lock_dir), &|_| false),
        RunStatus::Crashed
    );
}
