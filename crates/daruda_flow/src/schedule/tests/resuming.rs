//! Picking a run up where a killed process left it.
//!
//! The crash is simulated by *cancelling* mid-run and then removing the
//! marker the cancel wrote: what a `kill -9` leaves is a directory with a
//! journal, a spec, some outputs and no marker, and that is exactly what
//! these build. Nothing here can be tested by resuming a run this process
//! finished — the whole question is what survives one that did not.

use super::*;

/// Three nodes in a chain, all commands, so a run costs nothing and every
/// pass is invisible on disk — which is the case a journal exists for.
const CHAIN_OF_THREE: &str = "\
version: 1
nodes:
  - id: one
    kind: command
    run: \"true\"
  - id: two
    kind: command
    deps: [one]
    run: \"true\"
  - id: three
    kind: command
    deps: [two]
    run: \"true\"
";

/// The first node retries before it passes, so the crash lands with the
/// evidence counter already past 1 and a node that has more than one
/// attempt to its name.
const RETRIES_THEN_CHAINS: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: one
    kind: agent
    output: one.md
    prompt: write it
    on_fail:
      retry:
        hint: again after {{failure}}
        max_attempts: 2
  - id: two
    kind: command
    deps: [one]
    run: \"true\"
";

/// Run until `stop_at` has run, then leave the directory the way a killed
/// process would: no marker, and the lock still naming a pid.
fn killed_after(
    dir: &std::path::Path,
    flow: &str,
    stop_at: &str,
    runner: FakeRunner,
) -> std::path::PathBuf {
    let runner = runner.cancel_at(stop_at, 1);
    let report = execute(&request_for(flow, dir), &runner, &CancelToken::default());
    for marker in ["DONE", "FAILED", "CANCELED"] {
        let _ = std::fs::remove_file(report.run_dir.join(marker));
    }
    // A lock naming this run, whose holder the resume will be told is gone.
    std::fs::write(
        report.run_dir.parent().expect("runs dir").join(".lock"),
        format!(
            "pid: 999999\nrun_id: {}\nstarted_unix_secs: 1\n",
            report.run_dir.file_name().expect("name").to_string_lossy()
        ),
    )
    .expect("lock");
    report.run_dir
}

/// Resume the run in `run_dir`, with the same fake runner the first half
/// used.
fn resume_run(run_dir: &std::path::Path, flow: &str, runner: &FakeRunner) -> RunReport {
    // One predicate answers both questions — whether the run is resumable
    // at all, and whether its stale lock may be reclaimed. A host that let
    // them disagree would offer a resume the lock then refuses.
    let dead = |_: u32| false;
    let resumed = crate::resume::prepare(run_dir, &dead).expect("a killed run is resumable");
    let dir = run_dir.parent().expect("runs").parent().expect("cwd");
    let mut request = request_for(flow, dir);
    request.loaded = resumed.loaded;
    request.run_dir = run_dir.to_path_buf();
    request.resume = Some(resumed.replay);
    request.is_alive = Box::new(dead);
    execute(&request, runner, &CancelToken::default())
}

/// The point of the whole feature: what already ran does not run again.
#[test]
fn a_resumed_run_does_not_repeat_what_the_first_half_finished() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());

    let runner = FakeRunner::new();
    let report = resume_run(&run_dir, CHAIN_OF_THREE, &runner);

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        runner.ids(),
        vec![crate::NodeId::from("two"), crate::NodeId::from("three")],
        "the resumed run re-ran a node that had already passed"
    );
}

/// The account of the run covers both halves. A resume that started its
/// record afresh would leave `run.md` beginning in the middle.
#[test]
fn the_finished_record_covers_the_half_before_the_crash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());

    let report = resume_run(&run_dir, CHAIN_OF_THREE, &FakeRunner::new());

    let ids: Vec<&str> = report.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["one", "two", "three"], "{ids:?}");
    // And the spend carries: `one` was a runner call somebody paid for.
    assert!(
        report.node_runs >= 3,
        "the first half's calls were forgotten: {}",
        report.node_runs
    );
}

/// Evidence numbers are never reused across the crash. The number is what
/// separates one attempt's log from another's — a resumed run that started
/// counting again would write over the only account of an attempt nobody
/// can re-run.
///
/// Asserted as "every number in the journal is distinct" rather than by
/// looking for an overwritten file: whether two attempts collide depends on
/// which nodes retried, so a file-level check passes for the wrong reason
/// on any fixture where they happen not to.
#[test]
fn the_resumed_half_never_reuses_an_evidence_number() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(
        dir.path(),
        RETRIES_THEN_CHAINS,
        "two",
        FakeRunner::new().script(
            "one",
            vec![
                Step::fail(NodeFailure::TurnFailed("first try".to_string())),
                Step::Ok {
                    writes: Some("one\n".to_string()),
                },
            ],
        ),
    );

    resume_run(&run_dir, RETRIES_THEN_CHAINS, &FakeRunner::new());

    let seqs: Vec<u32> = crate::journal::read(&run_dir)
        .records
        .iter()
        .flat_map(|node| node.attempts.iter().map(|a| a.evidence_seq))
        .collect();
    let mut unique = seqs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        seqs.len(),
        unique.len(),
        "an evidence number came round again: {seqs:?}"
    );
    assert!(
        seqs.len() >= 4,
        "too few attempts to be evidence of anything: {seqs:?}"
    );
}

