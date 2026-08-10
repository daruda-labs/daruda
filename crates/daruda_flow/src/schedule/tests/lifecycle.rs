//! Lock and marker: what a run leaves behind, and what it must not
//! leave behind when it never started. The fixtures here plant and steal
//! locks, which nothing else does.

use super::*;

/// The order the whole lifecycle depends on: the marker is written before
/// the lock is released. Reversed, a reader that sees a free lock and no
/// marker calls a finished run `Unknown`.
#[test]
fn a_finished_run_leaves_a_marker_and_frees_the_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new();
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(report.run_dir.join("DONE").is_file());
    assert!(!dir.path().join(".lock").exists());
    assert_eq!(
        crate::marker::run_status(&report.run_dir, &|_| true),
        crate::marker::RunStatus::Done
    );
}

/// Releasing only on the happy path is the failure mode that wedges a
/// directory: `is_alive` then says the holder is live and nothing reclaims
/// it until `STALE_AFTER`.
#[test]
fn a_failed_run_frees_the_lock_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new().script(
        "design",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Failed { .. }),
        "{:?}",
        report.outcome
    );
    assert!(report.run_dir.join("FAILED").is_file());
    // Not just "the file is gone": the next run has to be able to take it.
    RunLock::acquire(dir.path(), "next", &|_| true)
        .expect("a failed run leaves the directory free")
        .release()
        .expect("release");
}

/// A run that could not take the lock leaves nothing at all — writing a
/// marker here would stamp a status onto the directory of a run that is
/// still going.
#[test]
fn a_run_that_loses_the_lock_writes_no_marker() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The lock lives in the runs directory, beside the run dirs — the one
    // place `.gitignore` covers, so it stays out of the user's `git status`.
    let runs_dir = dir.path().join(".daruda/flow-runs");
    std::fs::create_dir_all(&runs_dir).expect("mkdir");
    let held = RunLock::acquire(&runs_dir, "other", &|_| true).expect("free");
    let runner = FakeRunner::new();
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    match &report.outcome {
        RunOutcome::LockHeld { holder } => assert_eq!(holder.run_id, "other"),
        other => panic!("expected LockHeld, got {other:?}"),
    }
    assert!(runner.calls().is_empty());
    assert!(
        std::fs::read_dir(&report.run_dir)
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    );
    // And it did not release what it never took.
    held.release()
        .expect("the original holder still owns the lock");
}

/// A run whose lock was taken over while it was still going must not
/// delete the lock of whoever took it — that would turn one mistaken
/// reclaim into a directory nobody holds. The run's own result stands.
#[test]
fn a_run_whose_lock_was_stolen_leaves_the_new_holders_lock_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = LockStealer(FakeRunner::new());
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(report.run_dir.join("DONE").is_file());
    assert!(
        report
            .run_dir
            .parent()
            .is_some_and(|runs| runs.join(".lock").is_file()),
        "the other run still holds the directory"
    );
    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
}

/// The one way a run leaves without its lock: it was already gone. That is
/// not a failure to report — there is nothing left to release.
#[test]
fn a_run_whose_lock_vanished_reports_nothing_extra() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = LockLoser(FakeRunner::new());
    let report = execute(
        &request_for(CHAIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
}

/// A relative `cwd` is not a flow's mistake, it is the host's — and until
/// something refused it the engine took a lock, wrote a spec and built a run
/// directory relative to whatever the process's own directory happened to
/// be, then failed at the adapter with a message about `cwd`.
///
/// The run must not start, and nothing may be written outside the request's
/// own paths.
#[test]
fn a_request_with_a_relative_path_never_starts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut request = request_for(CHAIN, dir.path());
    request.cwd = std::path::PathBuf::from("Users/woo/git/temp");
    request.run_dir = request.cwd.join(".daruda/flow-runs/01J");

    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());

    match &report.outcome {
        RunOutcome::Invalid { issues } => assert!(
            issues.iter().any(|i| matches!(
                i.kind,
                crate::error::ValidationKind::RelativeRequestPath { .. }
            )),
            "{issues:?}"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(runner.calls().is_empty(), "no node may run");
    assert!(
        !std::path::Path::new("Users").exists(),
        "a relative path was resolved against the process's own directory"
    );
}
