//! What a resume does before it hands the run back to the scheduler (§5.5):
//! under the lock, close what the dead process left open, clear its marker,
//! and check its passes still hold.

use super::crash::{Process, process, process_asking, records};
use super::*;
use crate::run_log::machine::{AbandonReason, FlowRunEvent};
use crate::runner::{AskRequest, Permission};
use daruda_runlog::testing::{Fault, FaultBackend};

/// Leave the run in `run_dir` the way a killed process would: no end in its
/// log, no marker, and a lock naming a pid that is gone.
pub(super) fn as_if_killed(request: &crate::request::RunRequest, run_dir: &Path) {
    crate::run_log::testing::forget_stop(run_dir);
    for marker in ["DONE", "FAILED", "CANCELED"] {
        let _ = std::fs::remove_file(run_dir.join(marker));
    }
    let lock_dir = super::lock_dir_of(request);
    std::fs::create_dir_all(&lock_dir).expect("lock dir");
    std::fs::write(
        lock_dir.join(".lock"),
        format!(
            "pid: 999999\nrun_id: {}\nstarted_unix_secs: 1\n",
            run_dir.file_name().expect("name").to_string_lossy()
        ),
    )
    .expect("lock");
}

/// The request that continues `run_dir`, as a host builds it.
pub(super) fn resume_request(
    text: &str,
    cwd: &Path,
    run_dir: &Path,
) -> Result<crate::request::RunRequest, crate::resume::ResumeError> {
    let mut request = request_for(text, cwd);
    let dead = |_: u32| false;
    let resumed = crate::resume::prepare(run_dir, Some(&super::lock_dir_of(&request)), &dead)?;
    request.resume = Some(at_once(&resumed));
    request.loaded = resumed.loaded;
    request.run_dir = run_dir.to_path_buf();
    request.is_alive = Box::new(dead);
    Ok(request)
}

/// **F8.** A stalled run picked up again stops reading as stalled: its
/// marker goes, and how it ends this time is what it says.
#[test]
fn a_resume_clears_the_stalled_marker() {
    const ONLY_A_SUBDIR: &str = "\
version: 1
nodes:
  - id: there
    kind: command
    cwd: sub
    run: \"true\"
";
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(ONLY_A_SUBDIR, None).expect("valid flow");
    let run_dir = dir.path().join("run");
    let log_dir = crate::run_log::log_dir(&run_dir);
    std::fs::create_dir_all(&log_dir).expect("mkdir");
    // `sub` is missing, so the node cannot be placed and the run stalls.
    let Process::Drove(report) = process(
        daruda_runlog::FileBackend::new(log_dir),
        &loaded,
        dir.path(),
        &FakeRunner::new(),
    ) else {
        panic!("the run did not start");
    };
    assert!(matches!(report.outcome, RunOutcome::Stalled { .. }));
    crate::marker::write_marker(&run_dir, &report.outcome).expect("marker");
    std::fs::write(run_dir.join(crate::resume::RUN_SPEC_FILE), ONLY_A_SUBDIR).expect("spec");
    std::fs::create_dir_all(dir.path().join("sub")).expect("now it exists");

    let request = resume_request(ONLY_A_SUBDIR, dir.path(), &run_dir).expect("stalled resumes");
    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert!(!run_dir.join("STALLED").exists(), "the stale marker stayed");
    assert_eq!(
        crate::marker::run_status(&run_dir, Some(&super::lock_dir_of(&request)), &|_| true),
        crate::marker::RunStatus::Done
    );
}

/// Asks once per attempt for `design`, through the run's port.
struct Asks(FakeRunner);

impl NodeRunner for Asks {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            if let Permission::Ask(port) = &ctx.permission {
                let request = AskRequest {
                    tool: "write".to_string(),
                    detail: None,
                    options: Vec::new(),
                };
                port.ask(request).await;
            }
            self.0.run_agent(ctx, agent, prompt).await
        })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.0.run_command(ctx, run)
    }
}

