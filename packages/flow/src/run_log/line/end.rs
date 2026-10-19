//! How a started run ended, as the log keeps it.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::FailureLine;
use crate::NodeId;
use crate::error::IoSite;
use crate::event::RunEnd;
use crate::schedule::{BudgetLimit, RunOutcome};

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
    /// The log refused a record for a reason other than the disk, and could
    /// still take this one.
    Log {
        site: SiteLine,
        doing: String,
        path: PathBuf,
        message: String,
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
            RunOutcome::Log(e) => Self::Log {
                site: (&e.site).into(),
                doing: e.doing.to_string(),
                path: e.path.clone(),
                message: e.refusal.to_string(),
            },
            RunOutcome::LockHeld { .. }
            | RunOutcome::Invalid { .. }
            | RunOutcome::ShutDown
            | RunOutcome::NotResumed(_) => {
                return None;
            }
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
            Self::Log {
                site,
                doing,
                path,
                message,
            } => RunEnd::Log {
                site: site.into(),
                doing: doing.clone(),
                path: path.clone(),
                message: message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
