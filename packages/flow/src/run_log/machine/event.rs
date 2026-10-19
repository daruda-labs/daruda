//! Everything that can happen to a flow run. Fields the log numbers
//! (attempt, evidence, ask id) are proposed as 0 and filled in by `assign`.

use serde::{Deserialize, Serialize};

use super::state::Produces;
use crate::NodeId;
use crate::run_log::line::{AnswerLine, AskLine, AttemptLine, EndLine, FailureLine, SpentLine};
use crate::runner::AskOwner;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowRunEvent {
    RunStarted {
        nodes: Vec<NodeStart>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<NodeId>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pinned: Vec<NodeId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
    },
    /// A run an older build recorded in `progress.jsonl`, taken over whole
    /// so it can be resumed. Only into an empty log.
    Imported(Box<Imported>),
    /// A later process picked the run up.
    Resumed {
        carried: usize,
    },
    AttemptStarted {
        node: NodeId,
        generation: u32,
        attempt: u32,
        evidence: u32,
    },
    /// Refused before the runner was called; no attempt was made.
    NodeRefused {
        node: NodeId,
        attempt: u32,
        failure: FailureLine,
    },
    /// A command attempt's process group, so a resume after a crash can
    /// end it. `identity` is `daruda_core::process::ProcessIdentity::to_raw`.
    ProcessSpawned {
        node: NodeId,
        evidence: u32,
        leader: u32,
        identity: u64,
    },
    AttemptPassed {
        node: NodeId,
        record: Box<AttemptLine>,
    },
    AttemptFailed {
        node: NodeId,
        failure: FailureLine,
        record: Box<AttemptLine>,
    },
    AttemptCanceled {
        node: NodeId,
        record: Box<AttemptLine>,
    },
    /// A passed agent node whose output was gone when the run resumed. It
    /// starts a new generation.
    OutputLost {
        node: NodeId,
    },
    /// Committed before a repair moves the members' outputs aside. Each
    /// member starts a new generation.
    Invalidated {
        gate: NodeId,
        members: Vec<NodeId>,
    },
    /// A repair re-derives these, in the generations `Invalidated` began.
    RerunScheduled {
        gate: NodeId,
        members: Vec<NodeId>,
    },
    FixStarted {
        gate: NodeId,
        evidence: u32,
    },
    FixEnded {
        gate: NodeId,
        outcome: FixOutcome,
        record: Box<AttemptLine>,
    },
    AskRaised {
        owner: AskOwner,
        ask: AskLine,
        id: u64,
    },
    AskAnswered {
        id: u64,
        answer: AnswerLine,
    },
    AskAbandoned {
        id: u64,
        reason: AbandonReason,
    },
    RunEnded {
        end: EndLine,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeStart {
    pub id: NodeId,
    pub produces: Produces,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub nodes: Vec<NodeStart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<NodeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Passed nodes, pins excluded, in the order they passed.
    pub passed: Vec<NodeId>,
    pub next_evidence: u32,
    pub spent: SpentLine,
    /// The old journal's attempts, for `run.md`; each with its outcome.
    pub records: Vec<ImportedAttempt>,
    /// For each node that has not passed, what its current generation
    /// spent, so a resume keeps counting from there. Absent from
    /// imports written before it was carried.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<ImportedNode>,
}

/// A node's settled attempts in its current generation, as the old journal
/// knew them. Only settled ones: the journal never saw an attempt a crash
/// cut short, and such an attempt spends nothing anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedNode {
    pub node: NodeId,
    pub attempts: u32,
    pub last: ImportedLast,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ImportedLast {
    Failed { reason: String },
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedAttempt {
    pub node: NodeId,
    pub outcome: ImportedOutcome,
    pub record: AttemptLine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ImportedOutcome {
    Passed,
    Canceled,
    Reported { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum FixOutcome {
    Passed,
    Failed { failure: FailureLine },
    Canceled,
}

/// Why a question closed without an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbandonReason {
    /// The call that asked ended first: a stop or a timeout.
    Interrupted,
    /// Nobody was there to take the question or send the answer.
    HostGone,
    /// The process that asked died; its reply channel died with it.
    AppRestart,
    /// Closed so the run could record its end.
    RunEnded,
}

impl FlowRunEvent {
    pub(crate) fn attempt_started(node: NodeId, generation: u32) -> Self {
        Self::AttemptStarted {
            node,
            generation,
            attempt: 0,
            evidence: 0,
        }
    }

    pub(crate) fn fix_started(gate: NodeId) -> Self {
        Self::FixStarted { gate, evidence: 0 }
    }

    pub(crate) fn ask_raised(owner: AskOwner, ask: AskLine) -> Self {
        Self::AskRaised { owner, ask, id: 0 }
    }

    pub(crate) fn node_refused(node: NodeId, failure: FailureLine) -> Self {
        Self::NodeRefused {
            node,
            attempt: 0,
            failure,
        }
    }
}