/// **B4, B5.** A question open when the process died is closed as
/// abandoned on resume; the next question takes the next id, and the run
/// can still end.
#[test]
fn a_question_left_open_by_a_crash_is_abandoned_and_ids_continue() {
    let flow = "\
version: 1
defaults: { agent: { id: claude, mode: default, permission: ask } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry: { max_attempts: 3, wait: 0s, hint: again }
";
    let loaded = load(flow, None).expect("valid flow");
    let dir = tempfile::tempdir().expect("tempdir");
    let (ask_tx, ask_rx) = smol::channel::unbounded::<crate::runner::PendingAsk>();
    let host = std::thread::spawn(move || {
        let mut ids = Vec::new();
        while let Ok(pending) = ask_rx.recv_blocking() {
            ids.push(pending.ask_id);
            let _ = pending.answer(daruda_acp::PermissionDecision::Allow {
                option_id: "allow".to_string(),
            });
        }
        ids
    });

    // opened 0, run_started 1, attempt_started 2, ask_raised 3: the answer
    // never reaches the log.
    let first = FaultBackend::new().with_fault(Fault::TornAppend { k: 4, keep: 0 });
    let _ = process_asking(
        first.clone(),
        &loaded,
        dir.path(),
        &Asks(FakeRunner::new()),
        Some(&ask_tx),
    );
    let second = first.crash();
    let Process::Drove(report) = process_asking(
        second.clone(),
        &loaded,
        dir.path(),
        &Asks(FakeRunner::new()),
        Some(&ask_tx),
    ) else {
        panic!("the run could not be picked up");
    };
    drop(ask_tx);
    let asked = host.join().expect("the host");

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(asked, vec![1, 2], "the second question reused an id");
    let events = records(&second);
    assert!(events.iter().any(|event| matches!(
        event,
        FlowRunEvent::AskAbandoned {
            id: 1,
            reason: AbandonReason::AppRestart
        }
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, FlowRunEvent::RunEnded { .. })),
        "the run could not record its end"
    );
}

/// **M9.** A pinned node and a command node are not re-run on resume, and
/// neither is an output a resume checks for.
#[test]
fn a_pin_and_a_command_survive_a_resume_untouched() {
    const PIN_COMMAND_AGENT: &str = "\
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
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("earlier-design.md");
    std::fs::write(&source, "reused\n").expect("the earlier output");
    let mut request = request_for(PIN_COMMAND_AGENT, dir.path());
    request.pinned = vec![crate::request::PinnedOutput {
        node: "design".into(),
        from: source,
    }];
    let first = execute(
        &request,
        &FakeRunner::new().cancel_at("review", 1),
        &CancelToken::default(),
    );
    as_if_killed(&request, &first.run_dir);

    let resumed = resume_request(PIN_COMMAND_AGENT, dir.path(), &first.run_dir)
        .expect("a killed run is resumable");
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.ids(), vec![NodeId::from("review")]);
    let log = std::fs::read_to_string(crate::run_log::log_file(&first.run_dir)).expect("log");
    assert!(!log.contains("\"type\":\"output_lost\""), "{log}");
}

/// **F14.** A resume reads the run only once it holds the lock: refused as
/// held, it leaves the log and the markers exactly as they were.
#[test]
fn a_resume_refused_by_the_lock_reads_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let first = execute(
        &request,
        &FakeRunner::new().cancel_at("test", 1),
        &CancelToken::default(),
    );
    as_if_killed(&request, &first.run_dir);
    let mut resumed =
        resume_request(CHAIN, dir.path(), &first.run_dir).expect("a killed run is resumable");
    let log = crate::run_log::log_file(&first.run_dir);
    let before = std::fs::read(&log).expect("log");
    // Someone else took the lane between the prompt and the click.
    resumed.is_alive = Box::new(|_| true);

    let report = execute(&resumed, &FakeRunner::new(), &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::LockHeld { .. }),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        std::fs::read(&log).expect("log"),
        before,
        "the log was touched"
    );
}

