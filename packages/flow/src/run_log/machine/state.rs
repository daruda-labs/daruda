//! What a run log replays into: per node, which generation and attempt it is
//! on and how its last attempt ended; per run, the numbers the log owns.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::NodeId;
use crate::run_log::line::{AskLine, FailureLine, SpentLine};
use crate::runner::AskOwner;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlowRunState {
    pub(crate) phase: RunPhase,
    pub(crate) start: StartFacts,
    pub(crate) nodes: BTreeMap<NodeId, NodeState>,
    /// The next evidence number; never reused, across resumes too (F2).
    pub(crate) next_evidence: u32,
    pub(crate) next_ask: u64,
    pub(crate) spent: SpentLine,
    /// Every question still waiting, keyed by its id: a set, not one slot.
    pub(crate) open_asks: BTreeMap<u64, OpenAsk>,
    /// Gates whose fix session is running, with that session's evidence.
    pub(crate) fixing: BTreeMap<NodeId, u32>,
    pub(crate) resumes: u32,
    /// How many passes and pins the run has recorded; each node keeps the
    /// ordinal of its latest, so the order nodes passed in is replayed too.
    #[serde(default)]
    pub(crate) passes: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    /// Nothing committed yet.
    #[default]
    Fresh,
    Running,
    Ended(EndKind),
}

/// How an ended run classifies, the axis the markers write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndKind {
    Done,
    Failed,
    Canceled,
    Stalled,
}

/// What a run was asked to be, fixed when it started.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StartFacts {
    /// Topological order, the order a host draws.
    pub(crate) order: Vec<NodeId>,
    pub(crate) until: Option<NodeId>,
    pub(crate) pinned: Vec<NodeId>,
    pub(crate) profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeState {
    pub(crate) produces: Produces,
    /// Raised each time the node's pass is taken away (a repair invalidates
    /// it, or its output is lost); attempts count from 1 within one.
    pub(crate) generation: u32,
    /// Attempts started in this generation, interrupted ones included: the
    /// next attempt's number is one past this.
    pub(crate) attempts: u32,
    /// Of those, the ones a crash interrupted. They do not count against
    /// `max_attempts` (D4-2); a stop or a failure does.
    #[serde(default)]
    pub(crate) crashed: u32,
    pub(crate) last: AttemptState,
    /// A repair took this node's output away; it is no longer current.
    pub(crate) invalidated: bool,
    /// The run's pass ordinal when this node last passed or was pinned.
    #[serde(default)]
    pub(crate) passed_as: u32,
}

/// Whether a passed node leaves a file the run can check is still there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Produces {
    File,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AttemptState {
    NotRun,
    /// Reused from an earlier run; counts as passed and has no attempt.
    Pinned,
    Started {
        attempt: u32,
        evidence: u32,
        /// The process group a command attempt started, once recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        process: Option<SpawnedProcess>,
    },
    Passed {
        /// `None` for a pass an older build's journal recorded, which has no
        /// attempt in this log.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attempt: Option<u32>,
    },
    Failed {
        attempt: u32,
        failure: FailureLine,
    },
    Canceled {
        attempt: u32,
    },
    /// Started by a process that died before it settled. A user's or an
    /// overage stop settles as `Canceled` instead.
    Interrupted {
        attempt: u32,
    },
}

/// A command attempt's process group as the log keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnedProcess {
    pub(crate) leader: u32,
    pub(crate) identity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAsk {
    pub(crate) owner: AskOwner,
    pub(crate) ask: AskLine,
}

impl FlowRunState {
    pub fn phase(&self) -> RunPhase {
        self.phase
    }

    pub(crate) fn node(&self, id: &NodeId) -> Option<&NodeState> {
        self.nodes.get(id)
    }

    /// Nodes whose output stands: passed or pinned, and not invalidated.
    pub(crate) fn finished(&self) -> BTreeSet<NodeId> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.counts_as_passed())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The same nodes in the order they passed: what a resumed run starts
    /// from, and the one fold of which passes stand.
    pub(crate) fn passed_in_order(&self) -> Vec<NodeId> {
        let mut passed: Vec<(&NodeId, &NodeState)> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.counts_as_passed())
            .collect();
        passed.sort_by_key(|(_, node)| node.passed_as);
        passed.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// Every node the run started with, in the order a host draws them,
    /// with what the log says of it.
    pub fn node_states(&self) -> impl Iterator<Item = (&NodeId, &NodeState)> {
        self.start
            .order
            .iter()
            .filter_map(|id| self.nodes.get_key_value(id))
    }

    /// Whether `gate`'s repair is running its fix session.
    pub fn is_fixing(&self, gate: &NodeId) -> bool {
        self.fixing.contains_key(gate)
    }

    pub fn open_ask_ids(&self) -> Vec<u64> {
        self.open_asks.keys().copied().collect()
    }

    pub(crate) fn has_open_asks(&self) -> bool {
        !self.open_asks.is_empty()
    }

    /// Process groups of attempts that started and never settled.
    pub(crate) fn unsettled_processes(&self) -> Vec<(NodeId, SpawnedProcess)> {
        self.nodes
            .iter()
            .filter_map(|(id, node)| match node.last {
                AttemptState::Started {
                    process: Some(process),
                    ..
                } => Some((id.clone(), process)),
                _ => None,
            })
            .collect()
    }

    /// Whether `resume` may pick this run up: it stopped without saying it
    /// ended, or it ended stalled.
    pub(crate) fn is_resumable(&self) -> bool {
        matches!(
            self.phase,
            RunPhase::Running | RunPhase::Ended(EndKind::Stalled)
        )
    }
}

impl NodeState {
    pub(crate) fn new(produces: Produces) -> Self {
        Self {
            produces,
            generation: 0,
            attempts: 0,
            crashed: 0,
            last: AttemptState::NotRun,
            invalidated: false,
            passed_as: 0,
        }
    }

    /// How this node's latest attempt in its generation ended, if it did.
    pub fn last(&self) -> &AttemptState {
        &self.last
    }

    /// Attempts in this generation that count against `max_attempts`.
    pub(crate) fn spent(&self) -> u32 {
        self.attempts - self.crashed
    }

    /// A repair took this node's output away and it has not passed since.
    pub fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    pub(crate) fn counts_as_passed(&self) -> bool {
        !self.invalidated
            && matches!(
                self.last,
                AttemptState::Passed { .. } | AttemptState::Pinned
            )
    }
}
