//! What a run produced, as the host reads it. Separate from the drive
//! loop because these are the crate's public surface — `run.md`, the event
//! stream and the marker are all written from them — while the loop next
//! door is machinery none of them needs to see.

use crate::NodeId;
use crate::error::FlowIoError;
use crate::lock::LockHolder;
use crate::record::NodeRecord;
use crate::request::CostLimit;
use crate::runner::NodeFailure;
use std::path::PathBuf;

/// How a run ended.
#[derive(Debug)]
pub enum RunOutcome {
    Done,
    Failed {
        node: NodeId,
        failure: NodeFailure,
    },
    /// The run was stopped. `node` names what was in flight, or `None` if
    /// the cancel landed between nodes.
    Canceled {
        node: Option<NodeId>,
    },
    /// A runaway defence tripped. Which one it was is the only thing that
    /// tells a raised limit from a hung flow.
    BudgetExhausted {
        limit: BudgetLimit,
    },
    /// Reading a flow-local prompt, or reading/writing under the run
    /// directory, failed.
    Io(FlowIoError),
    /// The request itself did not hold up. Nothing ran, and nothing was
    /// written — the paths a run would write to are among what failed.
    Invalid {
        issues: Vec<crate::error::ValidationIssue>,
    },
    /// Another run holds this working directory. Nothing was started, so
    /// nothing was recorded.
    LockHeld {
        holder: LockHolder,
    },
    /// A runtime the run needs could not be prepared. No node ran, so this
    /// names the agent rather than a node.
    Unprovisioned {
        agent: String,
        message: String,
    },
}

/// Which of the three defences stopped the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLimit {
    WallClock,
    NodeRuns,
    Cost,
}

/// What the run did, beyond how it ended. `run.md` (P2b-3) is written from
/// exactly this, so the accounting lives here rather than in the writer.
#[derive(Debug)]
pub struct RunReport {
    pub outcome: RunOutcome,
    /// Where the run's artifacts and its completion marker are, so the
    /// recording code (P2b-3) needs only the report and not the request.
    pub run_dir: PathBuf,
    /// Runner calls made, including reruns and fix sessions.
    pub node_runs: u32,
    /// Accumulated only while every reported cost shares one currency.
    pub cost: Option<CostLimit>,
    /// Things the user has to know that are not failures — chiefly that a
    /// cost limit never applied because nothing reported a cost.
    ///
    /// Private because order carries meaning: setting the run up happens
    /// before anything the run itself has to say, and callers were
    /// enforcing that with a two-line prepend shuffle at one of five sites
    /// that wrote this field directly. [`RunReport::warn`] and
    /// [`RunReport::warn_from_setup`] are the two orders there are.
    warnings: Vec<String>,
    /// Every attempt the run made, in the order the nodes first ran. The
    /// attempt counts here sum to `node_runs`.
    pub nodes: Vec<NodeRecord>,
}

impl RunReport {
    /// The report for a run that never reached its first node — refused
    /// before the lock, or refused by the lock. Nothing ran, so there is
    /// nothing to account for, and **nothing was written**: no run
    /// directory, no `run.md`, no marker.
    ///
    /// Named rather than assembled at the call site because that
    /// distinction is load-bearing for a host — it is the difference
    /// between a run that has a story to show and one that does not — and
    /// because two hand-built `RunReport`s are two places to keep in step.
    pub fn refused(run_dir: PathBuf, outcome: RunOutcome) -> Self {
        Self {
            outcome,
            run_dir,
            node_runs: 0,
            cost: None,
            warnings: Vec::new(),
            nodes: Vec::new(),
        }
    }

    /// The report of a run that actually ran. `pub(in crate::schedule)`
    /// because assembling one is the accounting's job and nobody else's —
    /// that is what keeps `warnings` sealed from the setup code, which had
    /// been rewriting the whole vector to get the order it wanted.
    pub(in crate::schedule) fn completed(
        outcome: RunOutcome,
        run_dir: PathBuf,
        node_runs: u32,
        cost: Option<CostLimit>,
        warnings: Vec<String>,
        nodes: Vec<NodeRecord>,
    ) -> Self {
        Self {
            outcome,
            run_dir,
            node_runs,
            cost,
            warnings,
            nodes,
        }
    }

    /// A warning from the run, after everything already recorded.
    pub fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    /// Warnings from preparing the run — they happened before anything the
    /// run itself said, so they read first.
    pub fn warn_from_setup(&mut self, mut earlier: Vec<String>) {
        if earlier.is_empty() {
            return;
        }
        earlier.append(&mut self.warnings);
        self.warnings = earlier;
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// One node's history, or `None` if it never ran. `FIX_SESSION_ID` names
    /// the repair sessions, which are recorded like any other attempt.
    pub fn node(&self, id: &str) -> Option<&NodeRecord> {
        self.nodes.iter().find(|record| record.id == id)
    }
}
