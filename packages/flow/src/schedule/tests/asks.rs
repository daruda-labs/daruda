//! Questions put to a person while the rest of the run goes on.

use super::*;
use crate::runner::{AskRequest, Permission};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One node asks a person; its sibling in the same wave fails and retries.
const ASK_BESIDE_A_RETRY: &str = "\
version: 1
defaults: { agent: { id: claude, mode: default }, parallel: 2 }
nodes:
  - id: asker
    kind: agent
    cwd: a
    output: asker.md
    prompt: write
    agent: { id: claude, mode: default, permission: ask }
  - id: retrier
    kind: agent
    cwd: b
    output: retrier.md
    prompt: write
    on_fail:
      retry:
        hint: again
        max_attempts: 2
        wait: 0s
";

/// Long enough for setup under a loaded test run, and well short of when the
/// sibling fails, which is the moment the deadline is asked about.
const DEADLINE: Duration = Duration::from_millis(1000);
const SIBLING_FAILS_AFTER: Duration = Duration::from_millis(1600);

/// `asker` waits on its question; `retrier` fails slowly once, then passes
/// and says so, which is when the host answers.
struct AskBesideRetry {
    inner: FakeRunner,
    retried: Arc<AtomicBool>,
}

impl NodeRunner for AskBesideRetry {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            if ctx.node_id == "asker" {
                if let Permission::Ask(port) = &ctx.permission {
                    let request = AskRequest {
                        tool: "write".to_string(),
                        detail: None,
                        options: Vec::new(),
                    };
                    port.ask(request).await;
                }
            } else if ctx.attempt == 1 {
                crate::runner::sleep(SIBLING_FAILS_AFTER).await;
            } else {
                self.retried.store(true, Ordering::SeqCst);
            }
            self.inner.run_agent(ctx, agent, prompt).await
        })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.inner.run_command(ctx, run)
    }
}

/// Answers the first question once `retried` is set, or after a bound so a
/// run that never retries still ends.
fn answer_after_retry(
    asks: smol::channel::Receiver<crate::runner::PendingAsk>,
    retried: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Ok(pending) = asks.recv_blocking() else {
            return;
        };
        let start = std::time::Instant::now();
        while !retried.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = pending.answer(daruda_acp::PermissionDecision::Allow {
            option_id: "allow".to_string(),
        });
    })
}

/// The run's deadline stops while a question is open, not only once
/// the asking call returns: otherwise the sibling's retry is refused as over
/// time while the run is in fact parked on a person.
#[test]
fn an_open_question_stops_the_run_clock_for_its_siblings_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let retried = Arc::new(AtomicBool::new(false));
    let runner = AskBesideRetry {
        inner: FakeRunner::new().script(
            "retrier",
            vec![
                Step::fail(NodeFailure::TurnFailed("first".to_string())),
                Step::Ok {
                    writes: Some("ok\n".to_string()),
                },
            ],
        ),
        retried: retried.clone(),
    };
    let (ask_tx, ask_rx) = smol::channel::unbounded();
    let host = answer_after_retry(ask_rx, retried.clone());
    let mut request = request_for(ASK_BESIDE_A_RETRY, dir.path());
    request.ask = Some(ask_tx);
    request.budget = Budget {
        deadline: Some(std::time::Instant::now() + DEADLINE),
        ..Budget::unlimited()
    };

    let report = execute(&request, &runner, &CancelToken::default());
    host.join().expect("the host thread");

    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "the sibling's retry was refused while the run was parked: {:?}",
        report.outcome
    );
    assert!(retried.load(Ordering::SeqCst), "the retry never ran");
}

/// Every event the run in `run_dir` committed, by kind and node.
fn log_lines(run_dir: &Path) -> Vec<String> {
    std::fs::read_to_string(crate::run_log::log_file(run_dir))
        .expect("the run's log")
        .lines()
        .map(str::to_string)
        .collect()
}

fn position(lines: &[String], needles: &[&str]) -> usize {
    lines
        .iter()
        .position(|line| needles.iter().all(|needle| line.contains(needle)))
        .unwrap_or_else(|| panic!("no record with {needles:?}: {lines:#?}"))
}

/// A sibling's retry does not close a question: it stays open
/// through the sibling's failure and next start, and once answered the node
/// that asked goes on.
#[test]
fn a_question_stays_open_through_a_sibling_s_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let retried = Arc::new(AtomicBool::new(false));
    let runner = AskBesideRetry {
        inner: FakeRunner::new().script(
            "retrier",
            vec![
                Step::fail(NodeFailure::TurnFailed("first".to_string())),
                Step::Ok {
                    writes: Some("ok\n".to_string()),
                },
            ],
        ),
        retried: retried.clone(),
    };
    let (ask_tx, ask_rx) = smol::channel::unbounded();
    let host = answer_after_retry(ask_rx, retried);
    let mut request = request_for(ASK_BESIDE_A_RETRY, dir.path());
    request.ask = Some(ask_tx);

    let report = execute(&request, &runner, &CancelToken::default());
    host.join().expect("the host thread");
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );

    let lines = log_lines(&report.run_dir);
    let raised = position(&lines, &["\"ask_raised\"", "\"asker\""]);
    let sibling_failed = position(&lines, &["\"attempt_failed\"", "\"retrier\""]);
    let sibling_again = position(
        &lines,
        &["\"attempt_started\"", "\"retrier\"", "\"attempt\":2"],
    );
    let answered = position(&lines, &["\"ask_answered\""]);
    let asker_passed = position(&lines, &["\"attempt_passed\"", "\"asker\""]);
    assert!(raised < sibling_failed && sibling_failed < sibling_again);
    assert!(sibling_again < answered && answered < asker_passed);
    assert!(
        !lines.iter().any(|line| line.contains("\"ask_abandoned\"")),
        "the question was dropped: {lines:#?}"
    );
}

/// Asks, and gives up waiting when the run is stopped — the way the ACP
/// runner's turn is cut short.
struct AsksUntilStopped(FakeRunner);

impl NodeRunner for AsksUntilStopped {
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
                smol::future::or(port.ask(request), async {
                    crate::runner::canceled(ctx.cancel).await;
                    daruda_acp::PermissionDecision::Cancelled
                })
                .await;
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

/// An answer to a question the run has abandoned is reported as
/// failed, and the log says the question was abandoned.
#[test]
fn answering_an_abandoned_question_fails() {
    let flow = "\
version: 1
defaults: { agent: { id: claude, mode: default, permission: ask } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write
";
    let dir = tempfile::tempdir().expect("tempdir");
    let cancel = CancelToken::default();
    let (ask_tx, ask_rx) = smol::channel::unbounded::<crate::runner::PendingAsk>();
    let host = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            let pending = ask_rx.recv_blocking().expect("a question");
            cancel.cancel();
            let start = std::time::Instant::now();
            while !pending.reply.is_closed() && start.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(5));
            }
            pending.answer(daruda_acp::PermissionDecision::Allow {
                option_id: "allow".to_string(),
            })
        })
    };
    let mut request = request_for(flow, dir.path());
    request.ask = Some(ask_tx);

    let report = execute(&request, &AsksUntilStopped(FakeRunner::new()), &cancel);
    let answered = host.join().expect("the host thread");

    assert!(
        matches!(report.outcome, RunOutcome::Canceled { .. }),
        "{:?}",
        report.outcome
    );
    assert_eq!(answered, Err(crate::runner::AskClosed));
    let lines = log_lines(&report.run_dir);
    position(&lines, &["\"ask_abandoned\"", "\"interrupted\""]);
}
