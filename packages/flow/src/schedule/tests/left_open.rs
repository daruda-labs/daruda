//! A run whose log never ended and whose lock nobody holds: what a resume
//! refused under the lock, or a run thread that panicked, leaves behind.
//! Only a live writer holds the lock, so such a run is crashed and must
//! stay resumable.

use super::crash::{Process, process};
use super::resume_steps::{as_if_killed, resume_request};
use super::*;
use crate::marker::{RunStatus, run_status};
use crate::model::AgentSpec;
use crate::runner::{NodeRunner, RunContext, RunResult};
use daruda_runlog::testing::{Fault, FaultBackend};
use std::future::Future;
use std::pin::Pin;

/// A run of `CHAIN` killed in its `test` node, with its stale lock in place.
fn killed(dir: &Path) -> (crate::request::RunRequest, std::path::PathBuf) {
    let request = request_for(CHAIN, dir);
    let first = execute(
        &request,
        &FakeRunner::new().cancel_at("test", 1),
        &CancelToken::default(),
    );
    as_if_killed(&request, &first.run_dir);
    (request, first.run_dir)
}

/// A resume that takes the lock and then cannot open the log gives the lock
/// back. The log still never ended, so the run reads as crashed rather than
/// `Unknown`, and a later resume can finish it.
#[test]
fn a_resume_refused_under_the_lock_leaves_the_run_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (request, run_dir) = killed(dir.path());
    let log = crate::run_log::log_file(&run_dir);
    let whole = std::fs::read(&log).expect("the log");
    // A record from a newer build: the open refuses it, after the lock.
    let mut newer = whole.clone();
    newer.extend_from_slice(b"{\"v\":999}\n");
    std::fs::write(&log, &newer).expect("append");

    let resumed = resume_request(CHAIN, dir.path(), &run_dir).expect("a killed run is resumable");
    let refused = execute(&resumed, &FakeRunner::new(), &CancelToken::default());

    assert!(
        matches!(
            &refused.outcome,
            RunOutcome::Log(e) if matches!(e.refusal, crate::error::LogRefusal::TooNew { .. })
        ),
        "{:?}",
        refused.outcome
    );
    let lock_dir = super::lock_dir_of(&request);
    assert!(
        crate::lock::read_holder(&lock_dir).is_none(),
        "the refused resume kept the lock"
    );
    assert_eq!(
        run_status(&run_dir, Some(&lock_dir), &|_| false),
        RunStatus::Crashed
    );

    std::fs::write(&log, &whole).expect("restore");
    let resumed = resume_request(CHAIN, dir.path(), &run_dir).expect("still resumable");
    let report = execute(&resumed, &FakeRunner::new(), &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
}

/// A pick-up whose `Resumed` commit fails, after the markers were cleared:
/// the log it leaves is still open, and with no lock held the run reads as
/// crashed and the next process finishes it.
#[test]
fn a_commit_failure_during_a_resume_leaves_the_run_resumable() {
    let loaded = load(super::crash::TWO_RETRYING, None).expect("valid flow");
    let dir = tempfile::tempdir().expect("tempdir");
    // opened 0, run_started 1, attempt_started 2, attempt_passed 3.
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: 3, keep: 0 });
    let _ = process(first.clone(), &loaded, dir.path(), &FakeRunner::new());
    // opened 0, resumed 1.
    let second = first.crash().with_fault(Fault::FailSyncAfter { k: 1 });
    assert!(matches!(
        process(second.clone(), &loaded, dir.path(), &FakeRunner::new()),
        Process::Refused(_)
    ));

    // What the failed pick-up left, where a host reads it.
    let third = second.crash();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(crate::run_log::log_dir(&run_dir)).expect("mkdir");
    let left = daruda_runlog::LogBackend::read(&third, 0)
        .expect("a log")
        .log;
    std::fs::write(crate::run_log::log_file(&run_dir), left).expect("write");
    let no_lock = dir.path().join("locks");
    assert_eq!(
        run_status(&run_dir, Some(&no_lock), &|_| false),
        RunStatus::Crashed
    );

    let Process::Drove(report) = process(third, &loaded, dir.path(), &FakeRunner::new()) else {
        panic!("the run could not be picked up");
    };
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
}

/// Panics inside its first agent call.
struct Panicking;

impl NodeRunner for Panicking {
    fn run_agent<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _agent: &'a AgentSpec,
        _prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        panic!("the runner panicked");
    }

    fn run_command<'a>(
        &'a self,
        _ctx: &'a RunContext<'a>,
        _run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        panic!("the runner panicked");
    }
}

/// A run thread that panics gives its lock back while unwinding. Its log
/// never ended, so the run reads as crashed, and a resume finishes it.
#[test]
fn a_run_that_panicked_is_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(&request, &Panicking, &CancelToken::default())
    }));
    assert!(panicked.is_err(), "the runner did not panic");

    let lock_dir = super::lock_dir_of(&request);
    assert!(crate::lock::read_holder(&lock_dir).is_none());
    assert_eq!(
        run_status(&request.run_dir, Some(&lock_dir), &|_| false),
        RunStatus::Crashed
    );
    let resumed =
        resume_request(CHAIN, dir.path(), &request.run_dir).expect("a panicked run is resumable");
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.calls()[0].attempt, 2, "the cut attempt was reused");
}

/// A run that ended between `prepare` and the lock is refused under the
/// lock as not resumable, which a host must be able to tell from a disk
/// failure, and nothing in its directory changes.
#[test]
fn a_resume_of_a_run_that_ended_meanwhile_is_refused_as_such() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let first = execute(&request, &FakeRunner::new(), &CancelToken::default());
    assert!(matches!(first.outcome, RunOutcome::Done));
    let mut resumed = request_for(CHAIN, dir.path());
    resumed.resume = Some(crate::resume::Continuation::default());

    let refused = execute(&resumed, &FakeRunner::new(), &CancelToken::default());

    assert!(
        matches!(
            refused.outcome,
            RunOutcome::NotResumed(crate::resume::ResumeError::NotResumable(RunStatus::Done))
        ),
        "{:?}",
        refused.outcome
    );
    assert!(first.run_dir.join("DONE").is_file());
}
