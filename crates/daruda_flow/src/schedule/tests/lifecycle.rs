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
        crate::marker::run_status(&report.run_dir, report.run_dir.parent(), &|_| true),
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
    // The lock lives outside the tree, so `git clean -fdx` inside the tree
    // cannot take it while a run still holds it.
    let request = request_for(CHAIN, dir.path());
    let lock_dir = super::lock_dir_of(&request);
    std::fs::create_dir_all(&lock_dir).expect("mkdir");
    let held = RunLock::acquire(&lock_dir, "other", &|_| true).expect("free");
    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());
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
    let request = request_for(CHAIN, dir.path());
    let lock_dir = super::lock_dir_of(&request);
    let runner = LockStealer(FakeRunner::new(), lock_dir.clone());
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(report.run_dir.join("DONE").is_file());
    assert!(
        lock_dir.join(".lock").is_file(),
        "the other run still holds the directory"
    );
    assert!(report.warnings().is_empty(), "{:?}", report.warnings());
}

/// The one way a run leaves without its lock: it was already gone. That is
/// not a failure to report — there is nothing left to release.
#[test]
fn a_run_whose_lock_vanished_reports_nothing_extra() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let runner = LockLoser(FakeRunner::new(), super::lock_dir_of(&request));
    let report = execute(&request, &runner, &CancelToken::default());
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

/// **The lock is under the root the host handed over, not wherever the run
/// happens to be.**
///
/// Which root stays the host's call; what the engine owes is to key it off
/// the tree by `lock_dir_for`. Asserted mid-run, because `execute` gives
/// the lock back on the way out and the end state looks the same either
/// way.
#[test]
fn execute_takes_the_lock_under_the_given_root_and_the_copy_inside_the_tree() {
    /// Looks at both places on each call, then delegates. Carries the root
    /// off the request rather than rebuilding it, so the test cannot agree
    /// with itself about a path the engine was never given.
    struct Watcher(
        FakeRunner,
        std::path::PathBuf,
        std::cell::RefCell<Vec<(bool, bool)>>,
    );

    impl Watcher {
        fn look(&self, ctx: &RunContext<'_>) {
            let tree = crate::lock::CanonicalTree::resolve(ctx.cwd).expect("the tree resolves");
            let under_root = crate::lock::lock_dir_for(&self.1, &tree);
            self.2.borrow_mut().push((
                crate::lock::read_holder(&under_root).is_some(),
                ctx.run_dir
                    .parent()
                    .and_then(crate::lock::read_holder)
                    .is_some(),
            ));
            // The lock lives outside the tree and nowhere else.
        }
    }

    impl NodeRunner for Watcher {
        fn run_agent<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            agent: &'a crate::model::AgentSpec,
            prompt: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.look(ctx);
            self.0.run_agent(ctx, agent, prompt)
        }

        fn run_command<'a>(
            &'a self,
            ctx: &'a RunContext<'a>,
            run: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
            self.look(ctx);
            self.0.run_command(ctx, run)
        }
    }

    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let watcher = Watcher(
        FakeRunner::new(),
        request.lock_dir.clone(),
        std::cell::RefCell::new(Vec::new()),
    );
    execute(&request, &watcher, &CancelToken::default());

    let seen = watcher.2.borrow().clone();
    assert!(!seen.is_empty(), "no node ran, so nothing was observed");
    assert!(
        seen.iter().all(|(under_root, _)| *under_root),
        "the authoritative lock must sit under the given root for the whole run: {seen:?}"
    );
    assert!(
        seen.iter().all(|(_, inside)| !*inside),
        "nothing is written inside the tree any more: {seen:?}"
    );
}
