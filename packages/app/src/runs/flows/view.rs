//! One live run as a window draws it: the questions it holds open, the nodes
//! it has running, and each node's colour. GPUI-free, so every rule here is
//! covered by a plain test.
//!
//! Questions and progress are separate fields on purpose. Nodes running
//! together ask independently and keep working while one waits, so a
//! question is not a stage the run is in: an event from a sibling must never
//! be able to take one down, or its node waits on a reply nobody can send.
//! Only an answer, a stop, or the engine closing its reply channel does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use daruda_flow::NodeId;
use daruda_flow::event::FlowEvent;
use daruda_flow::runner::{AskClosed, PendingAsk};

use super::colour::{NodeRunState, NodeRunStates, RunColouring, apply_run_event};

/// Which flow file a run is of, when that is knowable — the key an open
/// graph is matched by.
///
/// Kept because `RunRequest` cannot say: a resumed run's `prepare` reads
/// `run.yaml` and sets `flow_dir` to the run directory, so the original
/// `.daruda/flows/x.yaml` is not recoverable from anything the request
/// carries. Closing that gap means writing the origin into the run directory
/// at start, a host/engine contract change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FlowSource {
    /// A fresh submission, which knows the file it was asked for.
    File(PathBuf),
    /// Picked up from a run directory; the originating file is unknown.
    Resumed { run_dir: PathBuf },
}

/// What a run says between nodes, when nothing is running.
///
/// Deliberately not "node N of M": a repair sends nodes back to pending, so
/// a count runs backwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lull {
    /// Submitted; the engine has not started a node yet.
    Starting,
    /// A resume waiting for what the dead run left to exit.
    Settling,
    /// The last node that started, and its try.
    After { node: NodeId, attempt: u32 },
    /// A fix is done and its gate is about to be re-derived.
    Rederiving { gate: NodeId },
}

impl Lull {
    /// Still before the first node.
    fn is_setup(&self) -> bool {
        matches!(self, Lull::Starting | Lull::Settling)
    }
}

/// A question waiting for a person, and the way to answer it.
#[derive(Debug)]
pub(crate) struct ParkedAsk {
    pending: PendingAsk,
}

impl ParkedAsk {
    pub(crate) fn new(pending: PendingAsk) -> Self {
        Self { pending }
    }

    pub(crate) fn ask_id(&self) -> u64 {
        self.pending.ask_id
    }

    /// The node the question is shown against: the asker, or the gate whose
    /// fix is asking.
    pub(crate) fn node(&self) -> &NodeId {
        self.pending.owner.node()
    }

    pub(crate) fn tool(&self) -> &str {
        &self.pending.request.tool
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        self.pending.request.detail.as_deref()
    }

    pub(crate) fn options(&self) -> &[daruda_acp::PermissionChoice] {
        &self.pending.request.options
    }

    /// The engine stopped waiting: the call that asked ended first.
    pub(crate) fn is_closed(&self) -> bool {
        self.pending.reply.is_closed()
    }
}

/// Why an answer was not delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AnswerRefused {
    /// No such question is open here: a click on a frame painted before it
    /// was answered. Nothing to tell anyone.
    NotOpen,
    /// The engine had already abandoned the question, so the answer reached
    /// nobody; the person is told rather than shown an answer that landed.
    Closed,
}

/// Whether anything a window draws changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Advanced {
    Nothing,
    /// It moved. `left_setup` is the one moment retention has run, so the
    /// history read from disk is stale exactly then.
    Moved {
        left_setup: bool,
    },
}

/// Where a newly arrived question went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Parked {
    /// No other question of this run was open: the one to bring forward.
    Alone,
    /// Beside others already open. Raising it too would stack dialogs.
    Beside,
}

/// One live run, as every window draws it.
pub(crate) struct RunView {
    run_dir: PathBuf,
    source: FlowSource,
    /// The flow's node ids as they were when the run was submitted. A file
    /// edited since can draw nodes this run never had, and an id taken by a
    /// different node would otherwise wear the old one's state.
    nodes_at_start: Vec<NodeId>,
    /// Every node's state, from the stream and reconciled with the log.
    /// Running and fixing nodes are the run's set of nodes under way.
    nodes: NodeRunStates,
    lull: Lull,
    /// Every open question by its log-assigned id: a set, not one slot.
    questions: BTreeMap<u64, ParkedAsk>,
}

impl RunView {
    pub(crate) fn new(run_dir: PathBuf, source: FlowSource, nodes_at_start: Vec<NodeId>) -> Self {
        Self {
            run_dir,
            source,
            nodes_at_start,
            nodes: NodeRunStates::new(),
            lull: Lull::Starting,
            questions: BTreeMap::new(),
        }
    }

    pub(crate) fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub(crate) fn source(&self) -> &FlowSource {
        &self.source
    }

