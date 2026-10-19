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
mod metadata;
#[cfg(any(test, feature = "screenshot"))]
use metadata::flow_metadata;
mod view;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd, RunNotice};
use daruda_flow::lock::CanonicalTree;
use daruda_flow::request::RunRequest;
use daruda_flow::runner::{CancelToken, OverageStop, PendingAsk, Runners};
use gpui::{App, AsyncApp, BorrowAppContext as _};

#[cfg(test)]
pub(crate) use colour::apply_run_event;
pub(crate) use colour::{NodeRunState, NodeRunStates, RunColouring};
use view::{Advanced, Parked};
pub(crate) use view::{AnswerRefused, FlowSource, Lull, ParkedAsk, RunView};

use super::RunsHost;

/// A run in flight.
pub(crate) struct FlowRun {
    view: RunView,
    snapshot: super::result::RunSnapshot,
    pointers: super::result::Pointers,
    started: std::time::Instant,
    cancel: CancelToken,
    /// Read when the run ends, to say why it was canceled.
    overage: OverageStop,
    /// Who submitted this episode, start or resume: the one owed its
    /// answer. The run's first starter is provenance, kept in its log.
    submitted_by: daruda_flow::request::StartedBy,
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

    pub(super) fn result(&self) -> super::result::RunSnapshot {
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
        snapshot.state = super::result::RunState::Running {
            progress: super::result::Progress {
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
    Asked { ask_id: u64, alone: bool },
    /// It ended and is no longer held.
    Ended(Box<FlowEnded>),
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

impl RunsHost {
    /// The run in `worktree`, however it is spelled, if this process holds
    /// one.
    pub(crate) fn flow_run<'a>(worktree: &Path, cx: &'a App) -> Option<&'a FlowRun> {
        cx.try_global::<RunsHost>()?.flows.get(worktree)
    }

    /// Whether `spelling` names `tree`, the key a run's changes are told by.
    pub(crate) fn names_flow_tree(spelling: &Path, tree: &CanonicalTree, cx: &App) -> bool {
        spelling == tree.as_path()
            || cx
                .try_global::<RunsHost>()
                .and_then(|host| host.flows.resolve(spelling))
                .is_some_and(|key| &key == tree)
    }

    /// The next run id. Wrapping because it only has to differ from the
    /// last one, not count anything.
    pub(crate) fn next_flow_run_id(cx: &mut App) -> u32 {
        cx.update_global::<RunsHost, _>(|host, _| {
            host.flows.counter = host.flows.counter.wrapping_add(1);
            host.flows.counter
        })
    }

    /// Start a run: its thread, its pump, and its entry, together, returning
    /// the run id its results are keyed by. Refused, with nothing started,
    /// when the worktree already has a run: replacing its entry would leave
    /// that run with no stop switch anywhere.
    pub(crate) fn start_flow(start: FlowStart, cx: &mut App) -> Result<String, AlreadyRunning> {
        let FlowStart {
            tree,
            request,
            source,
            runners,
            overage,
            submitted_by,
            events,
            asks,
        } = start;
        if cx.global::<RunsHost>().flows.runs.contains_key(&tree) {
            return Err(AlreadyRunning);
        }
        let run_dir = request.run_dir.clone();
        let location = super::result::FlowRunLocation {
            run_dir: run_dir.clone(),
            lock_dir: Some(request.lock_dir.clone()),
        };
        let nodes_at_start = request
            .loaded
            .flow()
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let (snapshot, pointers) = metadata::for_request(&request, &source);
        let cancel = CancelToken::default();
        let (finished_tx, finished) = std::sync::mpsc::channel();
        {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                // Dropped when this returns, panicking or not: the signal
                // `wait_ended` blocks on.
                let _finished = finished_tx;
                let runners = runners(&request);
                let report = daruda_flow::schedule::execute(&request, &runners, &cancel);
                // A run refused before it started records nothing and only
                // returns; this is its end. A run that recorded its own end
                // has already been retired by then, so this is never read.
                if let Some(events) = &request.events {
                    // SILENT-OK: a closed channel means the pump is gone.
                    let _ = events.try_send(RunNotice::Unrecorded(RunEnd::from(&report.outcome)));
                }
            });
        }
        let run = FlowRun {
            snapshot,
            pointers,
            started: std::time::Instant::now(),
            view: RunView::new(run_dir.clone(), source, nodes_at_start),
            cancel,
            overage,
            submitted_by,
            finished: Some(finished),
        };
        cx.update_global::<RunsHost, _>(|host, _| {
            host.watch.begin(&run.snapshot.key);
            host.flows.insert(tree.clone(), run)?;
            host.watch.locate(location);
            Ok(())
        })?;
        Self::tell_windows(tree.clone(), FlowChanged::Repainted, cx);
        let run_id = super::result::flow_run_id(&run_dir);
        cx.spawn(async move |cx| pump(tree, run_dir, events, asks, cx).await)
            .detach();
        Ok(run_id)
    }

