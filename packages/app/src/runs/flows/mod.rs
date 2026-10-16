//! The flow runs this process is running, owned by [`RunsHost`] rather than
//! by the window that started them: closing that window leaves a run going,
//! and any window holding its worktree can still watch, answer and stop it
//! (F6).
//!
//! A run is keyed by its worktree, the unit its lock admits one run to. Each
//! run has one pump, which folds its notices and questions into the
//! [`RunView`] every window reads, and tells windows through
//! [`super::RunsEffects`] after the state changed — never from inside it.

mod colour;
mod view;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd, RunNotice};
use daruda_flow::request::RunRequest;
use daruda_flow::runner::{CancelToken, OverageStop, PendingAsk, Runners};
use gpui::{App, AsyncApp, BorrowAppContext as _};

#[cfg(test)]
pub(crate) use colour::apply_run_event;
pub(crate) use colour::{NodeRunState, NodeRunStates, RunColouring};
use view::{Advanced, Parked};
pub(crate) use view::{AnswerRefused, FlowSource, Lull, ParkedAsk, RunView};

use super::RunsHost;

/// What a run owes the caller that asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Caller {
    /// Started at the desktop, which shows its own toast.
    Desktop,
    /// Asked for from a phone, which has no screen to read a toast on.
    Remote,
}

