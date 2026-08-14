//! The runs this window started, and what each one is doing.
//!
//! A type rather than a field on `Workspace`: "which lane is running what" is
//! one thing with its own rules — a question queue that must not lose a parked
//! node, a stage that only moves forward, an id counter — and those rules were
//! spelled out across four files that all reached into one `HashMap`.
//!
//! GPUI-free, so every rule here is covered by a plain test. What the workspace
//! keeps is the part that needs a window: telling the world, raising a modal,
//! opening a report.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use daruda_flow::event::FlowEvent;
use daruda_flow::runner::CancelToken;
use daruda_store::project::LaneRef;

use super::flow_request::FlowSource;
use super::main_area::flow_graph_pane::model::{NodeRunStates, RunColouring, apply_run_event};
use crate::surface::strings as s;

/// A run in flight. The token is the whole of the stop switch; the handle
/// is kept so the workspace can tell a finished run from a wedged one
/// without asking the engine.
pub(in crate::workspace) struct RunHandle {
    pub cancel: CancelToken,
    pub run_dir: PathBuf,
    /// What the run is doing right now, as its own stream reports it.
    pub doing: RunStage,
    /// Which flow file this run is of, when that is knowable — the key a
    /// graph pane is matched by (see [`FlowSource`]).
    pub source: FlowSource,
    /// Every node's state as the stream has reported it so far. `doing` says
    /// what is happening *now*; this is what a graph needs to colour all of
    /// it at once, including the nodes a repair sent back.
    pub nodes: NodeRunStates,
    /// The flow's node ids as they were when the run was submitted. A run
    /// executes the flow it resolved at the start, so a file edited since can
    /// draw nodes this run never had — and an id taken by a different node
    /// would otherwise be painted with the old one's state.
    pub nodes_at_start: Vec<String>,
    _thread: std::thread::JoinHandle<()>,
}

/// Where a run is, in the only terms that stay true.
///
/// Deliberately not "node N of M": a repair sends nodes back to pending,
/// so a count runs backwards — and repair is this engine's headline
/// feature, not an edge case. What a person actually wants at a glance is
/// which node is on, and whether it is on its second try.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum RunStage {
    /// Submitted; the engine has not announced a node yet.
    Starting,
    Node {
        id: String,
        attempt: u32,
    },
    /// A gate's repair is running its fix session.
    Fixing {
        gate: String,
    },
    /// The fix is done and the gate is about to be re-derived. Not
    /// `Node { attempt: 0 }`: a zeroth attempt does not exist, and the
    /// number would have to be read as a sentinel at every use.
    Rederiving {
        gate: String,
    },
    /// Waiting for a person to answer a permission question. Both clocks
    /// are stopped, so the only thing that ends this is an answer or a Stop.
    ///
    /// A variant rather than a flag beside an `Option`: a stage that is
    /// "asking" and has no question is not a state worth representing.
    ///
    /// More than one at a time is now possible — nodes running together ask
    /// independently — so the rest wait behind the one on screen. The queue
    /// lives *in* the variant because a queue of questions in a run that is
    /// not asking is not a state either.
    Asking {
        question: std::sync::Arc<ParkedAsk>,
        /// Arrived while `question` was up. Answering promotes the front.
        queued: std::collections::VecDeque<std::sync::Arc<ParkedAsk>>,
    },
}

/// A question waiting for an answer, and the way to send one.
///
/// `PartialEq` by `ask_id` alone: the reply channel has no equality, and
/// what the comparison is actually for is "is this the same question" —
/// which the id answers. `RunStage` keeps its derive because of it.
#[derive(Debug)]
pub(in crate::workspace) struct ParkedAsk {
    pub ask_id: u64,
    pub node: String,
    /// The attempt that asked — what the run goes back to being on once
    /// the question is answered.
    pub attempt: u32,
    pub tool: String,
    pub detail: Option<String>,
    pub options: Vec<daruda_acp::PermissionChoice>,
    reply: smol::channel::Sender<daruda_acp::PermissionDecision>,
}

