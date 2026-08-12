//! What a node's failure means: which `on_fail` policy applies, what that
//! policy invalidates, and which nodes it re-derives. Separated from the
//! drive loop because the loop only asks "retry, repair or stop" — how that
//! answer is computed is a question of its own.

use super::Run;
use crate::NodeId;
use crate::model::{AgentFail, GateFail, Node, NodeKind, Prompt};
use std::path::PathBuf;
use std::time::Duration;

/// `AgentFail` and `GateFail` normalised into one shape, so the loop has a
/// single cap check. `Halt` is `max_attempts = 1`: the first failure is
/// already at the cap.
pub(super) struct Policy {
    pub(super) kind: PolicyKind,
    pub(super) max_attempts: u32,
    pub(super) wait: Duration,
}

pub(super) enum PolicyKind {
    Halt,
    Retry { hint: Prompt },
    Repair { fix: String, rerun: Vec<NodeId> },
}

impl Run<'_> {
    pub(super) fn policy_of(&self, node: &Node) -> Policy {
        match &node.kind {
            NodeKind::Agent {
                on_fail:
                    AgentFail::Retry {
                        hint,
                        max_attempts,
                        wait,
                    },
                ..
            } => Policy {
                kind: PolicyKind::Retry { hint: hint.clone() },
                max_attempts: *max_attempts,
                wait: *wait,
            },
            NodeKind::Command {
                on_fail:
                    GateFail::Repair {
                        fix,
                        rerun,
                        max_attempts,
                        wait,
                    },
                ..
            } => Policy {
                kind: PolicyKind::Repair {
                    fix: fix.clone(),
                    rerun: rerun.clone(),
                },
                max_attempts: *max_attempts,
                wait: *wait,
            },
            NodeKind::Agent {
                on_fail: AgentFail::Halt,
                ..
            }
            | NodeKind::Command {
                on_fail: GateFail::Halt,
                ..
            } => Policy {
                kind: PolicyKind::Halt,
                max_attempts: 1,
                wait: Duration::ZERO,
            },
        }
    }

    /// What this failure invalidates: for `Repair` the rerun members plus
    /// the gate, for `Retry` and `Halt` just this node. Each entry is a
    /// node and the output path to move aside.
    pub(super) fn invalidation_set(
        &self,
        node: &Node,
        policy: &Policy,
    ) -> Vec<(NodeId, Option<PathBuf>)> {
        let mut ids = match &policy.kind {
            PolicyKind::Repair { rerun, .. } => self.rerun_members(rerun, &node.id),
            PolicyKind::Retry { .. } | PolicyKind::Halt => Vec::new(),
        };
        ids.push(node.id.clone());
        ids.into_iter()
            .map(|id| {
                let output = self.node_outputs.get(&id).cloned();
                (id, output)
            })
            .collect()
    }

    /// `graph.rerun_closure(rerun)` intersected with `executed`, minus the
    /// gate (this `drive` call re-runs it) and minus anything already on
    /// the drive stack (it will re-run when its own loop comes round;
    /// driving it here is what makes two mutually-rerunning gates recurse
    /// until the stack aborts).
    pub(super) fn rerun_members(&self, rerun: &[NodeId], gate: &NodeId) -> Vec<NodeId> {
        let executed = self.executed.borrow();
        let driving = self.driving.borrow();
        self.graph
            .rerun_closure(rerun)
            .into_iter()
            .filter(|id| id != gate && executed.contains(id) && !driving.contains(id))
            .collect()
    }
}