/// A run in flight.
pub(crate) struct FlowRun {
    view: RunView,
    cancel: CancelToken,
    /// Read when the run ends, to say why it was canceled.
    overage: OverageStop,
    caller: Caller,
    /// Seeded runs have no engine behind them.
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl FlowRun {
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
    runs: HashMap<PathBuf, FlowRun>,
    /// Distinguishes two runs started in the same millisecond.
    counter: u32,
}

/// Everything a run needs to start. The runners are built on the run's own
/// thread, from the request, by the host that decides what they are given.
pub(crate) struct FlowStart {
    pub request: RunRequest,
    pub source: FlowSource,
    pub runners: Box<dyn FnOnce(&RunRequest) -> Runners + Send>,
    pub overage: OverageStop,
    pub events: smol::channel::Receiver<RunNotice>,
    pub asks: smol::channel::Receiver<PendingAsk>,
}

/// What changed about the run in `worktree`, for the windows holding it.
#[derive(Clone, Debug)]
pub(crate) struct FlowChange {
    pub worktree: PathBuf,
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
    /// The run in `worktree`, if this process holds one.
    pub(crate) fn flow_run<'a>(worktree: &Path, cx: &'a App) -> Option<&'a FlowRun> {
        cx.try_global::<RunsHost>()?.flows.runs.get(worktree)
    }

    /// The next run id. Wrapping because it only has to differ from the
    /// last one, not count anything.
    pub(crate) fn next_flow_run_id(cx: &mut App) -> u32 {
        cx.update_global::<RunsHost, _>(|host, _| {
            host.flows.counter = host.flows.counter.wrapping_add(1);
            host.flows.counter
        })
    }

    /// Start a run: its thread, its pump, and its entry, together.
    pub(crate) fn start_flow(start: FlowStart, cx: &mut App) {
        let FlowStart {
            request,
            source,
            runners,
            overage,
            events,
            asks,
        } = start;
        let worktree = request.cwd.clone();
        let run_dir = request.run_dir.clone();
        let nodes_at_start = request
            .loaded
            .flow()
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect();
        let cancel = CancelToken::default();
        let thread = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                let runners = runners(&request);
                let report = daruda_flow::schedule::execute(&request, &runners, &cancel);
                // A run refused before it started records nothing and only
                // returns; this is its end. A run that recorded its own end
                // has already been retired by then, so this is never read.
                if let Some(events) = &request.events {
                    // SILENT-OK: a closed channel means the pump is gone.
                    let _ = events.try_send(RunNotice::Unrecorded(RunEnd::from(&report.outcome)));
                }
            })
        };
        let run = FlowRun {
            view: RunView::new(run_dir.clone(), source, nodes_at_start),
            cancel,
            overage,
            caller: Caller::Desktop,
            _thread: Some(thread),
        };
        cx.update_global::<RunsHost, _>(|host, _| host.flows.runs.insert(worktree.clone(), run));
        Self::tell_windows(worktree.clone(), FlowChanged::Repainted, cx);
        cx.spawn(async move |cx| pump(worktree, run_dir, events, asks, cx).await)
            .detach();
    }

    /// Owe `worktree`'s run's outcome to a remote caller. `false` when no
    /// run is there, which is also how a caller learns its submission
    /// produced none.
    #[must_use = "a submission that produced no run has to be answered"]
    pub(crate) fn owe_flow_outcome_remotely(worktree: &Path, cx: &mut App) -> bool {
        cx.update_global::<RunsHost, _>(|host, _| match host.flows.runs.get_mut(worktree) {
            Some(run) => {
                run.caller = Caller::Remote;
                true
            }
            None => false,
        })
    }

    /// Stop the run in `worktree`. Its questions go with it: they belong to
    /// a stopped run, and buttons left up would read as a Stop that did not
    /// land. The entry stays until the run says it ended.
    pub(crate) fn stop_flow(worktree: &Path, cx: &mut App) -> bool {
        let stopped = cx.update_global::<RunsHost, _>(|host, _| {
            let Some(run) = host.flows.runs.get_mut(worktree) else {
                return false;
            };
            run.cancel.cancel();
            run.view.withdraw_questions();
            true
        });
        if stopped {
            Self::tell_windows(worktree.to_path_buf(), FlowChanged::Repainted, cx);
        }
        stopped
    }

    /// Answer question `ask_id` of the run in `worktree`. Taken down at
    /// once, so the buttons do not stay up while the agent works.
    pub(crate) fn answer_flow_ask(
        worktree: &Path,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
        cx: &mut App,
    ) -> Result<(), AnswerRefused> {
        let answered =
            cx.update_global::<RunsHost, _>(|host, _| match host.flows.runs.get_mut(worktree) {
                Some(run) => run.view.answer(ask_id, decision),
                None => Err(AnswerRefused::NotOpen),
            });
        if answered != Err(AnswerRefused::NotOpen) {
            Self::tell_windows(worktree.to_path_buf(), FlowChanged::Repainted, cx);
        }
        answered
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
            let run = host
                .flows
                .runs
                .get_mut(worktree)
                .filter(|run| run.view.run_dir() == run_dir)?;
            Some(run.view.park(ParkedAsk::new(pending)))
        });
        if let Some(parked) = parked {
            let what = FlowChanged::Asked {
                ask_id,
                alone: parked == Parked::Alone,
            };
            Self::tell_windows(worktree.to_path_buf(), what, cx);
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
            let runs = &mut host.flows.runs;
            if runs.get(worktree)?.view.run_dir() != run_dir {
                return None;
            }
            if let FlowEvent::RunEnded { end } = event {
                let mut run = runs.remove(worktree)?;
                run.view.advance(event);
                if let Some(states) = from_log {
                    run.view.reconcile(states);
                }
                return Some(FlowChanged::Ended(Box::new(FlowEnded {
                    end: end.clone(),
                    run_dir: run_dir.to_path_buf(),
                    overage: run.overage.tripped().cloned(),
                    owes_remote_answer: run.caller == Caller::Remote,
                    colouring: run.view.colouring(),
                })));
            }
            let run = runs.get_mut(worktree)?;
            let advanced = run.view.advance(event);
            let reconciled = from_log.is_some_and(|states| run.view.reconcile(states));
            match (advanced, reconciled) {
                (Advanced::Moved { left_setup }, _) => Some(FlowChanged::Progressed { left_setup }),
                (Advanced::Nothing, true) => Some(FlowChanged::Progressed { left_setup: false }),
                (Advanced::Nothing, false) => None,
            }
        });
        if let Some(what) = what {
            Self::tell_windows(worktree.to_path_buf(), what, cx);
        }
    }

    /// Tell the windows, after this update: a window asking from inside its
    /// own update would otherwise be re-entered.
    fn tell_windows(worktree: PathBuf, what: FlowChanged, cx: &mut App) {
        let Some(effects) = cx.global::<RunsHost>().effects.clone() else {
            return;
        };
        let change = FlowChange { worktree, what };
        cx.defer(move |cx| effects.flow_changed(&change, cx));
    }

    /// Put a run in `worktree` with no engine behind it, for tests and
    /// `--screenshot`.
    #[cfg(any(test, feature = "screenshot"))]
    pub(crate) fn seed_flow_run(worktree: PathBuf, view: RunView, cx: &mut App) {
        let run = FlowRun {
            view,
            cancel: CancelToken::default(),
            overage: OverageStop::default(),
            caller: Caller::Desktop,
            _thread: None,
        };
        cx.update_global::<RunsHost, _>(|host, _| host.flows.runs.insert(worktree.clone(), run));
        Self::tell_windows(worktree, FlowChanged::Repainted, cx);
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
    }

    /// Stop `worktree`'s run for `limit` as its engine would, without one.
    #[cfg(test)]
    pub(crate) fn trip_flow_overage_for_test(
        worktree: &Path,
        limit: &daruda_acp::RateLimitView,
        cx: &mut App,
    ) {
        cx.update_global::<RunsHost, _>(|host, _| {
            let run = host.flows.runs.get_mut(worktree).expect("a seeded run");
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
    worktree: PathBuf,
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
                cx.update(|cx| RunsHost::park_flow_ask(&worktree, &run_dir, pending, cx));
                continue;
            }
            Next::AsksDone => {
                asks = None;
                continue;
            }
            Next::Notice(notice) => notice.into_event(),
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
        cx.update(|cx| RunsHost::apply_flow_event(&worktree, &run_dir, &event, from_log, cx));
        if ended {
            break;
        }
    }
}

#[cfg(test)]
mod tests;
