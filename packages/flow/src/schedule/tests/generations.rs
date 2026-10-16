//! The numbers a run's log hands out: attempts per generation, evidence
//! across a wave, and the questions a repair's fix asks.

use super::*;
use crate::run_log::machine::FlowRunEvent;
use crate::runner::{AskOwner, AskRequest, Permission};

/// Every event the run in `run_dir` committed.
fn logged(run_dir: &Path) -> Vec<FlowRunEvent> {
    let text = std::fs::read_to_string(crate::run_log::log_dir(run_dir).join("log.0.jsonl"))
        .expect("the run's log");
    text.lines()
        .filter_map(|line| serde_json::from_str::<daruda_runlog::Record<FlowRunEvent>>(line).ok())
        .filter_map(|record| match record.body {
            daruda_runlog::Body::Event { event } => Some(event),
            daruda_runlog::Body::Opened { .. } => None,
        })
        .collect()
}

/// A gate allowed three attempts, failing twice: its rerun member is
/// re-derived twice.
const GATED_THREE: &str = "\
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
        fix: fix it, see {{failure}}
        rerun: [review]
        max_attempts: 3
        wait: 0s
";

/// **B1.** Each repair rerun is a generation of its own, and each counts
/// its attempts from 1 — the log accepts all three, as the scheduler has
/// always numbered them.
#[test]
fn each_rerun_is_a_generation_counting_from_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let runner = FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    );
    let report = execute(
        &request_for(GATED_THREE, dir.path()),
        &runner,
        &CancelToken::default(),
    );
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let review: Vec<(u32, u32)> = logged(&report.run_dir)
        .into_iter()
        .filter_map(|event| match event {
            FlowRunEvent::AttemptStarted {
                node,
                generation,
                attempt,
                ..
            } if node == "review" => Some((generation, attempt)),
            _ => None,
        })
        .collect();
    assert_eq!(review, vec![(0, 1), (1, 1), (2, 1)]);
    let gate: Vec<u32> = report
        .node("gate")
        .expect("gate ran")
        .attempts
        .iter()
        .map(|a| a.attempt)
        .collect();
    assert_eq!(gate, vec![1, 2, 3], "the gate's own count runs on");
}

/// **B2.** Two nodes of one wave propose their starts together; the log
/// numbers them apart.
#[test]
fn a_wave_s_starts_get_distinct_evidence() {
    let flow = "\
version: 1
defaults: { agent: { id: claude }, parallel: 2 }
nodes:
  - id: left
    kind: agent
    cwd: a
    output: left.md
    prompt: write
  - id: right
    kind: agent
    cwd: b
    output: right.md
    prompt: write
";
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let report = execute(
        &request_for(flow, dir.path()),
        &FakeRunner::new(),
        &CancelToken::default(),
    );
    assert!(matches!(report.outcome, RunOutcome::Done));

    let mut evidence: Vec<u32> = logged(&report.run_dir)
        .into_iter()
        .filter_map(|event| match event {
            FlowRunEvent::AttemptStarted { evidence, .. } => Some(evidence),
            _ => None,
        })
        .collect();
    evidence.sort_unstable();
    assert_eq!(evidence, vec![1, 2]);
}

/// The fix asks a question; everything else is the fake.
struct FixAsks(FakeRunner);

impl NodeRunner for FixAsks {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            if ctx.node_id == FIX_SESSION_ID
                && let Permission::Ask(port) = &ctx.permission
            {
                let request = AskRequest {
                    tool: "edit".to_string(),
                    detail: None,
                    options: Vec::new(),
                };
                let decision = port.ask(request).await;
                assert!(
                    matches!(decision, daruda_acp::PermissionDecision::Allow { .. }),
                    "{decision:?}"
                );
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

/// **B3.** A repair's fix asks as its gate, the log takes the question, and
/// once it is answered the repair goes on.
#[test]
fn a_fix_session_s_question_is_taken_and_answered() {
    let flow = GATED.replace(
        "defaults: { agent: { id: claude } }",
        "defaults: { agent: { id: claude, mode: default, permission: ask } }",
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let (ask_tx, ask_rx) = smol::channel::unbounded::<crate::runner::PendingAsk>();
    let host = std::thread::spawn(move || {
        let mut asked = Vec::new();
        while let Ok(pending) = ask_rx.recv_blocking() {
            asked.push(pending.owner.clone());
            let _ = pending.answer(daruda_acp::PermissionDecision::Allow {
                option_id: "allow".to_string(),
            });
        }
        asked
    });
    let mut request = request_for(&flow, dir.path());
    request.ask = Some(ask_tx);
    let runner = FixAsks(FakeRunner::new().script(
        "gate",
        vec![
            Step::fail(NodeFailure::Exit { code: Some(1) }),
            Step::Ok { writes: None },
        ],
    ));
    let report = execute(&request, &runner, &CancelToken::default());
    drop(request);
    let asked = host.join().expect("the host");

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        asked,
        vec![AskOwner::Fix {
            gate: "gate".into()
        }]
    );
    let events = logged(&report.run_dir);
    assert!(events.iter().any(|event| matches!(
        event,
        FlowRunEvent::AskRaised { owner: AskOwner::Fix { gate }, .. } if gate == "gate"
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, FlowRunEvent::AskAnswered { .. }))
    );
}