impl RunHandle {
    /// A run that has just been submitted. The thread is taken so the handle
    /// can tell a finished run from a wedged one without asking the engine.
    pub(in crate::workspace) fn started(
        cancel: CancelToken,
        run_dir: PathBuf,
        source: FlowSource,
        nodes_at_start: Vec<String>,
        thread: std::thread::JoinHandle<()>,
    ) -> Self {
        Self {
            cancel,
            run_dir,
            doing: RunStage::Starting,
            source,
            nodes: NodeRunStates::new(),
            nodes_at_start,
            _thread: thread,
        }
    }

    /// A run with no engine behind it, for tests and for `--screenshot`.
    ///
    /// The node ids come off the file the seed names, the way a real
    /// submission takes them off what it loaded — a graph is only coloured
    /// when the run and the file still agree about which nodes exist.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn seeded(
        run_dir: PathBuf,
        source: FlowSource,
        doing: RunStage,
    ) -> Self {
        let nodes_at_start = match &source {
            FlowSource::File(path) => std::fs::read_to_string(path)
                .ok()
                .and_then(|text| daruda_flow::parse::parse_flow_file(&text).ok())
                .map(|file| file.nodes.iter().map(|n| n.id.clone()).collect())
                .unwrap_or_default(),
            FlowSource::Resumed { .. } => Vec::new(),
        };
        Self {
            cancel: CancelToken::default(),
            run_dir,
            doing,
            source,
            nodes: NodeRunStates::new(),
            nodes_at_start,
            _thread: std::thread::spawn(|| {}),
        }
    }
}

impl ParkedAsk {
    pub(in crate::workspace) fn new(pending: daruda_flow::runner::PendingAsk) -> Self {
        Self {
            ask_id: pending.ask_id,
            node: pending.node,
            attempt: pending.attempt,
            tool: pending.request.tool,
            detail: pending.request.detail,
            options: pending.request.options,
            reply: pending.reply,
        }
    }
}

impl PartialEq for ParkedAsk {
    fn eq(&self, other: &Self) -> bool {
        self.ask_id == other.ask_id
    }
}
impl Eq for ParkedAsk {}

impl RunStage {
    /// What to show for this stage. `attempt` is only worth saying past the
    /// first — every node has a first try, and "try 1" on every row is
    /// noise that buries the row where it says "try 3".
    pub(in crate::workspace) fn describe(&self) -> String {
        match self {
            RunStage::Starting => s::status_bar_flow_stage_starting(),
            RunStage::Node { id, attempt } if *attempt > 1 => {
                s::status_bar_flow_stage_node_retry(id, *attempt)
            }
            RunStage::Node { id, .. } => s::status_bar_flow_stage_node(id),
            RunStage::Fixing { gate } => s::status_bar_flow_stage_fixing(gate),
            RunStage::Rederiving { gate } => s::status_bar_flow_stage_rederiving(gate),
            // Names the node like every other stage does: dropping it while
            // asking would leave the one stage that does not say where the
            // run is. For a repair's fix session the engine sends the gate's
            // name, which is what makes that case renderable at all.
            RunStage::Asking { question, .. } => {
                s::status_bar_flow_stage_asking(&question.node, &question.tool)
            }
        }
    }
}

/// Every run this window started, by the lane holding it.
///
/// Keyed rather than single because lanes run in parallel — that is the whole
/// point of the app — so a second flow in another lane must not displace the
/// first one's cancel token, and a run ending must find *its own* handle rather
/// than whichever lane happens to be active. A run held by *another process* is
/// not in here at all and is recognised through the lock instead.
#[derive(Default)]
pub(in crate::workspace) struct FlowRuns {
    runs: HashMap<LaneRef, RunHandle>,
    /// Distinguishes two runs started in the same millisecond.
    counter: u32,
}

/// What [`FlowRuns::advance_stage`] did, in the terms the caller acts on.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Advanced {
    /// The run is already saying this, or there is no run.
    Nothing,
    /// It moved. `left_setup` is the one moment retention has run, so the
    /// history read from disk is stale exactly then.
    Moved { left_setup: bool },
}