/// **M7.** A run an older build started, and that build's process died,
/// is taken over from its journal and finished — not started over.
#[test]
fn a_run_an_older_build_started_is_resumed_from_its_journal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let run_dir = request.run_dir.clone();
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    std::fs::write(run_dir.join(crate::resume::RUN_SPEC_FILE), CHAIN).expect("spec");
    crate::journal::start(&run_dir, None, None, &[]).expect("start");
    let passed = crate::record::AttemptRecord {
        attempt: 1,
        evidence_seq: 1,
        outcome: crate::record::AttemptOutcome::Passed,
        invalidated: crate::record::Invalidation::default(),
        git_status: None,
        at: std::time::UNIX_EPOCH,
        took: Duration::ZERO,
        waited: crate::runner::Waiting::default(),
        tools: Vec::new(),
        turns: 1,
        usage: None,
    };
    let spent = crate::run_log::reader::Spent {
        node_runs: 1,
        ..Default::default()
    };
    crate::journal::append_attempt(&run_dir, &"design".into(), &passed, &spent).expect("line");
    std::fs::write(run_dir.join("design.md"), "design ran\n").expect("output");
    as_if_killed_without_log(&request, &run_dir);

    let resumed = resume_request(CHAIN, dir.path(), &run_dir).expect("a killed run is resumable");
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        runner.ids(),
        vec![NodeId::from("test"), NodeId::from("review")]
    );
    assert_eq!(report.provenance.carried_over, 1);
    assert_eq!(report.node("design").expect("carried").attempts.len(), 1);
    assert!(crate::run_log::reader::has_log(&run_dir));
    // Evidence continues past the journal's.
    let replay = crate::run_log::reader::read(&run_dir);
    assert_eq!(replay.next_seq, 4);
}

/// One node allowed two attempts, for the journal tests below.
const RETRY_TWICE: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
    on_fail:
      retry: { max_attempts: 2, wait: 0s, hint: again }
";

/// An older build's run whose `design` attempts all failed, journaled.
fn journal_of_failures(
    dir: &Path,
    failures: u32,
) -> (crate::request::RunRequest, std::path::PathBuf) {
    let request = request_for(RETRY_TWICE, dir);
    let run_dir = request.run_dir.clone();
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    std::fs::write(run_dir.join(crate::resume::RUN_SPEC_FILE), RETRY_TWICE).expect("spec");
    crate::journal::start(&run_dir, None, None, &[]).expect("start");
    for attempt in 1..=failures {
        let failed = crate::record::AttemptRecord {
            attempt,
            evidence_seq: attempt,
            outcome: crate::record::AttemptOutcome::Reported("timed out after 9 seconds".into()),
            invalidated: crate::record::Invalidation::default(),
            git_status: None,
            at: std::time::UNIX_EPOCH,
            took: Duration::ZERO,
            waited: crate::runner::Waiting::default(),
            tools: Vec::new(),
            turns: 1,
            usage: None,
        };
        let spent = crate::run_log::reader::Spent {
            node_runs: attempt,
            ..Default::default()
        };
        crate::journal::append_attempt(&run_dir, &"design".into(), &failed, &spent).expect("line");
    }
    as_if_killed_without_log(&request, &run_dir);
    (request, run_dir)
}

/// **F10 for imported runs.** A failure the older build journaled still
/// counts: with one of two attempts spent, the resume has one left.
#[test]
fn an_imported_run_keeps_the_attempts_its_journal_spent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, run_dir) = journal_of_failures(dir.path(), 1);

    let resumed = resume_request(RETRY_TWICE, dir.path(), &run_dir).expect("resumable");
    let runner = FakeRunner::new().script(
        "design",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::fail(NodeFailure::Exit { code: Some(1) }),
        ],
    );
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Failed { .. }),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        runner.ids(),
        vec![NodeId::from("design")],
        "a spent try ran again"
    );
    let design = crate::run_log::reader::read(&run_dir);
    assert_eq!(
        design.next_seq, 3,
        "evidence did not continue past the journal's"
    );
}

/// An imported node whose journal spent every attempt fails the resume
/// with the reason it was journaled with, and runs nothing.
#[test]
fn an_imported_node_with_no_attempts_left_fails_as_journaled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, run_dir) = journal_of_failures(dir.path(), 2);

    let resumed = resume_request(RETRY_TWICE, dir.path(), &run_dir).expect("resumable");
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(runner.ids().is_empty(), "{:?}", runner.ids());
    let RunOutcome::Failed { node, failure } = &report.outcome else {
        panic!("{:?}", report.outcome);
    };
    assert_eq!(node, &NodeId::from("design"));
    assert_eq!(failure.to_string(), "timed out after 9 seconds");
}

