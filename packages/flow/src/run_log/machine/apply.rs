//! Transitions. Infallible and deterministic: runs only on events `check`
//! accepted and `assign` numbered, and again on replay.

use super::event::{FlowRunEvent as E, Imported, ImportedLast, NodeStart};
use super::state::{
    AttemptState, EndKind, FlowRunState, NodeState, OpenAsk, RunPhase, SpawnedProcess, StartFacts,
};
use crate::NodeId;
use crate::run_log::line::{EndLine, FailureLine};

pub(super) fn apply(state: &mut FlowRunState, event: &E) {
    match event {
        E::RunStarted {
            started_by,
            nodes,
            until,
            pinned,
            profile,
        } => {
            start(state, nodes, until.clone(), pinned.clone(), profile.clone());
            state.start.started_by = *started_by;
        }
        E::Imported(imported) => import(state, imported),
        E::Resumed { .. } => {
            for node in state.nodes.values_mut() {
                if let AttemptState::Started { attempt, .. } = node.last {
                    node.last = AttemptState::Interrupted { attempt };
                    node.crashed += 1;
                }
            }
            state.open_asks.clear();
            state.fixing.clear();
            state.resumes += 1;
            state.phase = RunPhase::Running;
        }
        E::AttemptStarted {
            node,
            attempt,
            evidence,
            ..
        } => {
            if let Some(current) = state.nodes.get_mut(node) {
                current.attempts = *attempt;
                current.last = AttemptState::Started {
                    attempt: *attempt,
                    evidence: *evidence,
                    process: None,
                };
            }
            state.next_evidence = evidence + 1;
        }
        E::NodeRefused { .. } => {}
        E::ProcessSpawned {
            node,
            leader,
            identity,
            ..
        } => {
            if let Some(NodeState {
                last: AttemptState::Started { process, .. },
                ..
            }) = state.nodes.get_mut(node)
            {
                *process = Some(SpawnedProcess {
                    leader: *leader,
                    identity: *identity,
                });
            }
        }
        E::AttemptPassed { node, record } => {
            passed(
                state,
                node,
                AttemptState::Passed {
                    attempt: Some(record.attempt),
                },
            );
            if let Some(current) = state.nodes.get_mut(node) {
                current.invalidated = false;
            }
            state.spent = record.spent.clone();
        }
        E::AttemptFailed {
            node,
            failure,
            record,
        } => {
            settle(
                state,
                node,
                AttemptState::Failed {
                    attempt: record.attempt,
                    failure: failure.clone(),
                },
            );
            state.spent = record.spent.clone();
        }
        E::AttemptCanceled { node, record } => {
            settle(
                state,
                node,
                AttemptState::Canceled {
                    attempt: record.attempt,
                },
            );
            state.spent = record.spent.clone();
        }
        E::OutputLost { node } => {
            if let Some(current) = state.nodes.get_mut(node) {
                current.last = AttemptState::NotRun;
                next_generation(current);
            }
        }
        E::Invalidated { members, .. } => {
            for member in members {
                if let Some(current) = state.nodes.get_mut(member) {
                    current.invalidated = true;
                    next_generation(current);
                }
            }
        }
        // The members' generations began when their passes were taken away,
        // so a crash before this record still reruns them counting from 1.
        E::RerunScheduled { .. } => {}
        E::FixStarted { gate, evidence } => {
            state.fixing.insert(gate.clone(), *evidence);
            state.next_evidence = evidence + 1;
        }
        E::FixEnded { gate, record, .. } => {
            state.fixing.remove(gate);
            state.spent = record.spent.clone();
        }
        E::AskRaised { owner, ask, id } => {
            state.open_asks.insert(
                *id,
                OpenAsk {
                    owner: owner.clone(),
                    ask: ask.clone(),
                },
            );
            state.next_ask = id + 1;
        }
        E::AskAnswered { id, .. } | E::AskAbandoned { id, .. } => {
            state.open_asks.remove(id);
        }
        E::RunEnded { end } => state.phase = RunPhase::Ended(end_kind(end)),
    }
}

fn start(
    state: &mut FlowRunState,
    nodes: &[NodeStart],
    until: Option<NodeId>,
    pinned: Vec<NodeId>,
    profile: Option<String>,
) {
    state.nodes = nodes
        .iter()
        .map(|node| (node.id.clone(), NodeState::new(node.produces)))
        .collect();
    for pin in &pinned {
        passed(state, pin, AttemptState::Pinned);
    }
    state.start = StartFacts {
        started_by: None,
        order: nodes.iter().map(|node| node.id.clone()).collect(),
        until,
        pinned,
        profile,
    };
    state.phase = RunPhase::Running;
}

/// The old journal knew which nodes passed and, per node, the settled
/// attempts of its current generation; those keep counting.
fn import(state: &mut FlowRunState, imported: &Imported) {
    start(
        state,
        &imported.nodes,
        imported.until.clone(),
        imported.pinned.clone(),
        imported.profile.clone(),
    );
    for id in &imported.passed {
        passed(state, id, AttemptState::Passed { attempt: None });
    }
    for carried in &imported.carried {
        if let Some(node) = state.nodes.get_mut(&carried.node) {
            let attempt = carried.attempts;
            node.attempts = attempt;
            node.last = match &carried.last {
                ImportedLast::Failed { reason } => AttemptState::Failed {
                    attempt,
                    failure: FailureLine::Recorded {
                        message: reason.clone(),
                    },
                },
                ImportedLast::Canceled => AttemptState::Canceled { attempt },
            };
        }
    }
    state.next_evidence = imported.next_evidence;
    state.spent = imported.spent.clone();
}

/// A node whose pass was taken away is re-derived from scratch: its next
/// attempt is the first of a new generation, with the whole cap to spend.
fn next_generation(node: &mut NodeState) {
    node.generation += 1;
    node.attempts = 0;
    node.crashed = 0;
}

/// Settle `node` as passed or pinned, numbering the pass.
fn passed(state: &mut FlowRunState, node: &NodeId, last: AttemptState) {
    if let Some(current) = state.nodes.get_mut(node) {
        state.passes += 1;
        current.passed_as = state.passes;
        current.last = last;
    }
}

fn settle(state: &mut FlowRunState, node: &NodeId, last: AttemptState) {
    if let Some(current) = state.nodes.get_mut(node) {
        current.last = last;
    }
}

pub(super) fn end_kind(end: &EndLine) -> EndKind {
    match end {
        EndLine::Done => EndKind::Done,
        EndLine::Canceled { .. } => EndKind::Canceled,
        EndLine::Stalled { .. } => EndKind::Stalled,
        EndLine::Failed { .. }
        | EndLine::BudgetExhausted { .. }
        | EndLine::Io { .. }
        | EndLine::Log { .. }
        | EndLine::Unprovisioned { .. } => EndKind::Failed,
    }
}
