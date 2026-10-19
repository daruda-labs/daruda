//! What a settled attempt and an open question leave in the log.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use super::{epoch_millis, millis};
use crate::NodeId;
use crate::record::{AttemptOutcome, AttemptRecord, Invalidation};
use crate::request::CostLimit;
use crate::run_log::reader::Spent;
use crate::runner::{AskAnswer, AskRequest, Waiting};

/// One settled attempt, for `run.md`. `attempt` and `evidence` repeat the
/// started attempt's numbers so the line reads on its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptLine {
    pub attempt: u32,
    pub evidence: u32,
    /// Epoch milliseconds; display only.
    #[serde(default)]
    pub at_ms: u64,
    #[serde(default)]
    pub took_ms: u64,
    #[serde(default)]
    pub waited_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<AnswerLine>,
    pub turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invalidated: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived: Vec<PathBuf>,
    /// What the run had spent once this attempt settled: a snapshot, so the
    /// last line is the total.
    pub spent: SpentLine,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpentLine {
    pub node_runs: u32,
    #[serde(default)]
    pub parked_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostLine>,
    #[serde(default)]
    pub cost_mixed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostLine {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerLine {
    Allowed,
    Refused,
    Unanswered,
}

/// A question as the log keeps it. The choices stay with the live
/// `PendingAsk`: a reply channel cannot outlive the process anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskLine {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AttemptLine {
    pub(crate) fn new(record: &AttemptRecord, spent: &Spent) -> Self {
        Self {
            attempt: record.attempt,
            evidence: record.evidence_seq,
            at_ms: epoch_millis(record.at),
            took_ms: millis(record.took),
            waited_ms: millis(record.waited.total),
            answers: record.waited.answers.iter().map(|a| (*a).into()).collect(),
            turns: record.turns,
            git_status: record.git_status.clone(),
            invalidated: record.invalidated.nodes.clone(),
            archived: record.invalidated.archived.clone(),
            spent: SpentLine::from(spent),
        }
    }

    /// The attempt as a resumed run reports it: a failure comes back as the
    /// text it rendered, since a settled attempt's policy is never re-decided.
    pub(crate) fn to_record(&self, outcome: AttemptOutcome) -> AttemptRecord {
        AttemptRecord {
            attempt: self.attempt,
            evidence_seq: self.evidence,
            outcome,
            invalidated: Invalidation {
                nodes: self.invalidated.clone(),
                archived: self.archived.clone(),
            },
            git_status: self.git_status.clone(),
            at: std::time::UNIX_EPOCH + Duration::from_millis(self.at_ms),
            took: Duration::from_millis(self.took_ms),
            waited: Waiting {
                total: Duration::from_millis(self.waited_ms),
                answers: self.answers.iter().map(|a| (*a).into()).collect(),
            },
            // Not logged: high-frequency detail stays in the transcript.
            tools: Vec::new(),
            usage: None,
            turns: self.turns,
        }
    }
}

impl From<&Spent> for SpentLine {
    fn from(spent: &Spent) -> Self {
        Self {
            node_runs: spent.node_runs,
            parked_ms: millis(spent.parked),
            cost: spent.cost.as_ref().map(|c| CostLine {
                amount: c.amount,
                currency: c.currency.clone(),
            }),
            cost_mixed: spent.cost_mixed,
            warnings: spent.warnings.clone(),
        }
    }
}

impl From<&SpentLine> for Spent {
    fn from(line: &SpentLine) -> Self {
        Self {
            node_runs: line.node_runs,
            parked: Duration::from_millis(line.parked_ms),
            cost: line.cost.as_ref().map(|c| CostLimit {
                amount: c.amount,
                currency: c.currency.clone(),
            }),
            cost_mixed: line.cost_mixed,
            warnings: line.warnings.clone(),
        }
    }
}

impl From<AskAnswer> for AnswerLine {
    fn from(answer: AskAnswer) -> Self {
        match answer {
            AskAnswer::Allowed => Self::Allowed,
            AskAnswer::Refused => Self::Refused,
            AskAnswer::Unanswered => Self::Unanswered,
        }
    }
}

impl From<AnswerLine> for AskAnswer {
    fn from(answer: AnswerLine) -> Self {
        match answer {
            AnswerLine::Allowed => Self::Allowed,
            AnswerLine::Refused => Self::Refused,
            AnswerLine::Unanswered => Self::Unanswered,
        }
    }
}

impl From<&daruda_acp::PermissionDecision> for AnswerLine {
    fn from(decision: &daruda_acp::PermissionDecision) -> Self {
        match decision {
            daruda_acp::PermissionDecision::Allow { .. } => Self::Allowed,
            daruda_acp::PermissionDecision::Reject { .. } => Self::Refused,
            daruda_acp::PermissionDecision::Cancelled => Self::Unanswered,
        }
    }
}

impl From<&AskRequest> for AskLine {
    fn from(request: &AskRequest) -> Self {
        Self {
            tool: request.tool.clone(),
            detail: request.detail.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a run had spent reads back as itself, so a resume continues the
    /// budget where the crash left it.
    #[test]
    fn spent_reads_back_as_itself() {
        let spent = Spent {
            node_runs: 3,
            parked: Duration::from_millis(1500),
            cost: Some(CostLimit {
                amount: 1.25,
                currency: "USD".to_string(),
            }),
            cost_mixed: true,
            warnings: vec!["w".to_string()],
        };
        let line = SpentLine::from(&spent);
        let text = serde_json::to_string(&line).expect("serialize");
        let back: SpentLine = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(Spent::from(&back), spent);
    }
}
