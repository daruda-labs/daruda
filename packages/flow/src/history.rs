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
    pub flow: Option<Flow>,
    pub records: Vec<NodeRecord>,
    pub sessions: Vec<SessionBinding>,
    pub source: Option<PathBuf>,
    pub phase: Option<RunPhase>,
    pub torn: bool,
    pub started_at: Option<std::time::SystemTime>,
    pub elapsed: Option<std::time::Duration>,
    pub spent: reader::Spent,
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

impl RunHistory {
    pub fn read(dir: &Path) -> Result<Self, HistoryError> {
        let inspected = reader::inspect(dir).map_err(HistoryError)?;
        let state = inspected.as_ref().map(|i| i.state.clone());
        let started_at = inspected.as_ref().and_then(|i| i.started_at);
        let elapsed = inspected.as_ref().and_then(|i| i.elapsed);
        let in_flight = inspected
            .as_ref()
            .map(|i| {
                i.state
                    .node_states()
                    .filter_map(|(node, state)| match state.last() {
                        crate::run_log::machine::AttemptState::Started {
                            attempt,
                            evidence,
                            ..
                        } => Some((node.clone(), *attempt, *evidence)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (replay, phase, sessions, source, current_outputs) = match inspected {
            Some(i) => (
                i.replay,
                Some(i.state.phase()),
                i.state.sessions().cloned().collect(),
                i.state.source().map(Path::to_path_buf),
                i.state.finished(),
            ),
            None => {
                let replay = reader::read(dir);
                let current = replay.passed.iter().cloned().collect();
                (replay, None, Vec::new(), None, current)
            }
        };
        let flow = std::fs::read_to_string(dir.join(crate::resume::RUN_SPEC_FILE))
            .ok()
            .and_then(|text| crate::load(&text, None).ok())
            .map(|loaded| loaded.flow().clone());
        let mut result = Self {
            flow,
            started_at,
            elapsed,
            spent: replay.spent,
            records: replay.records,
            sessions,
            source,
            phase,
            in_flight,
            current_outputs,
            state,
            evidence_paths: Default::default(),
            attempt_order: replay.attempt_order,
            torn: replay.torn,
        };
        let nodes: std::collections::BTreeSet<_> = result
            .records
            .iter()
            .map(|r| r.id.clone())
            .chain(result.sessions.iter().map(|s| s.node.clone()))
            .chain(result.in_flight.iter().map(|(node, _, _)| node.clone()))
            .collect();
        for node in nodes {
            for (attempt, evidence) in result.attempts(&node) {
                let logs = dir.join(crate::schedule::LOG_DIR_NAME);
                let command = result.is_command(&node);
                let path = if command {
                    logs.join(format!("{node}.attempt-{attempt}.evidence-{evidence}.log"))
                } else {
                    let [modern, legacy] =
                        crate::runner::acp::transcript_paths(&logs, &node, attempt, evidence);
                    if !modern.exists()
                        && legacy.exists()
                        && !result.legacy_transcript_is_output(&node, &legacy)
                    {
                        legacy
                    } else {
                        modern
                    }
                };
                result.evidence_paths.insert((node.clone(), evidence), path);
            }
        }
        Ok(result)
    }

    fn legacy_transcript_is_output(&self, node: &NodeId, path: &Path) -> bool {
        let output = self
            .flow
            .as_ref()
            .and_then(|flow| flow.nodes.iter().find(|n| &n.id == node))
            .and_then(|n| n.kind.agent_body())
            .map(|body| body.output.as_path());
        if output.is_some_and(|p| p.extension().is_none_or(|ext| ext != "md")) {
            return false;
        }
        // Old Markdown archives shared the transcript name. Without a snapshot,
        // an invalidation naming that path is ambiguous and cannot prove a conversation.
        self.records
            .iter()
            .flat_map(|r| &r.attempts)
            .any(|attempt| {
                attempt.invalidated.nodes.contains(node)
                    && path.file_name()
                        == Some(std::ffi::OsStr::new(&crate::archive::archive_name(
                            node,
                            &format!(
                                "attempt-{}.evidence-{}",
                                attempt.attempt, attempt.evidence_seq
                            ),
                            output.unwrap_or(Path::new("output.md")),
                        )))
                    && attempt.invalidated.archived.iter().any(|p| p == path)
            })
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
        let nodes = self
            .flow
            .as_ref()
            .into_iter()
            .flat_map(|flow| &flow.nodes)
            .map(|n| &n.id)
            .chain(self.records.iter().map(|r| &r.id))
            .chain(self.sessions.iter().map(|s| &s.node));
        nodes.into_iter().find_map(|node| {
            self.attempts(node)
                .last()
                .map(|(_, evidence)| (node.clone(), *evidence))
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