    /// The app is quitting: tell every run to end what it started and stay
    /// resumable. Returns at once; [`Self::wait_flows_ended`] waits.
    pub(crate) fn shut_down_flows(cx: &App) {
        cx.global::<RunsHost>().flows.shut_down();
    }

    /// Wait, until `until`, for every run told to shut down to end. `false`
    /// when one was still going.
    pub(crate) fn wait_flows_ended(until: std::time::Instant, cx: &App) -> bool {
        cx.global::<RunsHost>().flows.wait_ended(until)
    }

    /// Stop the run in `worktree`. Its questions go with it: they belong to
    /// a stopped run, and buttons left up would read as a Stop that did not
    /// land. The entry stays until the run says it ended.
    pub(crate) fn stop_flow(worktree: &Path, cx: &mut App) -> bool {
        let stopped = cx.update_global::<RunsHost, _>(|host, _| {
            let key = host.flows.key_for(worktree)?;
            let run = host.flows.runs.get_mut(&key)?;
            run.cancel.cancel();
            run.view.withdraw_questions();
            host.watch.observe_waiting(run.result());
            Some(key)
        });
        match stopped {
            Some(key) => {
                Self::tell_windows(key, FlowChanged::Repainted, cx);
                true
            }
            None => false,
        }
    }

    /// Answer question `ask_id` of the run in `worktree`. Taken down at
    /// once, so the buttons do not stay up while the agent works.
    pub(crate) fn answer_flow_ask(
        worktree: &Path,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
        cx: &mut App,
    ) -> Result<(), AnswerRefused> {
        let answered = cx.update_global::<RunsHost, _>(|host, _| {
            let Some(key) = host.flows.key_for(worktree) else {
                return (None, Err(AnswerRefused::NotOpen));
            };
            match host.flows.runs.get_mut(&key) {
                Some(run) => {
                    let answer = run.view.answer(ask_id, decision);
                    host.watch.observe_waiting(run.result());
                    (Some(key), answer)
                }
                None => (None, Err(AnswerRefused::NotOpen)),
            }
        });
        if let (Some(key), answer) = &answered
            && *answer != Err(AnswerRefused::NotOpen)
        {
            Self::tell_windows(key.clone(), FlowChanged::Repainted, cx);
        }
        answered.1
    }

    /// Hold a question until somebody answers it.
    ///
    /// `run_dir` names the run that asked. A channel drains before it closes,
    /// so a question the previous run queued while stopping can arrive after
    /// the next run took the worktree; dropping it releases that old engine,
    /// the only thing still waiting on it.
    pub(crate) fn park_flow_ask(
        worktree: &Path,
        run_dir: &Path,
        pending: PendingAsk,
        cx: &mut App,
    ) {
        let ask_id = pending.ask_id;
        let parked = cx.update_global::<RunsHost, _>(|host, _| {
            let key = host.flows.key_for(worktree)?;
            let run = host
                .flows
                .runs
                .get_mut(&key)
                .filter(|run| run.view.run_dir() == run_dir && !run.cancel.is_canceled())?;
            // A closed question followed immediately by a new ask is a new episode.
            host.watch.observe_waiting(run.result());
            let parked = run.view.park(ParkedAsk::new(pending));
            host.watch.observe_waiting(run.result());
            Some((key, parked))
        });
        if let Some((key, parked)) = parked {
            let what = FlowChanged::Asked {
                ask_id,
                alone: parked == Parked::Alone,
            };
            Self::tell_windows(key, what, cx);
        }
    }

    fn withdraw_flow_asks(worktree: &Path, run_dir: &Path, cx: &mut App) {
        let changed = cx.update_global::<RunsHost, _>(|host, _| {
            let key = host.flows.key_for(worktree)?;
            let run = host
                .flows
                .runs
                .get_mut(&key)
                .filter(|run| run.view.run_dir() == run_dir)?;
            run.view.withdraw_questions();
            host.watch.observe_waiting(run.result());
            Some(key)
        });
        if let Some(key) = changed {
            Self::tell_windows(key, FlowChanged::Repainted, cx);
        }
    }

