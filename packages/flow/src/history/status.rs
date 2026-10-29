//! Semantic node status shared by history summaries and their visual adapters.

use super::*;
use crate::run_log::machine::AttemptState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    Pending,
    Running { attempt: u32 },
    Passed,
    Failed,
    Canceled,
    Interrupted,
    Pinned,
    Fixing,
}

impl NodeStatus {
    pub fn from_outcome(outcome: &AttemptOutcome) -> Self {
        match outcome {
            AttemptOutcome::Passed => Self::Passed,
            AttemptOutcome::Canceled => Self::Canceled,
            _ => Self::Failed,
        }
    }
    pub fn is_complete(self) -> bool {
        matches!(self, Self::Passed | Self::Pinned)
    }
}

impl RunHistory {
    pub fn node_status(&self, node: &NodeId) -> NodeStatus {
        if let Some(log) = &self.state
            && let Some(state) = log.node(node)
        {
            if log.is_fixing(node) {
                return NodeStatus::Fixing;
            }
            return match state.last() {
                AttemptState::NotRun => NodeStatus::Pending,
                // Invalidation revokes the previous result, not a newer attempt.
                AttemptState::Pinned | AttemptState::Passed { .. } if state.is_invalidated() => {
                    NodeStatus::Pending
                }
                AttemptState::Pinned => NodeStatus::Pinned,
                AttemptState::Started { attempt, .. } => NodeStatus::Running { attempt: *attempt },
                AttemptState::Passed { .. } => NodeStatus::Passed,
                AttemptState::Failed { .. } => NodeStatus::Failed,
                AttemptState::Canceled { .. } => NodeStatus::Canceled,
                AttemptState::Interrupted { .. } => NodeStatus::Interrupted,
            };
        }
        self.attempts(node)
            .last()
            .and_then(|(_, seq)| self.attempt(node, *seq))
            .map(|record| NodeStatus::from_outcome(&record.outcome))
            .unwrap_or(NodeStatus::Pending)
    }
}

#[cfg(test)]
mod tests;