    pub(crate) fn lull(&self) -> &Lull {
        &self.lull
    }

    /// The nodes under way, in the order the flow declares them.
    pub(crate) fn running(&self) -> Vec<(NodeId, NodeRunState)> {
        let mut running: Vec<(NodeId, NodeRunState)> = self
            .nodes
            .iter()
            .filter(|(_, state)| {
                matches!(state, NodeRunState::Running { .. } | NodeRunState::Fixing)
            })
            .map(|(id, state)| (id.clone(), *state))
            .collect();
        let rank = |id: &NodeId| {
            self.nodes_at_start
                .iter()
                .position(|known| known == id)
                .unwrap_or(usize::MAX)
        };
        running.sort_by(|(a, _), (b, _)| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        running
    }

    /// Every open question, oldest first.
    pub(crate) fn questions(&self) -> impl Iterator<Item = &ParkedAsk> {
        self.questions.values()
    }

    pub(super) fn waiting_on(&self) -> Option<super::super::result::WaitingOn> {
        self.questions().find(|ask| !ask.is_closed()).map(|ask| {
            super::super::result::WaitingOn::Question {
                node: ask.node().to_string(),
            }
        })
    }

    pub(crate) fn question(&self, ask_id: u64) -> Option<&ParkedAsk> {
        self.questions.get(&ask_id)
    }

    /// Fold one event in. Questions are untouched except for the ones the
    /// engine has already closed.
    pub(crate) fn advance(&mut self, event: &FlowEvent) -> Advanced {
        let nodes_moved = apply_run_event(&mut self.nodes, event);
        let lull = match event {
            FlowEvent::Settling { .. } => Some(Lull::Settling),
            FlowEvent::NodeStarted { node, attempt } => Some(Lull::After {
                node: node.clone(),
                attempt: *attempt,
            }),
            FlowEvent::FixEnded { gate, .. } | FlowEvent::Rerunning { gate, .. } => {
                Some(Lull::Rederiving { gate: gate.clone() })
            }
            _ => None,
        };
        let left_setup = lull.as_ref().is_some_and(|lull| !lull.is_setup()) && self.lull.is_setup();
        let lull_moved = lull.as_ref().is_some_and(|lull| *lull != self.lull);
        if let Some(lull) = lull {
            self.lull = lull;
        }
        let pruned = self.prune_closed();
        if nodes_moved || lull_moved || pruned {
            Advanced::Moved { left_setup }
        } else {
            Advanced::Nothing
        }
    }

    /// Replace every node's state with what the log says. The log is the
    /// record; the stream only says what happened while someone listened.
    pub(crate) fn reconcile(&mut self, from_log: NodeRunStates) -> bool {
        if self.nodes == from_log {
            return false;
        }
        self.nodes = from_log;
        true
    }

    /// Take a question. Never over another: each node waits on its own.
    pub(crate) fn park(&mut self, ask: ParkedAsk) -> Parked {
        self.prune_closed();
        let parked = if self.questions.is_empty() {
            Parked::Alone
        } else {
            Parked::Beside
        };
        self.questions.insert(ask.ask_id(), ask);
        parked
    }

    /// Answer question `ask_id` and take it down, delivered or not: a
    /// question the engine abandoned is never coming back.
    pub(crate) fn answer(
        &mut self,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
    ) -> Result<(), AnswerRefused> {
        let Some(question) = self.questions.remove(&ask_id) else {
            return Err(AnswerRefused::NotOpen);
        };
        question
            .pending
            .answer(decision)
            .map_err(|AskClosed| AnswerRefused::Closed)
    }

    /// Take every question down. Dropping the reply channels is what tells
    /// the engine nobody will answer them.
    pub(crate) fn withdraw_questions(&mut self) {
        self.questions.clear();
    }

    /// What this run makes of the flow it is of, when it can name one.
    pub(crate) fn colouring(&self) -> Option<(PathBuf, RunColouring)> {
        let FlowSource::File(path) = &self.source else {
            return None;
        };
        Some((
            path.clone(),
            RunColouring {
                states: self.nodes.clone(),
                of_nodes: self.nodes_at_start.clone(),
            },
        ))
    }

    /// The colouring for this run when it is of `path`.
    pub(crate) fn colouring_of(&self, path: &Path) -> Option<RunColouring> {
        let (running, colouring) = self.colouring()?;
        (running == path).then_some(colouring)
    }

    #[cfg(test)]
    pub(crate) fn node_state(&self, id: &str) -> NodeRunState {
        self.nodes
            .get(&NodeId::from(id))
            .copied()
            .unwrap_or_default()
    }

    fn prune_closed(&mut self) -> bool {
        let before = self.questions.len();
        self.questions.retain(|_, question| !question.is_closed());
        self.questions.len() != before
    }
}

#[cfg(test)]
mod tests;
