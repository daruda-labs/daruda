//! What a run says about each of its nodes, folded from its stream and
//! reconciled with its log. GPUI-free: the graph draws it, the run owns it.

use daruda_flow::NodeId;
use daruda_flow::event::FlowEvent;
use daruda_flow::run_log::machine::{AttemptState, FlowRunState};

/// Where a node is in the run driving this graph.
///
/// `Pending` is also the answer when no run drives it at all — a flow
/// opened from a file is every node pending, which is exactly how it
/// should read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NodeRunState {
    #[default]
    Pending,
    Running {
        attempt: u32,
    },
    Passed,
    Failed,
    /// A gate running its repair's `fix` session.
    Fixing,
}

/// Per-node state a run has reported so far. Absent means [`NodeRunState::Pending`]
/// — a node the run has not spoken about yet is one that has not started.
pub(crate) type NodeRunStates = std::collections::HashMap<NodeId, NodeRunState>;

/// What a run says about the flow a graph is drawing.
///
/// The states and the nodes they are *about* travel together and are only
/// meaningful together: an id freed by a delete and taken by a rename would
/// otherwise wear the first node's state. One value so a caller cannot pass
/// one without the other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunColouring {
    pub states: NodeRunStates,
    /// The flow's node ids as they were when the run was submitted.
    pub of_nodes: Vec<NodeId>,
}

impl RunColouring {
    /// Whether this colouring is about `nodes` — the same ids, in the same
    /// order. A graph that has changed shape since the run started is left
    /// uncoloured rather than coloured wrongly.
    pub(crate) fn is_about(&self, nodes: impl Iterator<Item = NodeId>) -> bool {
        self.of_nodes.iter().cloned().eq(nodes)
    }
}

/// Fold one event into the per-node states.
///
/// Pure so the projection is testable without a run: every rule here is a
/// claim about what the engine's stream means, and getting one wrong leaves
/// a node showing the wrong colour for the rest of the run.
pub(crate) fn apply_run_event(states: &mut NodeRunStates, event: &FlowEvent) {
    match event {
        // The node set comes from the file, which is already drawn. Seeding
        // from the run as well would make a file edited mid-run disagree with
        // itself, and the states default to `Pending` regardless.
        FlowEvent::RunStarted { .. } => {}
        FlowEvent::NodeStarted { node, attempt } => {
            states.insert(node.clone(), NodeRunState::Running { attempt: *attempt });
        }
        FlowEvent::NodePassed { node, .. } => {
            states.insert(node.clone(), NodeRunState::Passed);
        }
        FlowEvent::NodeFailed { node, .. } => {
            states.insert(node.clone(), NodeRunState::Failed);
        }
        FlowEvent::FixStarted { gate } => {
            states.insert(gate.clone(), NodeRunState::Fixing);
        }
        // The fix is over but the gate has not re-run, so it is still the
        // failure it was — not a pass, and no longer fixing.
        FlowEvent::FixEnded { gate, .. } => {
            states.insert(gate.clone(), NodeRunState::Failed);
        }
        // The one transition a host cannot infer (`event.rs` says so): these
        // already passed, and are about to be derived again. Left alone they
        // would keep the colour of a result that no longer holds.
        FlowEvent::Rerunning { members, .. } => {
            for node in members {
                states.insert(node.clone(), NodeRunState::Pending);
            }
        }
        // Whatever the last word on each node was, it stands.
        FlowEvent::RunEnded { .. } => {}
    }
}

/// Every node's state as the run's log has it.
///
/// The stream says only what happened while a watcher listened: a resumed
/// run never re-announces the nodes an earlier process passed, and a stopped
/// attempt narrates nothing at all. The log is what settles both (F12).
pub(crate) fn states_from_log(log: &FlowRunState) -> NodeRunStates {
    log.node_states()
        .map(|(id, node)| {
            let state = if log.is_fixing(id) {
                NodeRunState::Fixing
            } else {
                match node.last() {
                    AttemptState::Started { attempt, .. } => {
                        NodeRunState::Running { attempt: *attempt }
                    }
                    AttemptState::Passed { .. } | AttemptState::Pinned
                        if !node.is_invalidated() =>
                    {
                        NodeRunState::Passed
                    }
                    AttemptState::Failed { .. } => NodeRunState::Failed,
                    // Stopped, cut off by a dead process, never run, or a pass
                    // a repair took back: none of these is a result that holds.
                    AttemptState::Passed { .. }
                    | AttemptState::Pinned
                    | AttemptState::NotRun
                    | AttemptState::Canceled { .. }
                    | AttemptState::Interrupted { .. } => NodeRunState::Pending,
                }
            };
            (NodeId::clone(id), state)
        })
        .collect()
}

#[cfg(test)]
mod tests {}