    /// Fold one event of the run in `run_dir` into its view; `from_log` is
    /// the log's word on every node, read after the event was recorded.
    pub(crate) fn apply_flow_event(
        worktree: &Path,
        run_dir: &Path,
        event: &FlowEvent,
        from_log: Option<NodeRunStates>,
        cx: &mut App,
    ) {
        let what = cx.update_global::<RunsHost, _>(|host, _| {
            let key = host.flows.key_for(worktree)?;
            if host.flows.runs.get(&key)?.view.run_dir() != run_dir {
                return None;
            }
            if let FlowEvent::RunEnded { end } = event {
                let mut run = host.flows.remove(key.as_path())?;
                run.view.advance(event);
                if let Some(states) = from_log {
                    run.view.reconcile(states);
                }
                run.view.withdraw_questions();
                host.watch.observe_waiting(run.result());
                // Held, not announced: the pump reads the run's report first,
                // so no waiter or notice takes this end without its text.
                if !matches!(end, RunEnd::ShutDown) {
                    host.watch.hold_end(super::result::flow_ended(
                        run.snapshot.clone(),
                        end,
                        run.pointers.clone(),
                    ));
                }
                let ended = FlowChanged::Ended(Box::new(FlowEnded {
                    end: end.clone(),
                    run_dir: run_dir.to_path_buf(),
                    overage: run.overage.tripped().cloned(),
                    owes_remote_answer: matches!(
                        run.submitted_by,
                        daruda_flow::request::StartedBy::Phone
                            | daruda_flow::request::StartedBy::Orchestrator
                    ),
                    colouring: run.view.colouring(),
                }));
                return Some((key, ended));
            }
            let run = host.flows.runs.get_mut(&key)?;
            let advanced = run.view.advance(event);
            let reconciled = from_log.is_some_and(|states| run.view.reconcile(states));
            host.watch.observe_waiting(run.result());
            let what = match (advanced, reconciled) {
                (Advanced::Moved { left_setup }, _) => FlowChanged::Progressed { left_setup },
                (Advanced::Nothing, true) => FlowChanged::Progressed { left_setup: false },
                (Advanced::Nothing, false) => return None,
            };
            Some((key, what))
        });
        if let Some((key, what)) = what {
            Self::tell_windows(key, what, cx);
        }
    }

    /// Tell the windows, after this update: a window asking from inside its
    /// own update would otherwise be re-entered.
    fn tell_windows(worktree: CanonicalTree, what: FlowChanged, cx: &mut App) {
        let Some(effects) = cx.global::<RunsHost>().effects.clone() else {
            return;
        };
        let change = FlowChange { worktree, what };
        cx.defer(move |cx| effects.flow_changed(&change, cx));
    }

    /// Put a run in `worktree` with no engine behind it, for tests and
    /// `--screenshot`. Keyed as a started run is, by the folder resolved.
    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn seed_flow_run(worktree: PathBuf, view: RunView, cx: &mut App) {
        Self::seed_flow_run_submitted_by(
            worktree,
            view,
            daruda_flow::request::StartedBy::Desktop,
            cx,
        );
    }

    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn seed_flow_run_submitted_by(
        worktree: PathBuf,
        view: RunView,
        submitted_by: daruda_flow::request::StartedBy,
        cx: &mut App,
    ) {
        let tree = CanonicalTree::resolve(&worktree)
            .unwrap_or_else(|_| CanonicalTree::unchecked(worktree));
        let (snapshot, pointers) = flow_metadata(view.run_dir(), view.source(), None);
        let run = FlowRun {
            snapshot,
            pointers,
            started: std::time::Instant::now(),
            view,
            cancel: CancelToken::default(),
            overage: OverageStop::default(),
            submitted_by,
            finished: None,
        };
        // A seed lands on whatever a test left there: it stands in for
        // state, not for a start.
        cx.update_global::<RunsHost, _>(|host, _| {
            host.flows.remove(tree.as_path());
            host.flows.insert(tree.clone(), run)
        })
        .expect("the worktree was just cleared");
        Self::tell_windows(tree, FlowChanged::Repainted, cx);
    }

    /// Drive one event through the path the pump takes, log read included.
    #[cfg(test)]
    pub(crate) fn apply_flow_event_for_test(worktree: &Path, event: &FlowEvent, cx: &mut App) {
        let Some(run_dir) =
            Self::flow_run(worktree, cx).map(|run| run.view.run_dir().to_path_buf())
        else {
            return;
        };
        let from_log = reconciles_with_log(event)
            .then(|| states_in_log(&run_dir))
            .flatten();
        Self::apply_flow_event(worktree, &run_dir, event, from_log, cx);
        if let Some((snapshot, held)) = held_end(&run_dir, event, cx) {
            Self::release_run_end(held, super::result_io::enrich(snapshot), cx);
        }
    }

