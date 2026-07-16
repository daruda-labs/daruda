//! Background-pump helpers for watcher loops registered by `watchers_lifecycle`.
//!
//! Covers burst-drained unit signals, per-event workspace fanout, and periodic
//! ticks. All are detached tasks; app shutdown cancels them at the next
//! `timer.await`, so no explicit shutdown channel is needed.

use std::sync::mpsc;
use std::time::Duration;

use gpui::App;

use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// Pump a receiver where many filesystem signals collapse into one apply.
pub fn spawn_drain_burst_pump(
    rx: mpsc::Receiver<()>,
    tick: Duration,
    apply: impl Fn(&mut App) + Send + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(tick).await;
            if rx.try_recv().is_err() {
                continue;
            }
            // Drain bursts so we apply once per settling point.
            while rx.try_recv().is_ok() {}
            cx.update(|cx| apply(cx));
        }
    })
    .detach();
}

/// Pump low-rate events, fanning each item out to every open Workspace.
pub fn spawn_event_fanout_pump<E: Clone + Send + 'static>(
    rx: mpsc::Receiver<E>,
    tick: Duration,
    apply_per_event: impl Fn(E, &mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>)
    + Send
    + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(tick).await;
            let Ok(event) = rx.try_recv() else {
                continue;
            };
            cx.update(|cx| {
                WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
                    apply_per_event(event.clone(), ws, window, cx);
                });
            });
        }
    })
    .detach();
}

/// Pump a pure timer. Use sparingly; periodic app-wide invalidation should
/// have a concrete GPUI limitation behind it.
pub fn spawn_periodic_pump(
    period: Duration,
    apply: impl Fn(&mut App) + Send + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(period).await;
            cx.update(|cx| apply(cx));
        }
    })
    .detach();
}
