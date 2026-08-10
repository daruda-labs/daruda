//! Stopping. A cancel is not a failure: it must not trigger `on_fail`,
//! and it must leave the run directory holding only completed work.

use super::*;

/// Cancel lands while `design` is in flight. The node it interrupted must
/// not leave a live output behind: everything alive in the run directory
/// has to be a completed node's.
#[test]
fn a_cancel_mid_node_archives_that_nodes_output_and_stops() {
    let runner = FakeRunner::new().cancel_at("design", 1).script(
        "design",
        vec![Step::Ok {
            writes: Some("half a thought".to_string()),
        }],
    );
    let (report, dir) = run(CHAIN, &runner);

    match report.outcome {
        RunOutcome::Canceled { node } => assert_eq!(node.as_deref(), Some("design")),
        other => panic!("expected Canceled, got {other:?}"),
    }
    let run_dir = dir.path().join("run");
    assert!(
        !run_dir.join("design.md").exists(),
        "the interrupted output must not stay live"
    );
    assert_eq!(
        std::fs::read_to_string(run_dir.join("logs/design.canceled.md")).expect("archived"),
        "half a thought"
    );
    assert_eq!(runner.ids(), vec!["design"], "test and review never run");
}

/// Cancelling is not failing, so a node's `on_fail` must stay out of it —
/// a retry policy would otherwise re-run the node the user just stopped.
#[test]
fn a_cancel_does_not_trigger_the_retry_policy() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 3
        wait: 0s
        hint: retry
";
    let runner = FakeRunner::new()
        .cancel_at("design", 1)
        .script("design", vec![Step::Ok { writes: None }]);
    let (report, _dir) = run(flow, &runner);
    // The attribution is the assertion that bites. A cancel judged as a
    // failure first still stops the run, but one attempt later and from the
    // loop's own check — which reports no node, because by then nothing is
    // in flight. `Some("design")` is only reachable by leaving `judge` out.
    match report.outcome {
        RunOutcome::Canceled { node } => assert_eq!(node.as_deref(), Some("design")),
        other => panic!("expected Canceled, got {other:?}"),
    }
    assert_eq!(runner.ids(), vec!["design"], "no retry after a cancel");
}

/// A token cancelled before the run starts stops it at the first boundary,
/// with no node attributed — nothing was interrupted.
#[test]
fn a_run_cancelled_before_it_starts_touches_no_node() {
    let cancel = CancelToken::default();
    cancel.cancel();
    let runner = FakeRunner::new();
    let (report, _dir) = run_with_cancel(CHAIN, &runner, &cancel);
    match report.outcome {
        RunOutcome::Canceled { node } => assert!(node.is_none(), "{node:?}"),
        other => panic!("expected Canceled, got {other:?}"),
    }
    assert!(runner.calls().is_empty());
}

/// The wait is the longest stretch a run spends idle, and the repair that
/// follows it opens a fresh agent session. A stop that lands in between
/// must be seen before that session is paid for.
#[test]
fn a_cancel_during_the_wait_stops_before_the_fix_session_runs() {
    let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        max_attempts: 3
        wait: 60ms
";
    let runner = FakeRunner::new().cancel_at("gate", 1).script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let (report, _dir) = run(flow, &runner);
    assert!(
        matches!(report.outcome, RunOutcome::Canceled { .. }),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.ids(), vec!["gate"], "no fix session after a cancel");
}

/// A cancel that interrupts the fix session is a stop, not a failure of
/// the fix — reporting `Failed { node: __fix__ }` would name the wrong
/// reason for the run ending.
#[test]
fn a_cancel_during_the_fix_session_is_not_a_failed_fix() {
    let runner = FakeRunner::new().cancel_at("__fix__", 1).script(
        "gate",
        vec![Step::fail(NodeFailure::Exit { code: Some(1) })],
    );
    let (report, _dir) = run(GATED, &runner);
    match &report.outcome {
        RunOutcome::Canceled { node } => assert_eq!(node.as_deref(), Some("__fix__")),
        other => panic!("expected Canceled, got {other:?}"),
    }
    assert_eq!(
        runner.ids(),
        vec!["implement", "review", "gate", "__fix__"],
        "nothing is re-derived after the stop"
    );
    // A stop is a third fate for the fix session, and one the run still paid
    // for. Left out of the record, the totals stop agreeing.
    let fix = report.node(FIX_SESSION_ID).expect("a fix ran");
    assert!(matches!(fix.attempts[0].outcome, AttemptOutcome::Canceled));
    let recorded: usize = report.nodes.iter().map(|n| n.attempts.len()).sum();
    assert_eq!(recorded as u32, report.node_runs);
}