/// **Quit is not a stop.** The app quitting mid-node records no ending, no
/// marker and no settle, and keeps the lock: the run reads as crashed, and
/// a resume re-runs the cut attempt without spending a try on it (D4-2).
#[test]
fn a_quit_mid_node_leaves_the_run_to_resume_like_a_crash() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let first = execute(
        &request,
        &FakeRunner::new().shuts_down_at("test", 1),
        &CancelToken::default(),
    );

    assert!(
        matches!(first.outcome, RunOutcome::ShutDown),
        "{:?}",
        first.outcome
    );
    let state = crate::run_log::reader::state(&first.run_dir).expect("a log");
    assert_eq!(
        state.phase(),
        crate::run_log::machine::RunPhase::Running,
        "a quit recorded an ending"
    );
    for marker in ["DONE", "FAILED", "CANCELED", "STALLED"] {
        assert!(!first.run_dir.join(marker).exists(), "{marker} was written");
    }
    let lock_dir = super::lock_dir_of(&request);
    assert!(
        crate::lock::read_holder(&lock_dir).is_some(),
        "the lock was given back, so the run would read as Unknown"
    );
    let status = crate::marker::run_status(&first.run_dir, Some(&lock_dir), &|_| false);
    assert_eq!(status, crate::marker::RunStatus::Crashed);

    let resumed = resume_request(CHAIN, dir.path(), &first.run_dir).expect("resumable");
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let test = runner
        .calls()
        .into_iter()
        .find(|call| call.node.as_str() == "test")
        .expect("test ran again");
    assert_eq!(test.attempt, 2, "the cut attempt's number was reused");
}

/// A Stop that lands after the app began to quit leaves it a quit.
#[test]
fn a_stop_after_a_quit_does_not_end_the_run() {
    let cancel = CancelToken::default();
    cancel.shut_down();
    cancel.cancel();
    assert_eq!(cancel.kind(), Some(crate::runner::StopKind::ShutDown));
}

fn as_if_killed_without_log(request: &crate::request::RunRequest, run_dir: &Path) {
    let lock_dir = super::lock_dir_of(request);
    std::fs::create_dir_all(&lock_dir).expect("lock dir");
    std::fs::write(
        lock_dir.join(".lock"),
        format!(
            "pid: 999999\nrun_id: {}\nstarted_unix_secs: 1\n",
            run_dir.file_name().expect("name").to_string_lossy()
        ),
    )
    .expect("lock");
}

/// A resume right after the crash says it is waiting, then waits out the
/// rest of the window before its first node: an agent the dead run started
/// can outlive it by seconds.
#[test]
fn a_resume_right_after_a_crash_waits_for_what_it_left() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let first = execute(
        &request,
        &FakeRunner::new().cancel_at("test", 1),
        &CancelToken::default(),
    );
    as_if_killed(&request, &first.run_dir);
    let mut resumed =
        resume_request(CHAIN, dir.path(), &first.run_dir).expect("a killed run is resumable");
    let settle = Duration::from_secs(2);
    resumed.resume.as_mut().expect("a continuation").settle = settle;
    let (events, notices) = smol::channel::unbounded();
    resumed.events = Some(events);

    let began = std::time::Instant::now();
    let report = execute(&resumed, &FakeRunner::new(), &CancelToken::default());
    let took = began.elapsed();

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    let first_notice = notices.try_recv().expect("a notice").into_event();
    let FlowEvent::Settling { wait } = first_notice else {
        panic!("the resume did not say it was waiting: {first_notice:?}");
    };
    assert!(!wait.is_zero() && wait <= settle, "{wait:?}");
    assert!(took >= wait, "it said {wait:?} and took {took:?}");
}

/// **O3.** A resume that died after opening the log but before importing
/// the older build's journal leaves a log holding nothing of the run. The
/// journal is still the record, so the next resume imports it rather than
/// refusing the run as never started.
#[test]
fn a_resume_killed_before_its_journal_import_imports_it_next_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let request = request_for(CHAIN, dir.path());
    let run_dir = request.run_dir.clone();
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    std::fs::write(run_dir.join(crate::resume::RUN_SPEC_FILE), CHAIN).expect("spec");
    crate::journal::start(&run_dir, None, Some(&"test".into()), &[]).expect("start");
    let passed = crate::record::AttemptRecord {
        attempt: 1,
        evidence_seq: 1,
        outcome: crate::record::AttemptOutcome::Passed,
        invalidated: crate::record::Invalidation::default(),
        git_status: None,
        at: std::time::UNIX_EPOCH,
        took: Duration::ZERO,
        waited: crate::runner::Waiting::default(),
        tools: Vec::new(),
        turns: 1,
        usage: None,
    };
    let spent = crate::run_log::reader::Spent {
        node_runs: 1,
        ..Default::default()
    };
    crate::journal::append_attempt(&run_dir, &"design".into(), &passed, &spent).expect("line");
    std::fs::write(run_dir.join("design.md"), "design ran\n").expect("output");
    // The killed resume: its log opened, its import never committed.
    drop(crate::run_log::store::open_file_log(&run_dir).unwrap_or_else(|_| panic!("open")));
    as_if_killed_without_log(&request, &run_dir);

    let resumed = resume_request(CHAIN, dir.path(), &run_dir).expect("still resumable");
    assert_eq!(
        resumed.resume.as_ref().and_then(|c| c.until.clone()),
        Some(NodeId::from("test")),
        "the selection the journal recorded was lost"
    );
    let runner = FakeRunner::new();
    let report = execute(&resumed, &runner, &CancelToken::default());

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.ids(), vec![NodeId::from("test")]);
    assert_eq!(report.provenance.carried_over, 1);
}

