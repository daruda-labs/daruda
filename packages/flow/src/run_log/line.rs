//! The shapes a run log writes. Their own types rather than `serde` on the
//! scheduler's, so an internal rename is never a format change.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::NodeId;
use crate::error::IoSite;
use crate::event::RunEnd;
use crate::record::{AttemptOutcome, AttemptRecord, Invalidation};
use crate::request::CostLimit;
use crate::runner::{AskAnswer, AskRequest, NodeFailure, Waiting};
use crate::schedule::{BudgetLimit, RunOutcome};

use super::reader::Spent;

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

/// [`NodeFailure`], field for field, so a replayed `NodeFailed` is the one
/// the run reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureLine {
    Timeout {
        elapsed_ms: u64,
    },
    NoOutput {
        expected: PathBuf,
    },
    OutputNotAFile {
        expected: PathBuf,
    },
    OutputEscapes {
        expected: PathBuf,
        resolved: PathBuf,
    },
    OutputSchema {
        expected: PathBuf,
        problem: String,
        more: usize,
    },
    ContextExhausted,
    TurnLimit,
    Refused,
    TurnFailed {
        message: String,
    },
    SessionError {
        message: String,
    },
    UnsupportedSetting {
        field: String,
        value: String,
        available: Vec<String>,
    },
    PermissionDenied {
        tool: String,
    },
    SettingRejected {
        config_id: String,
        reason: String,
    },
    Exit {
        code: Option<i32>,
    },
    Interrupted,
}

/// How a started run ended. No `LockHeld` or `Invalid`: neither run ever
/// opened a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EndLine {
    Done,
    Failed {
        node: NodeId,
        failure: FailureLine,
    },
    Canceled {
        node: Option<NodeId>,
    },
    BudgetExhausted {
        limit: LimitLine,
    },
    Io {
        site: SiteLine,
        doing: String,
        path: PathBuf,
        message: String,
    },
    Unprovisioned {
        agent: String,
        message: String,
    },
    Stalled {
        nodes: Vec<NodeId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitLine {
    WallClock,
    NodeRuns,
    Cost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "at", rename_all = "snake_case")]
