//! Background-pump helpers for the four detached watcher loops in
//! `main.rs`. Each helper covers one of three loop shapes:
//!
//! 1. [`spawn_drain_burst_pump`] — burst-debounced `mpsc::Receiver<()>`
//!    where many notifications collapse into one global apply. Used by
//!    `config_watcher` and `panels_watcher`.
//! 2. [`spawn_event_fanout_pump`] — `mpsc::Receiver<E>` where every
//!    received event is fanned out to every Workspace. Used by the
//!    Claude status watcher.
//! 3. [`spawn_periodic_pump`] — pure `setInterval`-style tick with no
//!    receiver. Used by the 30 s render-tick that demotes stale
//!    `NeedsAttention` indicators.
//!
//! All three exit cleanly when `cx.update` returns `Err` (App dropped /
//! shutting down). The previous hand-rolled loops in `main.rs` never
//! noticed shutdown and would have spun forever — though `.detach()`
//! makes that a leak rather than a hang.

use std::sync::mpsc;
use std::time::Duration;

use gpui::App;

use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// Log a one-line shutdown notice when `cx.update` fails because the
/// App was dropped. Caller decides whether to `return` (exit the spawn)
/// or fall through.
fn log_app_drop(name: &'static str) {
    eprintln!("daruda: {name} pump exiting — App dropped");
}

/// Pump a "many-events-collapse-to-one-apply" receiver — the typical
/// shape for filesystem watchers feeding live-reload.
///
/// Pattern: poll `rx` every `tick`; if at least one signal arrived,
/// drain the rest of the burst (so a flurry of write/rename/chmod
/// events from a single editor save coalesces into one reload), then
/// run `apply` exactly once on the GPUI app.
pub fn spawn_drain_burst_pump(
    name: &'static str,
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
            if cx.update(|cx| apply(cx)).is_err() {
                log_app_drop(name);
                return;
            }
        }
    })
    .detach();
}

/// Pump a "one-event-per-apply" receiver — every received event is
/// fanned out to every open Workspace via `apply_per_event`. Suitable
/// for low-rate event streams where each item carries unique state.
///
/// `apply_per_event(event, workspace, cx)` is invoked once per
/// (event, workspace) pair via [`WindowRegistry::for_each_workspace`].
pub fn spawn_event_fanout_pump<E: Clone + Send + 'static>(
    name: &'static str,
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
            let r = cx.update(|cx| {
                WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
                    apply_per_event(event.clone(), ws, window, cx);
                });
            });
            if r.is_err() {
                log_app_drop(name);
                return;
            }
        }
    })
    .detach();
}

/// Pump a pure timer — runs `apply` on the GPUI app every `period`,
/// exits cleanly when the App drops. Use sparingly: the 30 s
/// `NeedsAttention` demotion tick exists because GPUI's animation
/// scope doesn't reach the parent `Workspace`. Adding more periodic
/// pumps without escalating the underlying limitation is a smell.
pub fn spawn_periodic_pump(
    name: &'static str,
    period: Duration,
    apply: impl Fn(&mut App) + Send + 'static,
    cx: &mut App,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(period).await;
            if cx.update(|cx| apply(cx)).is_err() {
                log_app_drop(name);
                return;
            }
        }
    })
    .detach();
}