/// The log of a killed run whose last record was written just now, and the
/// request that resumes it after waiting out `settle`.
fn freshly_killed(dir: &Path, settle: Duration) -> crate::request::RunRequest {
    let request = request_for(CHAIN, dir);
    let first = execute(
        &request,
        &FakeRunner::new().cancel_at("test", 1),
        &CancelToken::default(),
    );
    as_if_killed(&request, &first.run_dir);
    let mut resumed =
        resume_request(CHAIN, dir, &first.run_dir).expect("a killed run is resumable");
    resumed.resume.as_mut().expect("a continuation").settle = settle;
    resumed
}

/// A Stop while the resume waits ends the wait at once, and the run was
/// never taken: nothing is written, and it can still be resumed.
#[test]
fn a_stop_during_the_settle_wait_cuts_it_short_and_leaves_the_run_resumable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let settle = Duration::from_secs(60);
    let mut resumed = freshly_killed(dir.path(), settle);
    let (events, notices) = smol::channel::unbounded();
    resumed.events = Some(events);
    let log = crate::run_log::log_file(&resumed.run_dir);
    let before = std::fs::read(&log).expect("log");
    let cancel = CancelToken::default();
    let stopper = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            let notice = smol::block_on(notices.recv()).expect("a notice");
            assert!(
                matches!(notice, crate::event::RunNotice::Settling { .. }),
                "{notice:?}"
            );
            cancel.cancel();
        })
    };

    let began = std::time::Instant::now();
    let report = execute(&resumed, &FakeRunner::new(), &cancel);
    let took = began.elapsed();
    stopper.join().expect("stopper");

    assert!(took < settle / 2, "the stop did not cut the wait: {took:?}");
    assert!(
        matches!(report.outcome, RunOutcome::Canceled { node: None }),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        std::fs::read(&log).expect("log"),
        before,
        "the log was touched"
    );
    let lock_dir = super::lock_dir_of(&resumed);
    assert_eq!(
        crate::marker::run_status(&resumed.run_dir, Some(&lock_dir), &|_| false),
        crate::marker::RunStatus::Crashed,
    );

    resumed.events = None;
    resumed.resume.as_mut().expect("a continuation").settle = Duration::ZERO;
    let again = execute(&resumed, &FakeRunner::new(), &CancelToken::default());
    assert!(
        matches!(again.outcome, RunOutcome::Done),
        "{:?}",
        again.outcome
    );
}

/// A run whose log recorded its end left nothing running, so its resume
/// does not wait however recently it ended.
#[test]
fn a_run_that_recorded_its_end_does_not_wait() {
    const ONLY_A_SUBDIR: &str = "\
version: 1
nodes:
  - id: there
    kind: command
    cwd: sub
    run: \"true\"
";
    let dir = tempfile::tempdir().expect("tempdir");
    let loaded = load(ONLY_A_SUBDIR, None).expect("valid flow");
    let run_dir = dir.path().join("run");
    let log_dir = crate::run_log::log_dir(&run_dir);
    std::fs::create_dir_all(&log_dir).expect("mkdir");
    let Process::Drove(report) = process(
        daruda_runlog::FileBackend::new(log_dir),
        &loaded,
        dir.path(),
        &FakeRunner::new(),
    ) else {
        panic!("the run did not start");
    };
    assert!(matches!(report.outcome, RunOutcome::Stalled { .. }));
    std::fs::write(run_dir.join(crate::resume::RUN_SPEC_FILE), ONLY_A_SUBDIR).expect("spec");
    std::fs::create_dir_all(dir.path().join("sub")).expect("now it exists");
    let mut request = resume_request(ONLY_A_SUBDIR, dir.path(), &run_dir).expect("resumable");
    let settle = Duration::from_secs(60);
    request.resume.as_mut().expect("a continuation").settle = settle;
    let (events, notices) = smol::channel::unbounded();
    request.events = Some(events);

    let began = std::time::Instant::now();
    let report = execute(&request, &FakeRunner::new(), &CancelToken::default());

    assert!(began.elapsed() < settle / 2, "a stalled run waited");
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    while let Ok(notice) = notices.try_recv() {
        assert!(
            !matches!(notice, crate::event::RunNotice::Settling { .. }),
            "a stalled run said it was waiting"
        );
    }
}