/// Where a newly arrived question went.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Parked {
    /// Not this run's — a stale watcher for a directory this lane no longer
    /// holds.
    Elsewhere,
    /// Behind the one already up. Nothing new goes on screen.
    Queued,
    /// It is the question on screen now.
    Showing,
}

impl FlowRuns {
    pub(in crate::workspace) fn is_running(&self, lane: LaneRef) -> bool {
        self.runs.contains_key(&lane)
    }

    pub(in crate::workspace) fn insert(&mut self, lane: LaneRef, handle: RunHandle) {
        self.runs.insert(lane, handle);
    }

    /// Retire the run `lane` holds and hand back where it wrote.
    pub(in crate::workspace) fn retire(&mut self, lane: LaneRef) -> Option<PathBuf> {
        self.runs.remove(&lane).map(|handle| handle.run_dir)
    }

    pub(in crate::workspace) fn iter(&self) -> impl Iterator<Item = (LaneRef, &RunHandle)> {
        self.runs.iter().map(|(lane, handle)| (*lane, handle))
    }

    /// The next run id. Wrapping because it only has to differ from the last
    /// one, not count anything.
    pub(in crate::workspace) fn next_run_id(&mut self) -> u32 {
        self.counter = self.counter.wrapping_add(1);
        self.counter
    }

    /// Move the run to the stage `event` implies. Events that describe nothing
    /// a person watches leave it where it is.
    pub(in crate::workspace) fn advance_stage(
        &mut self,
        lane: LaneRef,
        event: &FlowEvent,
    ) -> Advanced {
        let Some(stage) = stage_for(event) else {
            return Advanced::Nothing;
        };
        let Some(handle) = self.runs.get_mut(&lane) else {
            return Advanced::Nothing;
        };
        if handle.doing == stage {
            return Advanced::Nothing;
        }
        let left_setup = handle.doing == RunStage::Starting;
        handle.doing = stage;
        Advanced::Moved { left_setup }
    }

    /// Fold the event into the run's per-node states, and say which flow file
    /// the result is about — a run that cannot name one colours no graph.
    pub(in crate::workspace) fn colour_after(
        &mut self,
        lane: LaneRef,
        event: &FlowEvent,
    ) -> Option<(PathBuf, RunColouring)> {
        let handle = self.runs.get_mut(&lane)?;
        apply_run_event(&mut handle.nodes, event);
        handle.colouring()
    }

    /// The colouring for the run in `lane`, when that run is of `path`.
    ///
    /// Matched by lane *and* path, because one lane can hold graphs of several
    /// flows and only one of them is the one running.
    pub(in crate::workspace) fn colouring_of(
        &self,
        lane: LaneRef,
        path: &Path,
    ) -> Option<RunColouring> {
        let (running, colouring) = self.runs.get(&lane)?.colouring()?;
        (running == path).then_some(colouring)
    }

    /// Take a question. Behind the one already up, never over it: nodes running
    /// together ask independently, and a second question that replaced the
    /// first would leave that first node parked on a reply nobody can send.
    pub(in crate::workspace) fn park_ask(
        &mut self,
        lane: LaneRef,
        run_dir: &Path,
        ask: ParkedAsk,
    ) -> Parked {
        let Some(handle) = self.runs.get_mut(&lane) else {
            return Parked::Elsewhere;
        };
        if handle.run_dir != run_dir {
            return Parked::Elsewhere;
        }
        let arrived = std::sync::Arc::new(ask);
        match &mut handle.doing {
            RunStage::Asking { queued, .. } => {
                queued.push_back(arrived);
                Parked::Queued
            }
            doing => {
                *doing = RunStage::Asking {
                    question: arrived,
                    queued: std::collections::VecDeque::new(),
                };
                Parked::Showing
            }
        }
    }

