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
fn as_if_killed(request: &crate::request::RunRequest, run_dir: &Path) {
    crate::run_log::testing::forget_end(run_dir);
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
fn resume_request(
    text: &str,
    cwd: &Path,
    run_dir: &Path,
) -> Result<crate::request::RunRequest, crate::resume::ResumeError> {
    let mut request = request_for(text, cwd);
    let dead = |_: u32| false;
    let resumed = crate::resume::prepare(run_dir, Some(&super::lock_dir_of(&request)), &dead)?;
    request.resume = Some(resumed.continuation());
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
    let log = std::fs::read_to_string(crate::run_log::log_dir(&first.run_dir).join("log.0.jsonl"))
        .expect("log");
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
    let log = crate::run_log::log_dir(&first.run_dir).join("log.0.jsonl");
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
