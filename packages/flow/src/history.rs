//! Read-only execution details. Progress is replayed strictly, never inferred from a report.

use crate::run_log::{
    machine::{EndKind, RunPhase},
    reader,
};
use crate::session::SessionBinding;
use crate::{
    NodeId,
    model::Flow,
    record::{AttemptOutcome, AttemptRecord, NodeRecord},
};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct RunHistory {
    flow: Option<Flow>,
    records: Vec<NodeRecord>,
    sessions: Vec<SessionBinding>,
    source: Option<PathBuf>,
    phase: Option<RunPhase>,
    torn: bool,
    started_at: Option<std::time::SystemTime>,
    elapsed: Option<std::time::Duration>,
    spent: reader::Spent,
    state: Option<crate::run_log::machine::FlowRunState>,
    evidence_paths: std::collections::BTreeMap<(NodeId, u32), PathBuf>,
    attempt_order: Vec<(NodeId, u32)>,
    in_flight: Vec<(NodeId, u32, u32)>,
    current_outputs: std::collections::BTreeSet<NodeId>,
}

#[derive(Debug)]
pub struct HistoryError(pub daruda_runlog::OpenError);
impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for HistoryError {}

mod load;
mod status;
pub use status::NodeStatus;

impl RunHistory {
    pub fn flow(&self) -> Option<&Flow> {
        self.flow.as_ref()
    }
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }
    pub fn started_at(&self) -> Option<std::time::SystemTime> {
        self.started_at
    }
    pub fn elapsed(&self) -> Option<std::time::Duration> {
        self.elapsed
    }
    pub fn spent(&self) -> &reader::Spent {
        &self.spent
    }

    /// Snapshot order first, then evidence and nodes known only to the durable log.
    pub fn nodes(&self) -> Vec<NodeId> {
        let mut nodes: Vec<_> = self
            .flow
            .as_ref()
            .map(|f| f.nodes.iter().map(|n| n.id.clone()).collect())
            .unwrap_or_default();
        for node in self
            .records
            .iter()
            .map(|r| &r.id)
            .chain(self.sessions.iter().map(|s| &s.node))
            .chain(
                self.state
                    .iter()
                    .flat_map(|state| state.node_states().map(|(id, _)| id)),
            )
        {
            if !nodes.contains(node) {
                nodes.push(node.clone());
            }
        }
        nodes
    }

    /// Stalled runs can be resumed by the engine and still own their sessions.
    pub fn can_continue(&self) -> bool {
        matches!(
            self.phase,
            Some(RunPhase::Ended(
                EndKind::Done | EndKind::Failed | EndKind::Canceled
            ))
        ) && !self.torn
    }

    pub fn attempts(&self, node: &NodeId) -> Vec<(u32, u32)> {
        let mut attempts: Vec<_> = self
            .records
            .iter()
            .filter(|r| &r.id == node)
            .flat_map(|r| &r.attempts)
            .map(|a| (a.attempt, a.evidence_seq))
            .collect();
        attempts.extend(
            self.sessions
                .iter()
                .filter(|s| &s.node == node && s.attempt > 0)
                .map(|s| (s.attempt, s.evidence)),
        );
        attempts.extend(
            self.in_flight
                .iter()
                .filter(|(id, _, _)| id == node)
                .map(|(_, attempt, evidence)| (*attempt, *evidence)),
        );
        attempts.sort_by_key(|(_, evidence)| *evidence);
        attempts.dedup_by_key(|(_, evidence)| *evidence);
        attempts
    }

    pub fn latest_selection(&self) -> Option<(NodeId, u32)> {
        self.nodes().into_iter().find_map(|node| {
            self.attempts(&node)
                .last()
                .map(|(_, evidence)| (node, *evidence))
        })
    }

    pub fn state(&self) -> Option<&crate::run_log::machine::FlowRunState> {
        self.state.as_ref()
    }

    pub fn is_command(&self, node: &NodeId) -> bool {
        self.flow
            .as_ref()
            .and_then(|flow| flow.nodes.iter().find(|n| &n.id == node))
            .map(|node| node.kind.agent_body().is_none())
            .or_else(|| {
                self.state
                    .as_ref()?
                    .node(node)
                    .map(|node| node.produces == crate::run_log::machine::Produces::Nothing)
            })
            .unwrap_or(false)
    }

    pub fn attempt(&self, node: &NodeId, evidence: u32) -> Option<&AttemptRecord> {
        self.records
            .iter()
            .find(|record| &record.id == node)?
            .attempts
            .iter()
            .find(|attempt| attempt.evidence_seq == evidence)
    }

    pub fn binding(&self, node: &NodeId, evidence: u32) -> Option<&SessionBinding> {
        self.sessions
            .iter()
            .find(|binding| &binding.node == node && binding.evidence == evidence)
    }

    pub fn transcript(&self, _dir: &Path, node: &NodeId, evidence: u32) -> Option<PathBuf> {
        self.evidence_paths.get(&(node.clone(), evidence)).cloned()
    }

    /// Resolve the first invalidation after this evidence, rather than another attempt's live output.
    pub fn output(&self, dir: &Path, node: &NodeId, evidence: u32) -> Option<PathBuf> {
        let selected = self.attempt(node, evidence)?;
        let output = &self
            .flow
            .as_ref()?
            .nodes
            .iter()
            .find(|n| &n.id == node)?
            .kind
            .agent_body()?
            .output;
        if matches!(selected.outcome, AttemptOutcome::Canceled) {
            return selected
                .invalidated
                .archived
                .iter()
                .find(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.starts_with(&format!("{node}.")) && n.contains("canceled")
                    })
                })
                .cloned();
        }
        let selected_at = self
            .attempt_order
            .iter()
            .position(|(id, seq)| id == node && *seq == evidence);
        let invalidation = selected_at.and_then(|at| {
            self.attempt_order[at..]
                .iter()
                .filter_map(|(id, seq)| self.attempt(id, *seq))
                .find(|a| a.invalidated.nodes.contains(node))
        });
        if let Some(invalidation) = invalidation {
            let expected = crate::archive::archive_name(
                node,
                &format!(
                    "attempt-{}.evidence-{}",
                    invalidation.attempt, invalidation.evidence_seq
                ),
                output,
            );
            return invalidation
                .invalidated
                .archived
                .iter()
                .find(|p| p.file_name() == Some(std::ffi::OsStr::new(&expected)))
                .cloned()
                .or_else(|| {
                    if matches!(selected.outcome, AttemptOutcome::Canceled) {
                        invalidation
                            .invalidated
                            .archived
                            .iter()
                            .find(|p| {
                                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                                    n.starts_with(&format!("{node}.")) && n.contains("canceled")
                                })
                            })
                            .cloned()
                    } else {
                        None
                    }
                });
        }
        let latest = self
            .records
            .iter()
            .find(|r| &r.id == node)?
            .attempts
            .last()?;
        (self.current_outputs.contains(node)
            && latest.evidence_seq == evidence
            && matches!(selected.outcome, AttemptOutcome::Passed))
        .then(|| dir.join(output))
    }
}

#[cfg(test)]
mod tests;
