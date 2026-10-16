//! Preconditions. Pure: never mutates, and accepts an event both as
//! proposed (numbers 0) and as `assign` filled it in.

use super::FlowRunRejected as R;
use super::event::FlowRunEvent as E;
use super::state::{AttemptState, FlowRunState, NodeState, RunPhase};
use crate::NodeId;
use crate::run_log::line::AttemptLine;
use crate::runner::AskOwner;

pub(super) fn check(state: &FlowRunState, event: &E) -> Result<(), R> {
    match event {
        E::RunStarted { .. } | E::Imported(_) => fresh(state),
        E::Resumed { .. } => {
            if !state.is_resumable() {
                return Err(R::NotResumable);
            }
            no_open_asks(state)
        }
        E::AttemptStarted {
            node,
            generation,
            attempt,
            evidence,
        } => {
            let current = running_node(state, node)?;
            if current.generation != *generation {
                return Err(R::WrongGeneration {
                    node: node.clone(),
                    current: current.generation,
                    proposed: *generation,
                });
            }
            idle(node, current)?;
            assigned(*attempt, current.attempts + 1)?;
            assigned(*evidence, next_evidence(state))
        }
        E::NodeRefused { node, attempt, .. } => {
            let current = running_node(state, node)?;
            idle(node, current)?;
            assigned(*attempt, current.attempts + 1)
        }
        E::ProcessSpawned { node, evidence, .. } => match running_node(state, node)?.last {
            AttemptState::Started {
                evidence: started,
                process: None,
                ..
            } if started == *evidence => Ok(()),
            _ => Err(R::NotInFlight(node.clone())),
        },
        E::AttemptPassed { node, record }
        | E::AttemptFailed { node, record, .. }
        | E::AttemptCanceled { node, record } => in_flight(state, node, record),
        E::OutputLost { node } => {
            let current = running_node(state, node)?;
            match current.last {
                AttemptState::Passed { .. } => Ok(()),
                _ => Err(R::NotPassed(node.clone())),
            }
        }
        E::Invalidated { members, .. } => {
            running(state)?;
            for member in members {
                let current = known(state, member)?;
                if !matches!(
                    current.last,
                    AttemptState::Passed { .. } | AttemptState::Pinned
                ) {
                    return Err(R::NotPassed(member.clone()));
                }
            }
            Ok(())
        }
        E::RerunScheduled { members, .. } => {
            running(state)?;
            for member in members {
                let current = known(state, member)?;
                let reusable = matches!(
                    current.last,
                    AttemptState::Passed { .. } | AttemptState::Pinned
                );
                if !reusable && !current.invalidated {
                    return Err(R::NotPassed(member.clone()));
                }
            }
            Ok(())
        }
        E::FixStarted { gate, evidence } => {
            running(state)?;
            known(state, gate)?;
            if state.fixing.contains_key(gate) {
                return Err(R::NoFix(gate.clone()));
            }
            assigned(*evidence, next_evidence(state))
        }
        E::FixEnded { gate, record, .. } => {
            running(state)?;
            match state.fixing.get(gate) {
                Some(evidence) if *evidence == record.evidence => Ok(()),
                _ => Err(R::NoFix(gate.clone())),
            }
        }
        E::AskRaised { owner, id, .. } => {
            running(state)?;
            owner_is_asking(state, owner)?;
            assigned_ask(*id, next_ask(state))
        }
        E::AskAnswered { id, .. } | E::AskAbandoned { id, .. } => {
            if state.open_asks.contains_key(id) {
                Ok(())
            } else {
                Err(R::NoSuchAsk(*id))
            }
        }
        E::RunEnded { .. } => {
            running(state)?;
            no_open_asks(state)
        }
    }
}

/// Numbers start at 1; a default state holds 0.
pub(super) fn next_evidence(state: &FlowRunState) -> u32 {
    state.next_evidence.max(1)
}

pub(super) fn next_ask(state: &FlowRunState) -> u64 {
    state.next_ask.max(1)
}

fn fresh(state: &FlowRunState) -> Result<(), R> {
    match state.phase {
        RunPhase::Fresh => Ok(()),
        _ => Err(R::RunAlreadyStarted),
    }
}

fn running(state: &FlowRunState) -> Result<(), R> {
    match state.phase {
        RunPhase::Running => Ok(()),
        _ => Err(R::NotRunning),
    }
}

fn known<'s>(state: &'s FlowRunState, node: &NodeId) -> Result<&'s NodeState, R> {
    state.node(node).ok_or_else(|| R::UnknownNode(node.clone()))
}

fn running_node<'s>(state: &'s FlowRunState, node: &NodeId) -> Result<&'s NodeState, R> {
    running(state)?;
    known(state, node)
}

fn idle(node: &NodeId, current: &NodeState) -> Result<(), R> {
    match current.last {
        AttemptState::Started { .. } => Err(R::InFlight(node.clone())),
        _ => Ok(()),
    }
}

fn in_flight(state: &FlowRunState, node: &NodeId, record: &AttemptLine) -> Result<(), R> {
    match running_node(state, node)?.last {
        AttemptState::Started {
            attempt, evidence, ..
        } if attempt == record.attempt && evidence == record.evidence => Ok(()),
        _ => Err(R::NotInFlight(node.clone())),
    }
}

fn owner_is_asking(state: &FlowRunState, owner: &AskOwner) -> Result<(), R> {
    match owner {
        AskOwner::Node {
            node,
            generation,
            attempt,
        } => {
            let current = known(state, node)?;
            match current.last {
                AttemptState::Started {
                    attempt: started, ..
                } if started == *attempt && current.generation == *generation => Ok(()),
                _ => Err(R::NotInFlight(node.clone())),
            }
        }
        AskOwner::Fix { gate } if state.fixing.contains_key(gate) => Ok(()),
        AskOwner::Fix { gate } => Err(R::NoFix(gate.clone())),
    }
}

fn no_open_asks(state: &FlowRunState) -> Result<(), R> {
    if state.has_open_asks() {
        return Err(R::OpenAsks(state.open_ask_ids()));
    }
    Ok(())
}

/// A proposed 0, or exactly the number `assign` would give.
fn assigned(proposed: u32, next: u32) -> Result<(), R> {
    if proposed == 0 || proposed == next {
        Ok(())
    } else {
        Err(R::WrongNumber)
    }
}

fn assigned_ask(proposed: u64, next: u64) -> Result<(), R> {
    if proposed == 0 || proposed == next {
        Ok(())
    } else {
        Err(R::WrongNumber)
    }
}
