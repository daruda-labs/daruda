//! Process-local notice acceptance. The run ledger retains pending episodes,
//! including reservations rolled back after a failed socket handoff.
mod protocol;

use crate::runs::{RunsHost, result::RunKey, watch::RunEvent};
use crate::window_registry::WindowRegistry;
use gpui::{App, Entity, EntityId, Global, Subscription};
use std::collections::HashSet;

type Episode = (RunKey, u64);

/// The conversation a notice reaches: a pane and the agent session behind it.
/// A replaced session has not read what its predecessor was told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Audience {
    pane: EntityId,
    session: Option<String>,
}

impl Audience {
    pub(crate) fn new(pane: EntityId, session: Option<&str>) -> Self {
        Self {
            pane,
            session: session.map(str::to_owned),
        }
    }
}

/// Episodes already put in front of one audience.
struct Told {
    audience: Audience,
    episodes: HashSet<Episode>,
}

#[derive(Default)]
struct Notices {
    told: Option<Told>,
    scheduled: bool,
    /// Watches a pane that could not take a notice now, until it can.
    retry: Option<Subscription>,
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

/// Schedule a flush once `ready` holds after a change to `entity`. A later
/// call replaces the watch; a successful send ends it.
pub(crate) fn retry_when<T: 'static>(
    entity: &Entity<T>,
    ready: impl Fn(&T) -> bool + 'static,
    cx: &mut App,
) {
    let subscription = cx.observe(entity, move |entity, cx| {
        if ready(entity.read(cx)) {
            cx.default_global::<Notices>().retry = None;
            schedule(cx);
        }
    });
    cx.default_global::<Notices>().retry = Some(subscription);
}

pub(crate) struct Batch {
    audience: Audience,
    episodes: Vec<Episode>,
    pub(crate) prompt: String,
}

/// Fresh filtering also recovers events created before effects installation.
pub(crate) fn batch(audience: Audience, cx: &mut App) -> Option<Batch> {
    let scope: HashSet<_> = RunsHost::run_scope(cx).into_iter().collect();
    let unreported = RunsHost::unreported_run_events(cx);
    let live: HashSet<Episode> = unreported
        .iter()
        .map(|event| (event.snapshot.key.clone(), event.sequence))
        .collect();
    let notices = cx.default_global::<Notices>();
    if let Some(told) = &mut notices.told {
        // Reported or superseded episodes can never be offered again.
        told.episodes.retain(|episode| live.contains(episode));
    }
    let told = notices
        .told
        .as_ref()
        .filter(|told| told.audience == audience)
        .map(|told| told.episodes.clone())
        .unwrap_or_default();
    let events: Vec<_> = unreported
        .into_iter()
        .filter(|event| {
            scope.contains(&event.snapshot.key)
                && !RunsHost::run_wait_covers(&event.snapshot.key, cx)
                && !told.contains(&(event.snapshot.key.clone(), event.sequence))
        })
        .collect();
    if events.is_empty() {
        return None;
    }
    Some(Batch {
        audience,
        prompt: protocol::prompt(&events),
        episodes: events
            .into_iter()
            .map(|event| (event.snapshot.key, event.sequence))
            .collect(),
    })
}

/// The audience received the batch; a new audience starts a fresh record.
pub(crate) fn accept(batch: Batch, cx: &mut App) {
    let notices = cx.default_global::<Notices>();
    notices.retry = None;
    match &mut notices.told {
        Some(told) if told.audience == batch.audience => told.episodes.extend(batch.episodes),
        told => {
            *told = Some(Told {
                audience: batch.audience,
                episodes: batch.episodes.into_iter().collect(),
            })
        }
    }
}

#[cfg(test)]
fn told_count(cx: &App) -> usize {
    cx.try_global::<Notices>()
        .and_then(|notices| notices.told.as_ref())
        .map_or(0, |told| told.episodes.len())
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