/// Retention never takes a run that could still be picked up. It sweeps by
/// marker, and a killed run has none — so this holds today for a reason
/// that has nothing to do with resume, which is exactly why it needs
/// stating here: the next person to make the sweep smarter has to know
/// this is load-bearing.
#[test]
fn the_sweep_leaves_a_killed_run_where_it_is() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());
    let runs = run_dir.parent().expect("runs dir");
    // Well past any retention: every other run may go.
    let removed = crate::marker::sweep_old_runs(runs, 0).expect("sweep");

    assert!(run_dir.is_dir(), "the killed run was swept away");
    assert!(!removed.contains(&run_dir), "{removed:?}");
}

/// The wall clock starts over, and what was spent does not.
///
/// A resumed run is given a fresh deadline measured from now. Carrying the
/// earlier half's waiting into it would add time for a wait already behind
/// the run — while forgetting its runner calls would let a `max_node_runs`
/// budget run its whole length a second time.
#[test]
fn a_resume_carries_the_spend_and_not_the_waiting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());

    let dead = |_: u32| false;
    let resumed = crate::resume::prepare(&run_dir, &dead).expect("resumable");
    let spent_before = resumed.replay.spent.node_runs;
    assert!(spent_before > 0, "the first half ran nothing");

    let mut request = request_for(CHAIN_OF_THREE, dir.path());
    request.loaded = resumed.loaded;
    request.run_dir = run_dir.clone();
    request.resume = Some(resumed.replay);
    request.is_alive = Box::new(dead);
    // One more call than the run needs, so the ceiling is reached only if
    // the first half's calls counted.
    request.budget = Budget {
        max_node_runs: Some(spent_before + 1),
        ..Budget::unlimited()
    };

    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::NodeRuns
            }
        ),
        "the first half's runner calls were forgotten: {:?}",
        report.outcome
    );
}

/// The other half of the same rule, and the one a fixture that never waits
/// cannot show: waiting done before the crash must not buy time after it.
///
/// The resumed run is given a deadline that has already passed, so it must
/// stop at once. If the earlier half's waiting were carried, it would be
/// added to that deadline and the run would sail past it — having been
/// handed an hour for a wait somebody finished yesterday.
#[test]
fn waiting_done_before_the_crash_does_not_extend_the_new_clock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(
        dir.path(),
        CHAIN_OF_THREE,
        "two",
        FakeRunner::new().parked_per_call(Duration::from_secs(30)),
    );

    let dead = |_: u32| false;
    let resumed = crate::resume::prepare(&run_dir, &dead).expect("resumable");
    assert!(
        resumed.replay.spent.parked >= Duration::from_secs(30),
        "the first half's waiting did not read back: {:?}",
        resumed.replay.spent.parked
    );

    let mut request = request_for(CHAIN_OF_THREE, dir.path());
    request.loaded = resumed.loaded;
    request.run_dir = run_dir.clone();
    request.resume = Some(resumed.replay);
    request.is_alive = Box::new(dead);
    request.budget = Budget {
        deadline: Some(std::time::Instant::now() - Duration::from_secs(10)),
        ..Budget::unlimited()
    };

    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());
    assert!(
        matches!(
            report.outcome,
            RunOutcome::BudgetExhausted {
                limit: BudgetLimit::WallClock
            }
        ),
        "the new clock was pushed out by the old half's waiting: {:?}",
        report.outcome
    );
}

/// A resumed run says it was resumed.
///
/// Written after a real `kill -9` test where the artifacts could not answer
/// the one question the test existed to ask: the run directory looked the
/// same whether it had been picked up or had simply run through. A feature
/// whose whole point is continuing a run has to leave that on the record.
#[test]
fn a_continued_run_says_so_on_its_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());

    let report = resume_run(&run_dir, CHAIN_OF_THREE, &FakeRunner::new());

    assert_eq!(
        report.provenance.carried_over, 1,
        "`one` was not counted as carried"
    );
    let rendered = crate::record::render_run_md(&report);
    assert!(
        rendered.contains("**Continued** — picked up with 1 node(s) already done"),
        "the report knows, the record does not say: {rendered}"
    );
    // And the journal keeps the boundary, for whoever reads the directory
    // rather than the report.
    let journal =
        std::fs::read_to_string(run_dir.join(crate::journal::JOURNAL_FILE)).expect("journal");
    assert!(
        journal.contains("\"kind\":\"resumed\""),
        "the journal does not show where it was taken over: {journal}"
    );
}

/// A run that started at its first node says nothing about being
/// continued — the line is a fact about this run, not decoration.
#[test]
fn a_run_that_started_normally_claims_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = execute(
        &request_for(CHAIN_OF_THREE, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );
    assert_eq!(report.provenance.carried_over, 0);
    assert!(!crate::record::render_run_md(&report).contains("**Continued**"));
}

/// Every attempt says when it settled, and the resume gap is visible as a
/// gap between two of them.
///
/// Written because reading a real killed-and-continued run meant opening
/// the *file timestamps* to answer "when did the second half start" — the
/// record itself only gave an order.
#[test]
fn the_record_dates_every_attempt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = killed_after(dir.path(), CHAIN_OF_THREE, "two", FakeRunner::new());
    let report = resume_run(&run_dir, CHAIN_OF_THREE, &FakeRunner::new());

    let rendered = crate::record::render_run_md(&report);
    let dated = rendered.matches(" at 20").count();
    assert_eq!(
        dated,
        report.nodes.iter().map(|n| n.attempts.len()).sum::<usize>(),
        "an attempt went undated: {rendered}"
    );

    // And the two halves are hours apart in principle — here only
    // milliseconds, but the first half's stamps must not be *later* than
    // the second's.
    let mut stamps: Vec<std::time::SystemTime> = report
        .nodes
        .iter()
        .flat_map(|n| n.attempts.iter().map(|a| a.at))
        .collect();
    let ordered = stamps.clone();
    stamps.sort();
    assert_eq!(
        ordered, stamps,
        "the record's attempts are not in the order they settled"
    );
}