pub enum SiteLine {
    Run,
    Node { node: NodeId, attempt: u32 },
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn epoch_millis(at: std::time::SystemTime) -> u64 {
    at.duration_since(std::time::UNIX_EPOCH)
        .map(millis)
        .unwrap_or_default()
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

/// The settings a node can pin, so a replayed failure names the same
/// `&'static` field the live one did.
const SETTING_FIELDS: [&str; 3] = ["model", "effort", "mode"];
const UNKNOWN_SETTING: &str = "setting";

impl From<&NodeFailure> for FailureLine {
    fn from(failure: &NodeFailure) -> Self {
        match failure.clone() {
            NodeFailure::Timeout { elapsed } => Self::Timeout {
                elapsed_ms: millis(elapsed),
            },
            NodeFailure::NoOutput { expected } => Self::NoOutput { expected },
            NodeFailure::OutputNotAFile { expected } => Self::OutputNotAFile { expected },
            NodeFailure::OutputEscapes { expected, resolved } => {
                Self::OutputEscapes { expected, resolved }
            }
            NodeFailure::OutputSchema {
                expected,
                problem,
                more,
            } => Self::OutputSchema {
                expected,
                problem,
                more,
            },
            NodeFailure::ContextExhausted => Self::ContextExhausted,
            NodeFailure::TurnLimit => Self::TurnLimit,
            NodeFailure::Refused => Self::Refused,
            NodeFailure::TurnFailed(message) => Self::TurnFailed { message },
            NodeFailure::SessionError(message) => Self::SessionError { message },
            NodeFailure::UnsupportedSetting {
                field,
                value,
                available,
            } => Self::UnsupportedSetting {
                field: field.to_string(),
                value,
                available,
            },
            NodeFailure::PermissionDenied { tool } => Self::PermissionDenied { tool },
            NodeFailure::SettingRejected { config_id, reason } => {
                Self::SettingRejected { config_id, reason }
            }
            NodeFailure::Exit { code } => Self::Exit { code },
            NodeFailure::Interrupted => Self::Interrupted,
        }
    }
}

impl From<&FailureLine> for NodeFailure {
    fn from(line: &FailureLine) -> Self {
        match line.clone() {
            FailureLine::Timeout { elapsed_ms } => Self::Timeout {
                elapsed: Duration::from_millis(elapsed_ms),
            },
            FailureLine::NoOutput { expected } => Self::NoOutput { expected },
            FailureLine::OutputNotAFile { expected } => Self::OutputNotAFile { expected },
            FailureLine::OutputEscapes { expected, resolved } => {
                Self::OutputEscapes { expected, resolved }
            }
            FailureLine::OutputSchema {
                expected,
                problem,
                more,
            } => Self::OutputSchema {
                expected,
                problem,
                more,
            },
            FailureLine::ContextExhausted => Self::ContextExhausted,
            FailureLine::TurnLimit => Self::TurnLimit,
            FailureLine::Refused => Self::Refused,
            FailureLine::TurnFailed { message } => Self::TurnFailed(message),
            FailureLine::SessionError { message } => Self::SessionError(message),
            FailureLine::UnsupportedSetting {
                field,
                value,
                available,
            } => Self::UnsupportedSetting {
                field: SETTING_FIELDS
                    .into_iter()
                    .find(|known| *known == field)
                    .unwrap_or(UNKNOWN_SETTING),
                value,
                available,
            },
            FailureLine::PermissionDenied { tool } => Self::PermissionDenied { tool },
            FailureLine::SettingRejected { config_id, reason } => {
                Self::SettingRejected { config_id, reason }
            }
            FailureLine::Exit { code } => Self::Exit { code },
            FailureLine::Interrupted => Self::Interrupted,
        }
    }
}

impl From<BudgetLimit> for LimitLine {
    fn from(limit: BudgetLimit) -> Self {
        match limit {
            BudgetLimit::WallClock => Self::WallClock,
            BudgetLimit::NodeRuns => Self::NodeRuns,
            BudgetLimit::Cost => Self::Cost,
        }
    }
}

impl From<LimitLine> for BudgetLimit {
    fn from(limit: LimitLine) -> Self {
        match limit {
            LimitLine::WallClock => Self::WallClock,
            LimitLine::NodeRuns => Self::NodeRuns,
            LimitLine::Cost => Self::Cost,
        }
    }
}

impl From<&IoSite> for SiteLine {
    fn from(site: &IoSite) -> Self {
        match site {
            IoSite::Run => Self::Run,
            IoSite::Node { node, attempt } => Self::Node {
                node: node.clone(),
                attempt: *attempt,
            },
        }
    }
}

impl From<&SiteLine> for IoSite {
    fn from(site: &SiteLine) -> Self {
        match site {
            SiteLine::Run => Self::Run,
            SiteLine::Node { node, attempt } => Self::Node {
                node: node.clone(),
                attempt: *attempt,
            },
        }
    }
}

impl EndLine {
    /// `None` for the two outcomes of a run that never opened its log.
    pub(crate) fn of(outcome: &RunOutcome) -> Option<Self> {
        Some(match outcome {
            RunOutcome::Done => Self::Done,
            RunOutcome::Failed { node, failure } => Self::Failed {
                node: node.clone(),
                failure: failure.into(),
            },
            RunOutcome::Canceled { node } => Self::Canceled { node: node.clone() },
            RunOutcome::BudgetExhausted { limit } => Self::BudgetExhausted {
                limit: (*limit).into(),
            },
            RunOutcome::Io(e) => Self::Io {
                site: (&e.site).into(),
                doing: e.doing.to_string(),
                path: e.path.clone(),
                message: e.source.to_string(),
            },
            RunOutcome::Unprovisioned { agent, message } => Self::Unprovisioned {
                agent: agent.clone(),
                message: message.clone(),
            },
            RunOutcome::Stalled { nodes } => Self::Stalled {
                nodes: nodes.clone(),
            },
            RunOutcome::LockHeld { .. } | RunOutcome::Invalid { .. } => return None,
        })
    }

    pub(crate) fn to_run_end(&self) -> RunEnd {
        match self {
            Self::Done => RunEnd::Done,
            Self::Failed { node, failure } => RunEnd::Failed {
                node: node.clone(),
                failure: failure.into(),
            },
            Self::Canceled { node } => RunEnd::Canceled { node: node.clone() },
            Self::BudgetExhausted { limit } => RunEnd::BudgetExhausted {
                limit: (*limit).into(),
            },
            Self::Io {
                site,
                doing,
                path,
                message,
            } => RunEnd::Io {
                site: site.into(),
                doing: doing.clone(),
                path: path.clone(),
                message: message.clone(),
            },
            Self::Unprovisioned { agent, message } => RunEnd::Unprovisioned {
                agent: agent.clone(),
                message: message.clone(),
            },
            Self::Stalled { nodes } => RunEnd::Stalled {
                nodes: nodes.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A replayed failure is the failure the run reported, variant for
    /// variant — `NodeFailed` from the log must match the live one.
    #[test]
    fn every_failure_reads_back_as_itself() {
        let path = PathBuf::from("/run/out.md");
        let failures = [
            NodeFailure::Timeout {
                elapsed: Duration::from_millis(1500),
            },
            NodeFailure::NoOutput {
                expected: path.clone(),
            },
            NodeFailure::OutputNotAFile {
                expected: path.clone(),
            },
            NodeFailure::OutputEscapes {
                expected: path.clone(),
                resolved: PathBuf::from("/elsewhere"),
            },
            NodeFailure::OutputSchema {
                expected: path,
                problem: "missing `a`".to_string(),
                more: 2,
            },
            NodeFailure::ContextExhausted,
            NodeFailure::TurnLimit,
            NodeFailure::Refused,
            NodeFailure::TurnFailed("x".to_string()),
            NodeFailure::SessionError("y".to_string()),
            NodeFailure::UnsupportedSetting {
                field: "effort",
                value: "max".to_string(),
                available: vec!["low".to_string()],
            },
            NodeFailure::PermissionDenied {
                tool: "write".to_string(),
            },
            NodeFailure::SettingRejected {
                config_id: "model".to_string(),
                reason: "no".to_string(),
            },
            NodeFailure::Exit { code: Some(2) },
            NodeFailure::Exit { code: None },
            NodeFailure::Interrupted,
        ];
        for failure in failures {
            let line = FailureLine::from(&failure);
            let text = serde_json::to_string(&line).expect("serialize");
            let back: FailureLine = serde_json::from_str(&text).expect("deserialize");
            assert_eq!(NodeFailure::from(&back), failure);
        }
    }

    /// The two outcomes of a run that never opened its log have no line.
    #[test]
    fn only_a_started_run_has_an_end_to_record() {
        let refused = RunOutcome::LockHeld {
            holder: crate::lock::LockHolder {
                pid: 1,
                run_id: "other".to_string(),
                started_unix_secs: 0,
            },
        };
        assert!(EndLine::of(&refused).is_none());
        let io = RunOutcome::Io(crate::error::FlowIoError {
            site: IoSite::Node {
                node: "design".into(),
                attempt: 2,
            },
            doing: "reading the prompt file",
            path: PathBuf::from("gone.md"),
            source: std::io::Error::other("boom"),
        });
        let line = EndLine::of(&io).expect("a started run's end");
        assert_eq!(
            format!("{:?}", line.to_run_end()),
            format!("{:?}", RunEnd::from(&io))
        );
    }
}
