//! The flow runs this process is running, owned by [`RunsHost`] rather than
//! by the window that started them: closing that window leaves a run going,
//! and any window holding its worktree can still watch, answer and stop it.
//!
//! A run is keyed by its worktree, resolved, the unit its lock admits one run
//! to. A window asks with its own spelling of a lane's folder — a symlinked
//! prefix, a folder opened twice under two names — and that spelling is
//! resolved once and remembered, never per frame. Each run has one pump, which folds its notices and questions into the
//! [`RunView`] every window reads, and tells windows through
//! [`super::RunsEffects`] after the state changed — never from inside it.

mod colour;
mod host;
mod metadata;
#[cfg(any(test, feature = "screenshot"))]
use metadata::flow_metadata;
mod pump;
mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use daruda_flow::event::{RunEnd, RunNotice};
use daruda_flow::lock::CanonicalTree;
use daruda_flow::request::RunRequest;
use daruda_flow::runner::{CancelToken, OverageStop, PendingAsk, Runners};

#[cfg(test)]
pub(crate) use colour::apply_run_event;
pub(crate) use colour::{NodeRunState, NodeRunStates, RunColouring};
#[cfg(test)]
use pump::pump;
pub(crate) use view::{AnswerRefused, FlowSource, Lull, ParkedAsk, RunView};

/// A run in flight.
pub(crate) struct FlowRun {
    view: RunView,
    snapshot: crate::runs::result::RunSnapshot,
    pointers: crate::runs::result::Pointers,
    started: std::time::Instant,
    cancel: CancelToken,
    /// Read when the run ends, to say why it was canceled.
    overage: OverageStop,
    /// Who submitted this episode, start or resume: the one owed its
    /// answer. The run's first starter is provenance, kept in its log.
    submitted_by: daruda_flow::request::StartedBy,
    solicited: bool,
    /// Disconnects when the run's thread returns, so a quit can block on
    /// it with a deadline. Seeded runs have no engine behind them.
    finished: Option<std::sync::mpsc::Receiver<std::convert::Infallible>>,
}

impl FlowRun {
    /// Whether the run's thread returned by `until`.
    fn wait_ended(&self, until: std::time::Instant) -> bool {
        let Some(finished) = &self.finished else {
            return true;
        };
        let left = until.saturating_duration_since(std::time::Instant::now());
        match finished.recv_timeout(left) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
        }
    }

    pub(super) fn result(&self) -> crate::runs::result::RunSnapshot {
        let mut snapshot = self.snapshot.clone();
        let (node, attempt) = self
            .view
            .running()
            .first()
            .map(|(node, state)| {
                (
                    Some(node.to_string()),
                    match state {
                        NodeRunState::Running { attempt } => Some(*attempt),
                        _ => None,
                    },
                )
            })
            .unwrap_or_default();
        snapshot.state = crate::runs::result::RunState::Running {
            progress: crate::runs::result::Progress {
                node,
                attempt,
                elapsed_secs: self.started.elapsed().as_secs(),
                ..Default::default()
            },
            waiting_on: self.view.waiting_on(),
        };
        snapshot
    }

    pub(crate) fn view(&self) -> &RunView {
        &self.view
    }

    /// Whether Stop has been pressed; the run is listed until it ends.
    #[cfg(test)]
    pub(crate) fn is_stopping(&self) -> bool {
        self.cancel.is_canceled()
    }
}

/// Every run this process holds, by the worktree it runs in.
#[derive(Default)]
pub(crate) struct FlowRuns {
    runs: HashMap<CanonicalTree, FlowRun>,
    /// What each spelling a window asked with resolves to, `None` for one
    /// that would not resolve. Lanes are looked up every frame and resolving
    /// is a filesystem call, so it is made once per spelling. Forgotten
    /// whenever a run starts or ends, which bounds how long an entry can
    /// outlive the folder it described.
    spellings: RefCell<HashMap<PathBuf, Option<CanonicalTree>>>,
    /// Distinguishes two runs started in the same millisecond.
    counter: u32,
    /// Signalled with every change told to windows, so a test can wait on a
    /// real run's progress instead of sleeping.
    #[cfg(test)]
    changed: Vec<smol::channel::Sender<()>>,
}

impl FlowRuns {
    pub(super) fn by_id(&self, id: &str) -> Option<&FlowRun> {
        self.runs.values().find(|run| {
            run.view
                .run_dir()
                .file_name()
                .and_then(|name| name.to_str())
                == Some(id)
        })
    }

