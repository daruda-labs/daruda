//! What the run remembers about itself — every attempt, including the
//! ones that were canceled, archived badly, or belonged to a fix session.

use super::*;

/// The flagship repair path, seen from the record: `review` runs twice in
/// two generations, and both of the gate's attempts are kept. A record that
/// only remembers the last attempt cannot explain why a run took the path it
/// did, which is the whole reason `run.md` exists.
#[test]
fn a_repair_records_every_attempt_of_every_node_it_re_derived() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(GATED, &runner);

    let gate = report.node("gate").expect("gate ran");
    assert_eq!(gate.attempts.len(), 2);
    assert!(matches!(
        gate.attempts[0].outcome,
        AttemptOutcome::Failed(NodeFailure::Exit { code: Some(1) })
    ));
    assert!(matches!(gate.attempts[1].outcome, AttemptOutcome::Passed));
    // The archive happened on the failed attempt, so its paths belong to
    // that attempt and not to the one that passed.
    assert!(!gate.attempts[0].invalidated.archived.is_empty());
    assert!(gate.attempts[1].invalidated.archived.is_empty());

    let review = report.node("review").expect("review ran");
    assert_eq!(review.attempts.len(), 2, "re-derived once");
    // Both are attempt 1 — a fresh generation resets the node's counter,
    // and `evidence_seq` is what actually distinguishes them.
    assert_eq!(review.attempts[0].attempt, 1);
    assert_eq!(review.attempts[1].attempt, 1);
    assert_ne!(
        review.attempts[0].evidence_seq,
        review.attempts[1].evidence_seq
    );
}

/// The host answers, the engine only asks — and it asks per attempt,
/// because a node that ran three times may have left the tree in three
/// different states.
#[test]
fn the_tree_state_is_recorded_for_each_attempt_the_host_answers_for() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Owned, because `RunRequest` holds the callback as a `Box<dyn Fn>` and
    // cannot borrow a local (that would put a lifetime on the struct).
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = seen.clone();
    let git = move |_: &std::path::Path| {
        let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Some(format!(" M file-{n}.rs"))
    };
    let mut request = request_for(CHAIN, dir.path());
    request.git_status = Some(Box::new(git));

    let runner = FakeRunner::new();
    let report = execute(&request, &runner, &CancelToken::default());

    let statuses: Vec<_> = report
        .nodes
        .iter()
        .flat_map(|n| n.attempts.iter())
        .filter_map(|a| a.git_status.as_deref())
        .collect();
    assert_eq!(
        statuses,
        vec![" M file-1.rs", " M file-2.rs", " M file-3.rs"]
    );
    assert_eq!(
        seen.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "one ask per attempt, and no more"
    );
}

/// The attempt a user stopped is the one they open the record to read
/// about, and it never reaches `judge`.
#[test]
fn a_canceled_attempt_is_still_in_the_record() {
    let runner = FakeRunner::new().cancel_at("design", 1).script(
        "design",
        vec![Step::Ok {
            writes: Some("half a thought".to_string()),
        }],
    );
    let (report, _dir) = run(CHAIN, &runner);
    let design = report.node("design").expect("design ran");
    assert_eq!(design.attempts.len(), 1);
    assert!(matches!(
        design.attempts[0].outcome,
        AttemptOutcome::Canceled
    ));
    assert!(
        design.attempts[0]
            .invalidated
            .archived
            .iter()
            .any(|p| p.to_string_lossy().contains("canceled")),
        "{:?}",
        design.attempts[0].invalidated.archived
    );
}

/// The fix is a session the run paid for — it counts toward `node_runs` and
/// the cost budget. A record that omits it cannot be reconciled with either,
/// and a reader cannot tell a long fix from a stalled run.
#[test]
fn the_fix_session_is_in_the_record_like_any_other_attempt() {
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let (report, _dir) = run(GATED, &runner);
    let fix = report.node(FIX_SESSION_ID).expect("a fix ran");
    assert_eq!(fix.attempts.len(), 1);
    assert!(matches!(fix.attempts[0].outcome, AttemptOutcome::Passed));

    // This fixture has no correction turns, so every budget unit has a record.
    let recorded: usize = report.nodes.iter().map(|n| n.attempts.len()).sum();
    assert_eq!(recorded as u32, report.node_runs);
}

/// The attempt that ends a run returns before any invalidation set is
/// computed, so it archives nothing — and a record built at the archive site
/// would drop it entirely.
#[test]
fn the_attempt_that_ends_the_run_is_recorded_with_no_evidence() {
    let runner = FakeRunner::new().script("design", vec![Step::fail(NodeFailure::Refused)]);
    let (report, _dir) = run(CHAIN, &runner);
    let design = report.node("design").expect("design ran");
    assert_eq!(design.attempts.len(), 1);
    assert!(matches!(
        design.attempts[0].outcome,
        AttemptOutcome::Failed(NodeFailure::Refused)
    ));
    assert!(design.attempts[0].invalidated.archived.is_empty());
}

/// Archiving can fail after an attempt has already run and been counted.
/// The record has to keep that attempt anyway. This fixture has no
/// correction turn, so dropping it would also make the budget-unit and
/// attempt totals disagree.
#[test]
fn an_attempt_whose_archive_failed_is_still_in_the_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(
        "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        wait: 0s
        hint: retry
",
        dir.path(),
    );
    // `logs/` as a file, so the archive's `create_dir_all` cannot succeed.
    std::fs::create_dir_all(&request.run_dir).expect("mkdir");
    std::fs::write(request.run_dir.join(LOG_DIR_NAME), "not a directory").expect("write");

    let runner =
        FakeRunner::new().script("design", vec![Step::fail(NodeFailure::ContextExhausted)]);
    let report = execute(&request, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Io(_)),
        "{:?}",
        report.outcome
    );
    let recorded: usize = report.nodes.iter().map(|n| n.attempts.len()).sum();
    assert_eq!(
        recorded as u32, report.node_runs,
        "the attempt ran and was counted, so it must be recorded"
    );
    let design = report.node("design").expect("design ran");
    assert!(design.attempts[0].invalidated.archived.is_empty());
}