    /// Answer the question `lane` is holding, and put the next one up.
    ///
    /// `ask_id` is checked rather than trusted: a surface can still be painted
    /// with a question that has already been answered, and a click on that
    /// frame must do nothing instead of answering the *next* one.
    ///
    /// Settled here rather than when the run's next event arrives: the agent
    /// goes back to work for as long as it likes, and until then the question
    /// and its buttons would still be on screen with no sign the click did
    /// anything. Observed as "the button does nothing" — and answered again.
    pub(in crate::workspace) fn answer_ask(
        &mut self,
        lane: LaneRef,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
    ) -> bool {
        let Some(handle) = self.runs.get_mut(&lane) else {
            return false;
        };
        let RunStage::Asking { question, queued } = &mut handle.doing else {
            return false;
        };
        if question.ask_id != ask_id {
            return false;
        }
        // Bounded to one, so this cannot block and a second send cannot land.
        let _ = question.reply.try_send(decision);
        handle.doing = next_stage_after(question, queued);
        true
    }

    /// Stop the run `lane` holds.
    ///
    /// The question goes with it, queue and all: every one behind it belongs to
    /// the same stopped run, and the engine releases them the same way it
    /// releases the one on screen. Settled now for the same reason answering
    /// is — otherwise the buttons stay up with nothing to say the Stop landed.
    pub(in crate::workspace) fn cancel(&mut self, lane: LaneRef) {
        let Some(handle) = self.runs.get_mut(&lane) else {
            return;
        };
        handle.cancel.cancel();
        if let RunStage::Asking { question, .. } = &handle.doing {
            handle.doing = RunStage::Node {
                id: question.node.clone(),
                attempt: question.attempt,
            };
        }
    }
}

impl RunHandle {
    /// What this run has made of the flow it is of, when it can name one.
    fn colouring(&self) -> Option<(PathBuf, RunColouring)> {
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
}

/// The stage an event puts a run in, or `None` when it describes nothing a
/// person watches.
fn stage_for(event: &FlowEvent) -> Option<RunStage> {
    Some(match event {
        FlowEvent::NodeStarted { node, attempt } => RunStage::Node {
            id: node.clone(),
            attempt: *attempt,
        },
        FlowEvent::FixStarted { gate, .. } => RunStage::Fixing { gate: gate.clone() },
        // Both mean the same thing to a watcher: the gate is coming back and the
        // members about to be re-derived have not started either.
        FlowEvent::FixEnded { gate, .. } | FlowEvent::Rerunning { gate, .. } => {
            RunStage::Rederiving { gate: gate.clone() }
        }
        // `RunStarted` leaves `Starting`; the passes and failures are already
        // described by whichever node starts next.
        _ => return None,
    })
}

/// The next one takes its place if there is one — answering must not hide a
/// question that is still waiting.
fn next_stage_after(
    answered: &std::sync::Arc<ParkedAsk>,
    queued: &mut std::collections::VecDeque<std::sync::Arc<ParkedAsk>>,
) -> RunStage {
    match queued.pop_front() {
        Some(next) => RunStage::Asking {
            question: next,
            queued: std::mem::take(queued),
        },
        None => RunStage::Node {
            id: answered.node.clone(),
            attempt: answered.attempt,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(id: u64) -> LaneRef {
        LaneRef {
            project: 1,
            lane: id,
        }
    }

    /// A run with nothing behind it. `Resumed` because these tests are about
    /// the queue and the stage, neither of which reads the flow file.
    fn seeded_run(runs: &mut FlowRuns, at: LaneRef, run_dir: &str) {
        let run_dir = PathBuf::from(run_dir);
        runs.insert(
            at,
            RunHandle::seeded(
                run_dir.clone(),
                FlowSource::Resumed { run_dir },
                RunStage::Starting,
            ),
        );
    }

    /// A question and the receiver an answer would arrive on.
    fn ask(
        id: u64,
        node: &str,
    ) -> (
        ParkedAsk,
        smol::channel::Receiver<daruda_acp::PermissionDecision>,
    ) {
        let (reply, rx) = smol::channel::bounded(1);
        (
            ParkedAsk {
                ask_id: id,
                node: node.to_string(),
                attempt: 1,
                tool: "write".into(),
                detail: None,
                options: Vec::new(),
                reply,
            },
            rx,
        )
    }

    fn asking(runs: &FlowRuns, at: LaneRef) -> Option<(u64, usize)> {
        match &runs.iter().find(|(lane, _)| *lane == at)?.1.doing {
            RunStage::Asking { question, queued } => Some((question.ask_id, queued.len())),
            _ => None,
        }
    }

    /// The trap: a second question replacing the first would leave that first
    /// node parked on a reply nobody can send, and the run would wait for ever
    /// on a question no longer on screen.
    #[test]
    fn a_second_question_waits_behind_the_first() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let (first, _rx1) = ask(1, "design");
        let (second, _rx2) = ask(2, "build");

        assert_eq!(
            runs.park_ask(here, Path::new("/run"), first),
            Parked::Showing
        );
        assert_eq!(
            runs.park_ask(here, Path::new("/run"), second),
            Parked::Queued
        );
        assert_eq!(asking(&runs, here), Some((1, 1)), "the first is still up");
    }

    #[test]
    fn a_question_for_another_run_is_not_taken() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let (stale, _rx) = ask(1, "design");
        assert_eq!(
            runs.park_ask(here, Path::new("/other-run"), stale),
            Parked::Elsewhere
        );
        assert_eq!(asking(&runs, here), None);
    }

    /// Answering sends the decision and puts the next one up — hiding a
    /// question that is still waiting would park its node for ever.
    #[test]
    fn answering_replies_and_promotes_the_next() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let (first, rx1) = ask(1, "design");
        let (second, _rx2) = ask(2, "build");
        runs.park_ask(here, Path::new("/run"), first);
        runs.park_ask(here, Path::new("/run"), second);

        assert!(runs.answer_ask(here, 1, daruda_acp::PermissionDecision::Cancelled));
        assert_eq!(
            rx1.try_recv(),
            Ok(daruda_acp::PermissionDecision::Cancelled)
        );
        assert_eq!(asking(&runs, here), Some((2, 0)), "the next one takes over");
    }

