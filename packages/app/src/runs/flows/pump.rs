//! The one task per run that folds its notices and questions into the
//! host, in arrival order, until it ends.

use std::path::{Path, PathBuf};

use daruda_flow::event::{FlowEvent, RunEnd, RunNotice};
use daruda_flow::lock::CanonicalTree;
use daruda_flow::runner::PendingAsk;
use gpui::{App, AsyncApp, BorrowAppContext as _};

use super::{NodeRunStates, colour};
use crate::runs::RunsHost;

/// The two moments a run's view is checked against its log: its start,
/// which for a resume is all the earlier process did, and its end, which a
/// stopped attempt does not narrate.
pub(super) fn reconciles_with_log(event: &FlowEvent) -> bool {
    matches!(
        event,
        FlowEvent::RunStarted { .. } | FlowEvent::RunEnded { .. }
    )
}

pub(super) fn states_in_log(run_dir: &Path) -> Option<NodeRunStates> {
    daruda_flow::run_log::reader::state(run_dir).map(|state| colour::states_from_log(&state))
}

/// The end `event` held for the run in `run_dir`, with its release token,
/// while its report is still to be read.
pub(super) fn held_end(
    run_dir: &Path,
    event: &FlowEvent,
    cx: &App,
) -> Option<(
    crate::runs::result::RunSnapshot,
    crate::runs::watch::HeldEnd,
)> {
    if !matches!(event, FlowEvent::RunEnded { end } if !matches!(end, RunEnd::ShutDown)) {
        return None;
    }
    cx.global::<RunsHost>()
        .watch
        .held_end(&crate::runs::result::RunKey::flow_of(run_dir))
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
pub(super) async fn pump(
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
                    let key = crate::runs::result::RunKey::flow_of(&run_dir);
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
                .spawn(async move { crate::runs::result_io::enrich(snapshot) })
                .await;
            cx.update(|cx| RunsHost::release_run_end(held, enriched, cx));
        }
        if ended {
            break;
        }
    }
}
