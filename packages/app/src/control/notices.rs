//! Process-local notice acceptance. The run ledger retains pending episodes,
//! including reservations rolled back after a failed socket handoff.
mod protocol;

use crate::runs::{RunsHost, result::RunKey, watch::RunEvent};
use crate::window_registry::WindowRegistry;
use gpui::{App, Global};
use std::collections::HashSet;

type Episode = (RunKey, u64);
#[derive(Default)]
struct Notices {
    accepted: HashSet<Episode>,
    scheduled: bool,
}
impl Global for Notices {}

pub(crate) fn enqueue(event: RunEvent, cx: &mut App) {
    if RunsHost::run_scope(cx).contains(&event.snapshot.key) {
        schedule(cx);
    }
}

/// Safe inside a Workspace update: no entity is read until the deferred flush.
pub(crate) fn schedule(cx: &mut App) {
    let notices = cx.default_global::<Notices>();
    if notices.scheduled {
        return;
    }
    notices.scheduled = true;
    cx.defer(|cx| {
        cx.global_mut::<Notices>().scheduled = false;
        let Some((_, host)) = WindowRegistry::orchestrator(cx) else {
            return;
        };
        if let Err(error) = host.update(cx, |ws, cx| ws.flush_run_notices(cx)) {
            log_failure(&error.to_string());
        }
    });
}

pub(crate) struct Batch {
    episodes: Vec<Episode>,
    pub(crate) prompt: String,
}

/// Fresh filtering also recovers events created before effects installation.
pub(crate) fn batch(cx: &mut App) -> Option<Batch> {
    let scope: HashSet<_> = RunsHost::run_scope(cx).into_iter().collect();
    let events: Vec<_> = RunsHost::unreported_run_events(cx)
        .into_iter()
        .filter(|event| {
            scope.contains(&event.snapshot.key)
                && !RunsHost::run_wait_covers(&event.snapshot.key, cx)
                && !cx.try_global::<Notices>().is_some_and(|notices| {
                    notices
                        .accepted
                        .contains(&(event.snapshot.key.clone(), event.sequence))
                })
        })
        .collect();
    if events.is_empty() {
        return None;
    }
    Some(Batch {
        prompt: protocol::prompt(&events),
        episodes: events
            .into_iter()
            .map(|event| (event.snapshot.key, event.sequence))
            .collect(),
    })
}

pub(crate) fn accept(batch: Batch, cx: &mut App) {
    cx.default_global::<Notices>()
        .accepted
        .extend(batch.episodes);
}

pub(crate) fn log_failure(reason: &str) {
    use daruda_store::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    LogWriter::log(
        ErrorReport::new("Orchestrator run notice was not accepted")
            .severity(ErrorSeverity::Warning)
            .with_context("reason", reason)
            .at(file!(), line!())
            .dedup("orchestrator.run_notice")
            .build(),
    );
}

#[cfg(test)]
mod tests;
