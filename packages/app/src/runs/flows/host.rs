//! What windows and the pump ask of the runs in flight: starting one, its
//! questions and answers, its events, and the end that retires it.

use std::path::Path;
#[cfg(any(test, feature = "screenshot"))]
use std::path::PathBuf;

use daruda_flow::event::{FlowEvent, RunEnd, RunNotice};
use daruda_flow::lock::CanonicalTree;
#[cfg(any(test, feature = "screenshot"))]
use daruda_flow::runner::OverageStop;
use daruda_flow::runner::{CancelToken, PendingAsk};
use gpui::{App, BorrowAppContext as _};

#[cfg(any(test, feature = "screenshot"))]
use super::flow_metadata;
use super::pump::pump;
#[cfg(test)]
use super::pump::{held_end, reconciles_with_log, states_in_log};
use super::view::{Advanced, Parked};
use super::{
    AlreadyRunning, AnswerRefused, FlowAskOutcome, FlowChange, FlowChanged, FlowEnded, FlowRun,
    FlowStart, NodeRunStates, ParkedAsk, RunView, metadata,
};
use crate::runs::RunsHost;

impl RunsHost {
    /// The run in `worktree`, however it is spelled, if this process holds
    /// one.
    pub(crate) fn flow_run<'a>(worktree: &Path, cx: &'a App) -> Option<&'a FlowRun> {
        cx.try_global::<RunsHost>()?.flows.get(worktree)
    }

    pub(crate) fn has_flow_asks(cx: &App) -> bool {
        cx.try_global::<RunsHost>().is_some_and(|host| {
            host.flows
                .runs
                .values()
                .any(|run| run.view.questions().any(|ask| !ask.is_closed()))
        })
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
            solicited,
            events,
            asks,
        } = start;
        if cx.global::<RunsHost>().flows.runs.contains_key(&tree) {
            return Err(AlreadyRunning);
        }
        let run_dir = request.run_dir.clone();
        let location = crate::runs::result::FlowRunLocation {
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
            solicited,
            finished: Some(finished),
        };
        cx.update_global::<RunsHost, _>(|host, _| {
            host.watch.begin_solicited(&run.snapshot.key, run.solicited);
            host.flows.insert(tree.clone(), run)?;
            host.watch.locate(location);
            Ok(())
        })?;
        Self::tell_windows(tree.clone(), FlowChanged::Repainted, cx);
        let run_id = crate::runs::result::flow_run_id(&run_dir);
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
            let run_dir = run.view.run_dir().to_path_buf();
            let questions = run.view.questions().map(ParkedAsk::ask_id).collect();
            run.cancel.cancel();
            run.view.withdraw_questions();
            host.watch.observe_waiting(run.result());
            Some((key, run_dir, questions))
        });
        match stopped {
            Some((key, run_dir, questions)) => {
                Self::close_flow_asks(&key, &run_dir, questions, cx);
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
        run_dir: &Path,
        ask_id: u64,
        decision: daruda_acp::PermissionDecision,
        cx: &mut App,
    ) -> Result<(), AnswerRefused> {
        let answered = cx.update_global::<RunsHost, _>(|host, _| {
            let Some(key) = host.flows.key_for(worktree) else {
                return (None, Err(AnswerRefused::NotOpen));
            };
            match host.flows.runs.get_mut(&key) {
                Some(run) if run.view.run_dir() == run_dir => {
                    let answer = run.view.answer(ask_id, decision.clone());
                    host.watch.observe_waiting(run.result());
                    (Some(key), answer)
                }
                _ => (None, Err(AnswerRefused::NotOpen)),
            }
        });
        if let (Some(key), answer) = &answered
            && *answer != Err(AnswerRefused::NotOpen)
        {
            Self::tell_windows(
                key.clone(),
                FlowChanged::AskClosed {
                    run_dir: run_dir.to_path_buf(),
                    ask_id,
                    outcome: if answer.is_ok() {
                        FlowAskOutcome::Answered(decision)
                    } else {
                        FlowAskOutcome::Withdrawn
                    },
                },
                cx,
            );
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
            let before: Vec<_> = run.view.questions().map(ParkedAsk::ask_id).collect();
            let parked = run.view.park(ParkedAsk::new(pending));
            let closed = before
                .into_iter()
                .filter(|id| run.view.question(*id).is_none())
                .collect();
            host.watch.observe_waiting(run.result());
            Some((key, parked, closed, run.submitted_by, run.solicited))
        });
        if let Some((key, parked, closed, submitted_by, solicited)) = parked {
            Self::close_flow_asks(&key, run_dir, closed, cx);
            let what = FlowChanged::Asked {
                run_dir: run_dir.to_path_buf(),
                ask_id,
                alone: parked == Parked::Alone,
                submitted_by,
                solicited,
            };
            Self::tell_windows(key, what, cx);
        }
    }

    pub(super) fn withdraw_flow_asks(worktree: &Path, run_dir: &Path, cx: &mut App) {
        let changed = cx.update_global::<RunsHost, _>(|host, _| {
            let key = host.flows.key_for(worktree)?;
            let run = host
                .flows
                .runs
                .get_mut(&key)
                .filter(|run| run.view.run_dir() == run_dir)?;
            let questions = run.view.questions().map(ParkedAsk::ask_id).collect();
            run.view.withdraw_questions();
            host.watch.observe_waiting(run.result());
            Some((key, questions))
        });
        if let Some((key, questions)) = changed {
            Self::close_flow_asks(&key, run_dir, questions, cx);
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
                let closed = run.view.questions().map(ParkedAsk::ask_id).collect();
                run.view.advance(event);
                if let Some(states) = from_log {
                    run.view.reconcile(states);
                }
                run.view.withdraw_questions();
                host.watch.observe_waiting(run.result());
                // Held, not announced: the pump reads the run's report first,
                // so no waiter or notice takes this end without its text.
                if !matches!(end, RunEnd::ShutDown) {
                    host.watch.hold_end(crate::runs::result::flow_ended(
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
                    ),
                    colouring: run.view.colouring(),
                }));
                return Some((key, closed, Some(ended)));
            }
            let run = host.flows.runs.get_mut(&key)?;
            let before: Vec<_> = run.view.questions().map(ParkedAsk::ask_id).collect();
            let advanced = run.view.advance(event);
            let closed = before
                .into_iter()
                .filter(|id| run.view.question(*id).is_none())
                .collect();
            let reconciled = from_log.is_some_and(|states| run.view.reconcile(states));
            host.watch.observe_waiting(run.result());
            let what = match (advanced, reconciled) {
                (Advanced::Moved { left_setup }, _) => Some(FlowChanged::Progressed { left_setup }),
                (Advanced::Nothing, true) => Some(FlowChanged::Progressed { left_setup: false }),
                (Advanced::Nothing, false) => None,
            };
            Some((key, closed, what))
        });
        if let Some((key, closed, what)) = what {
            Self::close_flow_asks(&key, run_dir, closed, cx);
            if let Some(what) = what {
                Self::tell_windows(key, what, cx);
            }
        }
    }

    fn close_flow_asks(
        worktree: &CanonicalTree,
        run_dir: &Path,
        questions: Vec<u64>,
        cx: &mut App,
    ) {
        for ask_id in questions {
            Self::tell_windows(
                worktree.clone(),
                FlowChanged::AskClosed {
                    run_dir: run_dir.to_path_buf(),
                    ask_id,
                    outcome: FlowAskOutcome::Withdrawn,
                },
                cx,
            );
        }
    }

    /// Tell the windows, after this update: a window asking from inside its
    /// own update would otherwise be re-entered.
    fn tell_windows(worktree: CanonicalTree, what: FlowChanged, cx: &mut App) {
        #[cfg(test)]
        cx.update_global::<RunsHost, _>(|host, _| {
            host.flows.changed.retain(|changed| !changed.is_closed());
            for changed in &host.flows.changed {
                // SILENT-OK: a full channel already holds a wake-up.
                let _ = changed.try_send(());
            }
        });
        let Some(effects) = cx.global::<RunsHost>().effects.clone() else {
            return;
        };
        let change = FlowChange { worktree, what };
        cx.defer(move |cx| effects.flow_changed(&change, cx));
    }

    /// Woken, coalesced, by every later change to any run.
    #[cfg(test)]
    pub(crate) fn flow_changes_for_test(cx: &mut App) -> smol::channel::Receiver<()> {
        let (changed, receiver) = smol::channel::bounded(1);
        cx.update_global::<RunsHost, _>(|host, _| host.flows.changed.push(changed));
        receiver
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
            solicited: submitted_by == daruda_flow::request::StartedBy::Phone,
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
            Self::release_run_end(held, crate::runs::result_io::enrich(snapshot), cx);
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
