//! Two nodes at once.
//!
//! Every test here turns on what the fake runner can observe about
//! *overlap*: a node that records "I started" and then yields, so a second
//! node can only appear inside the first one's window if the two really are
//! in flight together.

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Two independent nodes and one that waits for both.
const FORK_AND_JOIN: &str = "\
version: 1
defaults:
  parallel: 2
nodes:
  - id: left
    kind: command
    cwd: a
    run: \"true\"
  - id: right
    kind: command
    cwd: b
    run: \"true\"
  - id: join
    kind: command
    deps: [left, right]
    run: \"true\"
";

/// The same flow with nowhere to put the two branches: both work in the
/// run's own directory.
const FORK_SHARING_ONE_TREE: &str = "\
version: 1
defaults:
  parallel: 2
nodes:
  - id: left
    kind: command
    run: \"true\"
  - id: right
    kind: command
    run: \"true\"
";

/// Records the order of starts and finishes, and yields once inside every
/// call so a concurrent sibling has somewhere to interleave.
struct Interleaving {
    inner: FakeRunner,
    log: Rc<RefCell<Vec<String>>>,
}

impl Interleaving {
    fn new() -> Self {
        Self {
            inner: FakeRunner::new(),
            log: Rc::new(RefCell::new(Vec::new())),
        }
    }

    async fn trace(&self, ctx: &RunContext<'_>) {
        self.log.borrow_mut().push(format!("start {}", ctx.node_id));
        // One yield is all an overlap needs: if the scheduler is running
        // these one at a time, the next line still follows immediately.
        smol::future::yield_now().await;
        self.log.borrow_mut().push(format!("end {}", ctx.node_id));
    }
}

impl NodeRunner for Interleaving {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            self.trace(ctx).await;
            self.inner.run_agent(ctx, agent, prompt).await
        })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            self.trace(ctx).await;
            self.inner.run_command(ctx, run).await
        })
    }
}

fn trace_of(flow: &str, dir: &std::path::Path) -> Vec<String> {
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.join(sub)).expect("mkdir");
    }
    let runner = Interleaving::new();
    let log = runner.log.clone();
    execute(&request_for(flow, dir), &runner, &CancelToken::default());
    // Cloned out: the log is borrowed by the runner, which outlives it.
    log.borrow().clone()
}

/// Two nodes that depend on nothing and work in different directories are
/// in flight at the same time — the second starts before the first ends.
#[test]
fn two_independent_nodes_overlap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = trace_of(FORK_AND_JOIN, dir.path());
    assert_eq!(
        trace,
        vec![
            "start left",
            "start right",
            "end left",
            "end right",
            "start join",
            "end join",
        ],
        "{trace:?}"
    );
}

/// And the node that waits for them still does. Overlap is for nodes with
/// nothing between them; a dependency is exactly something between them.
#[test]
fn a_node_that_waits_still_waits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = trace_of(FORK_AND_JOIN, dir.path());
    let started_join = trace.iter().position(|e| e == "start join").expect("join");
    for finished in ["end left", "end right"] {
        let at = trace.iter().position(|e| e == finished).expect(finished);
        assert!(
            at < started_join,
            "join started before {finished}: {trace:?}"
        );
    }
}

/// **The safety rule.** Two nodes working in the same directory are run one
/// at a time however high `parallel` is — two agents editing one tree at
/// once corrupt each other, and nothing inside a node can prevent that.
#[test]
fn nodes_sharing_a_working_directory_never_overlap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let trace = trace_of(FORK_SHARING_ONE_TREE, dir.path());
    assert_eq!(
        trace,
        vec!["start left", "end left", "start right", "end right"],
        "two nodes shared a working directory and ran together: {trace:?}"
    );
}

/// A flow that says nothing runs one at a time. The feature costs the
/// flows that do not ask for it nothing.
#[test]
fn a_flow_that_does_not_ask_for_it_stays_serial() {
    let dir = tempfile::tempdir().expect("tempdir");
    let serial = FORK_AND_JOIN.replace("  parallel: 2\n", "");
    let trace = trace_of(&serial, dir.path());
    assert_eq!(
        trace,
        vec![
            "start left",
            "end left",
            "start right",
            "end right",
            "start join",
            "end join",
        ],
        "{trace:?}"
    );
}

/// When the run stops, nothing is still running. A wave is awaited whole,
/// so a failure never leaves a sibling mid-write in a directory the run is
/// about to stop caring about.
#[test]
fn a_failure_does_not_leave_a_sibling_in_flight() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let runner = Interleaving {
        inner: FakeRunner::new().script("left", vec![Step::fail(NodeFailure::Refused)]),
        log: Rc::new(RefCell::new(Vec::new())),
    };
    let log = runner.log.clone();
    let report = execute(
        &request_for(FORK_AND_JOIN, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    let trace = log.borrow().clone();
    assert!(
        trace.contains(&"end right".to_string()),
        "the sibling was abandoned in flight: {trace:?}"
    );
    assert!(
        !trace.iter().any(|e| e == "start join"),
        "the run carried on past a failure: {trace:?}"
    );
    assert!(
        matches!(report.outcome, RunOutcome::Failed { ref node, .. } if node == "left"),
        "{:?}",
        report.outcome
    );
}

/// Three branches with nothing between them. One fails, and the third
/// never starts.
const THREE_INDEPENDENT: &str = "\
version: 1
defaults:
  parallel: 2
nodes:
  - id: left
    kind: command
    cwd: a
    run: \"true\"
  - id: right
    kind: command
    cwd: b
    run: \"true\"
  - id: later
    kind: command
    run: \"true\"
";

/// Halt means stop, not "stop the ones that were waiting on it".
///
/// A node with no relation to the failure would otherwise keep going: it
/// is ready, nothing it depends on failed, and the loop would find it in
/// the next wave. Whether that is desirable is not the question — the flow
/// said `halt`, and a run that goes on spending after a halt is one nobody
/// asked for.
#[test]
fn a_halt_stops_the_branches_that_had_nothing_to_do_with_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    for sub in ["a", "b"] {
        std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
    }
    let runner = FakeRunner::new().script("left", vec![Step::fail(NodeFailure::Refused)]);
    let report = execute(
        &request_for(THREE_INDEPENDENT, dir.path()),
        &runner,
        &CancelToken::default(),
    );

    assert!(
        !runner.ids().contains(&"later".to_string()),
        "an unrelated branch ran after a halt: {:?}",
        runner.ids()
    );
    assert!(
        matches!(report.outcome, RunOutcome::Failed { ref node, .. } if node == "left"),
        "{:?}",
        report.outcome
    );
}