    /// A surface can still be painted with a question already answered, and a
    /// click on that frame must do nothing rather than answer the *next* one.
    #[test]
    fn an_answer_quoting_the_wrong_question_does_nothing() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let (first, _rx) = ask(1, "design");
        runs.park_ask(here, Path::new("/run"), first);

        assert!(!runs.answer_ask(here, 99, daruda_acp::PermissionDecision::Cancelled));
        assert_eq!(asking(&runs, here), Some((1, 0)), "still waiting");
    }

    /// Stopping takes the whole queue with it: every question behind the one on
    /// screen belongs to the same stopped run.
    #[test]
    fn stopping_settles_the_question_and_its_queue() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let (first, _rx1) = ask(1, "design");
        let (second, _rx2) = ask(2, "build");
        runs.park_ask(here, Path::new("/run"), first);
        runs.park_ask(here, Path::new("/run"), second);

        runs.cancel(here);
        assert_eq!(asking(&runs, here), None, "nothing is being asked now");
    }

    /// Only the first move off `Starting` reports it: that is the one moment
    /// retention has run, so the history read from disk is stale exactly then.
    #[test]
    fn leaving_setup_is_reported_once() {
        let mut runs = FlowRuns::default();
        let here = lane(1);
        seeded_run(&mut runs, here, "/run");
        let started = |node: &str| FlowEvent::NodeStarted {
            node: node.to_string(),
            attempt: 1,
        };
        assert_eq!(
            runs.advance_stage(here, &started("design")),
            Advanced::Moved { left_setup: true }
        );
        assert_eq!(
            runs.advance_stage(here, &started("build")),
            Advanced::Moved { left_setup: false }
        );
        assert_eq!(
            runs.advance_stage(here, &started("build")),
            Advanced::Nothing,
            "already saying it"
        );
        assert_eq!(
            runs.advance_stage(lane(2), &started("design")),
            Advanced::Nothing,
            "and no run there at all"
        );
    }

    #[test]
    fn a_run_id_differs_from_the_last() {
        let mut runs = FlowRuns::default();
        assert_ne!(runs.next_run_id(), runs.next_run_id());
    }
}
