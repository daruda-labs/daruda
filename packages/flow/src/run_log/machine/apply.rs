//! Transitions. Infallible and deterministic: runs only on events `check`
//! accepted and `assign` numbered, and again on replay.

use super::event::{FlowRunEvent as E, Imported, NodeStart};
use super::state::{AttemptState, EndKind, FlowRunState, NodeState, OpenAsk, RunPhase, StartFacts};
use crate::NodeId;
use crate::run_log::line::EndLine;

pub(super) fn apply(state: &mut FlowRunState, event: &E) {
    match event {
        E::RunStarted {
            nodes,
            until,
            pinned,
            profile,
        } => start(state, nodes, until.clone(), pinned.clone(), profile.clone()),
        E::Imported(imported) => import(state, imported),
        E::Resumed { .. } => {
            for node in state.nodes.values_mut() {
                if let AttemptState::Started { attempt, .. } = node.last {
                    node.last = AttemptState::Interrupted { attempt };
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
                };
            }
            state.next_evidence = evidence + 1;
        }
        E::NodeRefused { .. } => {}
        E::AttemptPassed { node, record } => {
            settle(
                state,
                node,
                AttemptState::Passed {
                    attempt: record.attempt,
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
        E::OutputLost { node } => settle(state, node, AttemptState::NotRun),
        E::Invalidated { members, .. } => {
            for member in members {
                if let Some(current) = state.nodes.get_mut(member) {
                    current.invalidated = true;
                }
            }
        }
        E::RerunScheduled { members, .. } => {
            for member in members {
                if let Some(current) = state.nodes.get_mut(member) {
                    current.generation += 1;
                    current.attempts = 0;
                }
            }
        }
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
        if let Some(node) = state.nodes.get_mut(pin) {
            node.last = AttemptState::Pinned;
        }
    }
    state.start = StartFacts {
        order: nodes.iter().map(|node| node.id.clone()).collect(),
        until,
        pinned,
        profile,
    };
    state.phase = RunPhase::Running;
}

/// The old journal knew which nodes passed and nothing per generation, so
/// every other node starts over at attempt 1, as a resume did before.
fn import(state: &mut FlowRunState, imported: &Imported) {
    start(
        state,
        &imported.nodes,
        imported.until.clone(),
        imported.pinned.clone(),
        imported.profile.clone(),
    );
    for id in &imported.passed {
        if let Some(node) = state.nodes.get_mut(id) {
            node.last = AttemptState::Passed { attempt: 0 };
        }
    }
    state.next_evidence = imported.next_evidence;
    state.spent = imported.spent.clone();
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
        | EndLine::Unprovisioned { .. } => EndKind::Failed,
    }
}
