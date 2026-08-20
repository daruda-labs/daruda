//! A scripted `NodeRunner` for tests. Every scheduler behaviour this crate
//! has — order, judgment, archiving, retry and rendered text — is exercised
//! through it, so nothing in the scheduler needs an agent or a process.

use crate::model::AgentSpec;
use crate::runner::{NodeFailure, NodeRunner, RunContext, RunResult};
use daruda_acp::{CostView, UsageView};
use std::cell::RefCell;
use std::collections::HashMap;

/// What the fake does for one attempt at one node. Every arm writes first
/// and reports second, because an agent that fails mid-turn still leaves
/// whatever it had already written on disk — the case the scheduler's
/// archiving exists to contain.
#[derive(Debug, Clone)]
pub(crate) enum Step {
    /// Succeed, and (for an agent node) write `output` with this text so the
    /// scheduler's "did it write anything" judgment has something to see.
    Ok { writes: Option<String> },
    Fail {
        /// A truncated result left behind by the failing attempt.
        writes: Option<String>,
        failure: NodeFailure,
    },
}

impl Step {
    /// A failure that produced nothing — the common case.
    pub(crate) fn fail(failure: NodeFailure) -> Self {
        Self::Fail {
            writes: None,
            failure,
        }
    }
}

/// One call the scheduler made, with the text it handed over. Recording
/// the text is what makes the scheduler's own choices observable: whether
/// it rendered at all, which surface it quoted for, and whether a repair
/// saw the archived evidence. Without it a test can only see that *a* call
/// happened, which every wrong ordering also produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) node: crate::NodeId,
    pub(crate) attempt: u32,
    /// The agent prompt, or the command line — already rendered.
    pub(crate) text: String,
}

/// Scripted per node id: the nth call to a node consumes the nth step. A
/// node whose script runs out keeps repeating its last step, so a test only
/// has to spell out the attempts it cares about.
pub(crate) struct FakeRunner {
    script: HashMap<String, Vec<Step>>,
    /// Reported by every call that has no entry in `cost`. `None` is an
    /// agent that reports nothing, which is the default because most
    /// adapters do not report a cost at all.
    default_cost: Option<CostView>,
    cost: HashMap<String, CostView>,
    /// Node id to the attempt during which this runner cancels the run. A
    /// separate axis from `Step`, because when a stop lands is independent
    /// of what the interrupted attempt was going to do.
    cancel_at: HashMap<String, u32>,
    /// Reported by every call as time spent waiting for a person. Scripted
    /// rather than really waited — what this exercises is the run's ceiling
    /// arithmetic, and a test that actually slept could not run.
    parked: std::time::Duration,
    calls: RefCell<Vec<Call>>,
}

impl FakeRunner {
    pub(crate) fn new() -> Self {
        Self {
            script: HashMap::new(),
            default_cost: None,
            cancel_at: HashMap::new(),
            cost: HashMap::new(),
            parked: std::time::Duration::ZERO,
            calls: RefCell::new(Vec::new()),
        }
    }

    /// Every call reports this much time waiting for a person, which the
    /// run's deadline has to be pushed out by.
    pub(crate) fn parked_per_call(mut self, parked: std::time::Duration) -> Self {
        self.parked = parked;
        self
    }

    /// Every unscripted node succeeds and writes a one-line output.
    pub(crate) fn script(mut self, node: &str, steps: Vec<Step>) -> Self {
        self.script.insert(node.to_string(), steps);
        self
    }

    /// Cancel the run from inside this node's nth attempt, before it does
    /// anything else. The scheduler is serial, so a stop can only originate
    /// from within a call — this is the only way to reach the mid-node path.
    pub(crate) fn cancel_at(mut self, node: &str, attempt: u32) -> Self {
        self.cancel_at.insert(node.to_string(), attempt);
        self
    }

    /// Report this cost from every call. Each call is its own session, so
    /// the run's total is the sum over calls, not this amount.
    pub(crate) fn cost_per_call(mut self, amount: f64, currency: &str) -> Self {
        self.default_cost = Some(CostView {
            amount,
            currency: currency.to_string(),
        });
        self
    }

    /// Report this cost from one node only — the rest report none, which is
    /// how a mixed-currency run is written without every node paying.
    pub(crate) fn cost_for(mut self, node: &str, amount: f64, currency: &str) -> Self {
        self.cost.insert(
            node.to_string(),
            CostView {
                amount,
                currency: currency.to_string(),
            },
        );
        self
    }

    /// Every call, in the order the scheduler made them.
    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    /// Just the node ids — for assertions about order that do not care
    /// about attempt numbers or text.
    pub(crate) fn ids(&self) -> Vec<crate::NodeId> {
        self.calls.borrow().iter().map(|c| c.node.clone()).collect()
    }

    fn step_for(&self, node: &str, attempt: u32) -> Step {
        match self.script.get(node) {
            Some(steps) if !steps.is_empty() => {
                let index = ((attempt as usize).saturating_sub(1)).min(steps.len() - 1);
                steps[index].clone()
            }
            _ => Step::Ok {
                writes: Some(format!("{node} ran\n")),
            },
        }
    }

    fn perform(
        &self,
        ctx: &RunContext<'_>,
        text: &str,
        output: Option<&std::path::Path>,
    ) -> RunResult {
        self.calls.borrow_mut().push(Call {
            node: ctx.node_id.clone(),
            attempt: ctx.attempt,
            text: text.to_string(),
        });
        let step = self.step_for(ctx.node_id.as_str(), ctx.attempt);
        let log = ctx.log_dir.join(format!(
            "{}.attempt-{}.evidence-{}.log",
            ctx.node_id, ctx.attempt, ctx.evidence_seq
        ));
        let _ = std::fs::create_dir_all(ctx.log_dir);
        let _ = std::fs::write(&log, format!("{} attempt {}\n", ctx.node_id, ctx.attempt));
        if self.cancel_at.get(ctx.node_id.as_str()) == Some(&ctx.attempt) {
            ctx.cancel.cancel();
        }
        let (writes, outcome) = match step {
            Step::Ok { writes } => (writes, Ok(())),
            Step::Fail { writes, failure } => (writes, Err(failure)),
        };
        if let (Some(text), Some(path)) = (writes, output) {
            let _ = std::fs::create_dir_all(ctx.run_dir);
            let _ = std::fs::write(path, text);
        }
        RunResult {
            outcome,
            artifacts: vec![log],
            usage: self.usage_for(ctx.node_id.as_str()),
            waiting: crate::runner::Waiting {
                total: self.parked,
                answers: Vec::new(),
            },
        }
    }

    /// `used`/`size` stay zero: they are context-window occupancy, and the
    /// only cumulative figure the scheduler can account for is the cost.
    fn usage_for(&self, node: &str) -> Option<UsageView> {
        let cost = self.cost.get(node).or(self.default_cost.as_ref())?;
        Some(UsageView {
            used: 0,
            size: 0,
            cost: Some(cost.clone()),
        })
    }
}

impl NodeRunner for FakeRunner {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        _agent: &'a AgentSpec,
        prompt: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        // The scheduler passes the resolved output path. The fake writes
        // exactly there so the file-contract judgment is exercisable.
        let result = self.perform(ctx, prompt, ctx.output);
        Box::pin(async move { result })
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = RunResult> + 'a>> {
        let result = self.perform(ctx, run, None);
        Box::pin(async move { result })
    }
}