/// **F7, S10.** A command's process group outlives a host killed while it
/// ran. A resume ends it before the node runs again, and only when the
/// recorded identity says the pid is still that command.
#[cfg(unix)]
mod orphans {
    use super::*;

    const ONE_COMMAND: &str = "\
version: 1
nodes:
  - id: build
    kind: command
    run: \"true\"
";

    /// A `sleep` standing in for the command the dead run left, ended on
    /// drop whatever the test found.
    struct Orphan(std::process::Child);

    impl Orphan {
        fn spawn() -> Self {
            let mut command = daruda_core::process::command("sleep");
            command.arg("600");
            daruda_core::process::lead_own_group(&mut command);
            Self(command.spawn().expect("spawn sleep"))
        }

        fn identity(&self) -> u64 {
            daruda_core::process::identity(self.0.id())
                .expect("a live child has an identity")
                .to_raw()
        }

        /// Whether it ended within a few seconds.
        fn ended(&mut self) -> bool {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if self.0.try_wait().expect("try_wait").is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            false
        }
    }

    impl Drop for Orphan {
        fn drop(&mut self) {
            // SILENT-OK: it may already be gone, which is what most tests want.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// The log a host killed mid-`build` leaves: the attempt started and its
    /// group recorded, nothing settled.
    fn killed_mid_command(dir: &Path, leader: u32, identity: u64) -> crate::load::LoadedFlow {
        let loaded = load(ONE_COMMAND, None).expect("valid flow");
        let log_dir = crate::run_log::log_dir(&dir.join("run"));
        std::fs::create_dir_all(&log_dir).expect("mkdir");
        let backend = daruda_runlog::FileBackend::new(log_dir.clone());
        let Ok(mut log) = crate::run_log::store::open_log(backend, &log_dir) else {
            panic!("the log did not open");
        };
        for event in [
            FlowRunEvent::RunStarted {
                nodes: crate::run_log::node_starts(&loaded),
                until: None,
                pinned: Vec::new(),
                profile: None,
            },
            FlowRunEvent::attempt_started("build".into(), 0),
            FlowRunEvent::ProcessSpawned {
                node: "build".into(),
                evidence: 1,
                leader,
                identity,
            },
        ] {
            log.commit(event).expect("commit");
        }
        loaded
    }

    fn resume(dir: &Path, loaded: &crate::load::LoadedFlow) {
        let log_dir = crate::run_log::log_dir(&dir.join("run"));
        let backend = daruda_runlog::FileBackend::new(log_dir);
        let Process::Drove(report) = process(backend, loaded, dir, &FakeRunner::new()) else {
            panic!("the run could not be picked up");
        };
        // The crashed attempt spent no try, so the node runs again (D4-2).
        assert!(
            matches!(report.outcome, RunOutcome::Done),
            "{:?}",
            report.outcome
        );
    }

    #[test]
    fn a_resume_ends_the_group_its_crashed_run_left() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut orphan = Orphan::spawn();
        let loaded = killed_mid_command(dir.path(), orphan.0.id(), orphan.identity());

        resume(dir.path(), &loaded);

        assert!(orphan.ended(), "the crashed run's command is still running");
    }

    /// The pid is someone else's now: the identity recorded with it differs
    /// from what runs under it, so it is left alone.
    #[test]
    fn a_reused_pid_is_never_killed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut bystander = Orphan::spawn();
        let other = bystander.identity() + 1;
        let loaded = killed_mid_command(dir.path(), bystander.0.id(), other);

        resume(dir.path(), &loaded);

        assert!(
            bystander.0.try_wait().expect("try_wait").is_none(),
            "a process that only shares the pid was signalled"
        );
    }
}