    /// Stop `worktree`'s run for `limit` as its engine would, without one.
    #[cfg(test)]
    pub(crate) fn trip_flow_overage_for_test(
        worktree: &Path,
        limit: &daruda_acp::RateLimitView,
        cx: &mut App,
    ) {
        cx.update_global::<RunsHost, _>(|host, _| {
            let run = host.flows.get_mut(worktree).expect("a seeded run");
            run.overage = OverageStop::new(std::sync::Arc::new(true.into()));
            assert!(run.overage.trip(limit));
        });
    }
}

/// The two moments a run's view is checked against its log: its start,
/// which for a resume is all the earlier process did, and its end, which a
/// stopped attempt does not narrate.
fn reconciles_with_log(event: &FlowEvent) -> bool {
    matches!(
        event,
        FlowEvent::RunStarted { .. } | FlowEvent::RunEnded { .. }
    )
}

fn states_in_log(run_dir: &Path) -> Option<NodeRunStates> {
    daruda_flow::run_log::reader::state(run_dir).map(|state| colour::states_from_log(&state))
}

/// The end `event` held for the run in `run_dir`, with its release token,
/// while its report is still to be read.
fn held_end(
    run_dir: &Path,
    event: &FlowEvent,
    cx: &App,
) -> Option<(super::result::RunSnapshot, super::watch::HeldEnd)> {
    if !matches!(event, FlowEvent::RunEnded { end } if !matches!(end, RunEnd::ShutDown)) {
        return None;
    }
    cx.global::<RunsHost>()
        .watch
        .held_end(&super::result::RunKey::flow_of(run_dir))
}

/// What a run receives next.
enum Next {
    Notice(RunNotice),
    Ask(PendingAsk),
    /// The question channel closed; the engine asks nothing more.
    AsksDone,
    /// The narration closed without an end: the thread is gone.
    Silent,
}

/// The run's one pump: its notices and questions, in arrival order, until it
/// ends. App-owned, so it outlives every window.
async fn pump(
    worktree: CanonicalTree,
    run_dir: PathBuf,
    events: smol::channel::Receiver<RunNotice>,
    asks: smol::channel::Receiver<PendingAsk>,
    cx: &mut AsyncApp,
) {
    let mut asks = Some(asks);
    loop {
        let next = match &asks {
            Some(open) => {
                smol::future::or(
                    async { events.recv().await.map_or(Next::Silent, Next::Notice) },
                    async { open.recv().await.map_or(Next::AsksDone, Next::Ask) },
                )
                .await
            }
            None => events.recv().await.map_or(Next::Silent, Next::Notice),
        };
        let event = match next {
            Next::Ask(pending) => {
                cx.update(|cx| RunsHost::park_flow_ask(worktree.as_path(), &run_dir, pending, cx));
                continue;
            }
            Next::AsksDone => {
                asks = None;
                cx.update(|cx| RunsHost::withdraw_flow_asks(worktree.as_path(), &run_dir, cx));
                continue;
            }
            Next::Notice(notice) => {
                if let RunNotice::Recorded(record) = &notice {
                    let key = super::result::RunKey::flow_of(&run_dir);
                    cx.update(|cx| {
                        cx.update_global::<RunsHost, _>(|host, _| {
                            host.watch.record_flow_sequence(key, record.seq)
                        })
                    });
                }
                notice.into_event()
            }
            // The engine reports by recording, and the thread by returning;
            // a thread that did neither panicked, and the run still ends.
            Next::Silent => FlowEvent::RunEnded {
                end: RunEnd::Io {
                    site: daruda_flow::error::IoSite::Run,
                    doing: "running the flow".to_string(),
                    path: PathBuf::new(),
                    message: crate::surface::strings::flow::run_panicked(),
                },
            },
        };
        let from_log = if reconciles_with_log(&event) {
            let dir = run_dir.clone();
            cx.background_executor()
                .spawn(async move { states_in_log(&dir) })
                .await
        } else {
            None
        };
        let ended = matches!(event, FlowEvent::RunEnded { .. });
        let held = cx.update(|cx| {
            RunsHost::apply_flow_event(worktree.as_path(), &run_dir, &event, from_log, cx);
            held_end(&run_dir, &event, cx)
        });
        // Released whatever the read found: an unreadable report still ends
        // the run, with its process facts and no text.
        if let Some((snapshot, held)) = held {
            let enriched = cx
                .background_executor()
                .spawn(async move { super::result_io::enrich(snapshot) })
                .await;
            cx.update(|cx| RunsHost::release_run_end(held, enriched, cx));
        }
        if ended {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