    /// The key `spelling` names, as the lock would resolve it. `None` when
    /// nothing runs, without touching the filesystem.
    fn key_for(&self, spelling: &Path) -> Option<CanonicalTree> {
        if self.runs.is_empty() {
            return None;
        }
        if let Some((key, _)) = self.runs.get_key_value(spelling) {
            return Some(key.clone());
        }
        self.resolve(spelling)
    }

    /// `spelling` resolved, remembered. Also asked after a run ended and
    /// left the map, by the windows working out which of their lanes it was.
    fn resolve(&self, spelling: &Path) -> Option<CanonicalTree> {
        // Read through `&App` every frame, hence the cell. The borrow cannot
        // re-enter: it ends in this statement, and resolving only touches
        // the filesystem, never the app.
        self.spellings
            .borrow_mut()
            .entry(spelling.to_path_buf())
            .or_insert_with(|| CanonicalTree::resolve(spelling).ok())
            .clone()
    }

    fn get(&self, spelling: &Path) -> Option<&FlowRun> {
        self.runs.get(&self.key_for(spelling)?)
    }

    #[cfg(test)]
    fn get_mut(&mut self, spelling: &Path) -> Option<&mut FlowRun> {
        let key = self.key_for(spelling)?;
        self.runs.get_mut(&key)
    }

    /// Hold `run` in `tree`, unless a run is already there.
    fn insert(&mut self, tree: CanonicalTree, run: FlowRun) -> Result<(), AlreadyRunning> {
        if self.runs.contains_key(&tree) {
            return Err(AlreadyRunning);
        }
        self.runs.insert(tree, run);
        self.spellings.get_mut().clear();
        Ok(())
    }

    /// Tell every run the app is quitting. Their threads end their
    /// processes and drain their logs on their own; [`Self::wait_ended`]
    /// waits for that.
    fn shut_down(&self) {
        for run in self.runs.values() {
            run.cancel.shut_down();
        }
    }

    /// Wait, until `until`, for every run's thread to return. `false` when
    /// one was still going; the resume's orphan clean-up is the backstop for
    /// what it leaves.
    fn wait_ended(&self, until: std::time::Instant) -> bool {
        self.runs.values().all(|run| run.wait_ended(until))
    }

    fn remove(&mut self, tree: &Path) -> Option<FlowRun> {
        let run = self.runs.remove(tree)?;
        self.spellings.get_mut().clear();
        Some(run)
    }
}

/// A run was asked for in a worktree that already has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AlreadyRunning;

/// Everything a run needs to start. The runners are built on the run's own
/// thread, from the request, by the host that decides what they are given.
pub(crate) struct FlowStart {
    /// The worktree resolved, which the run is keyed by: the tree its lock
    /// is named after.
    pub tree: CanonicalTree,
    pub request: RunRequest,
    pub source: FlowSource,
    pub runners: Box<dyn FnOnce(&RunRequest) -> Runners + Send>,
    pub overage: OverageStop,
    /// Who submitted this episode; see [`FlowRun::submitted_by`].
    pub submitted_by: daruda_flow::request::StartedBy,
    pub solicited: bool,
    pub events: smol::channel::Receiver<RunNotice>,
    pub asks: smol::channel::Receiver<PendingAsk>,
}

/// What changed about the run in `worktree`, for the windows holding it.
#[derive(Clone, Debug)]
pub(crate) struct FlowChange {
    pub worktree: CanonicalTree,
    pub what: FlowChanged,
}

#[derive(Clone, Debug)]
pub(crate) enum FlowChanged {
    /// Started, answered or stopped: only a repaint.
    Repainted,
    /// It moved along; `left_setup` the first time a node started.
    Progressed { left_setup: bool },
    /// A question arrived. `alone` when none of the run's was open, which is
    /// when one is brought forward.
    Asked {
        run_dir: PathBuf,
        ask_id: u64,
        alone: bool,
        submitted_by: daruda_flow::request::StartedBy,
        solicited: bool,
    },
    AskClosed {
        run_dir: PathBuf,
        ask_id: u64,
        outcome: FlowAskOutcome,
    },
    /// It ended and is no longer held.
    Ended(Box<FlowEnded>),
}

#[derive(Clone, Debug)]
pub(crate) enum FlowAskOutcome {
    Answered(daruda_acp::PermissionDecision),
    Withdrawn,
}

/// A run that ended, with what its windows say about it.
#[derive(Clone, Debug)]
pub(crate) struct FlowEnded {
    pub end: RunEnd,
    pub run_dir: PathBuf,
    /// The billed report that stopped it, if one did.
    pub overage: Option<daruda_acp::RateLimitView>,
    pub owes_remote_answer: bool,
    /// The run's last word on its flow's nodes, reconciled with its log.
    pub colouring: Option<(PathBuf, RunColouring)>,
}

#[cfg(test)]
mod tests;
