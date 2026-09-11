//! The one place that remembers how long daruda has been out of the
//! foreground.
//!
//! App-wide rather than per-`Workspace`: `NSApplication.isActive` is a fact
//! about the process, but `flush_deferred_telegram` runs once per open
//! window (`WindowRegistry::for_each_workspace`). State on `Workspace`
//! would give every window its own drifting copy of one shared truth.

use std::time::Instant;

use gpui::{App, Global, Window};

use crate::platform::attention::is_app_active;
use crate::platform::presence::Presence;

pub(crate) struct AppPresence {
    state: Presence,
}

impl Global for AppPresence {}

/// Install the tracker, seeded from the live foreground state. Idempotent
/// (the `globals.rs` convention) so a second call cannot reset an absence
/// already in progress.
pub(crate) fn init(cx: &mut App) {
    if cx.has_global::<AppPresence>() {
        return;
    }
    let state = Presence::Here.observe(is_app_active(), Instant::now());
    cx.set_global(AppPresence { state });
    track_new_windows::<crate::ui::Root>(cx);
    track_new_windows::<crate::welcome::WelcomeScreen>(cx);
}

/// Root wraps workspace and settings windows; WelcomeScreen is a bare root.
/// Register at app startup so auxiliary windows participate without a Workspace.
fn track_new_windows<T: 'static>(cx: &App) {
    cx.observe_new::<T>(|_, window, cx| {
        if let Some(window) = window {
            cx.observe_window_activation(window, |_, window, cx| {
                observe_window_activation(window, cx);
            })
            .detach();
        }
    })
    .detach();
}

fn observe_window_activation(window: &Window, cx: &mut App) {
    observe(cx);
    crate::telegram::trace::state("window.activation", || {
        let away_secs = snapshot(cx).away_secs(Instant::now());
        format!(
            "pid={} window_id={:?} window_active={} app_active={} away_secs={}",
            std::process::id(),
            window.window_handle().window_id(),
            window.is_window_active(),
            away_secs.is_none(),
            crate::telegram::trace::opt(away_secs),
        )
    });
}

/// Observe on activation, before a new relay, and at each flush tick.
/// Missing globals are a no-op for fixtures without the app startup sequence.
pub(crate) fn observe(cx: &mut App) {
    if !cx.has_global::<AppPresence>() {
        return;
    }
    let active = is_app_active();
    #[cfg(test)]
    let active = cx
        .try_global::<TestPresenceSample>()
        .map_or(active, |sample| sample.0);
    let now = Instant::now();
    let presence = cx.global_mut::<AppPresence>();
    presence.state = presence.state.observe(active, now);
}

/// The tracked presence. Falls back to `Here` when the global is absent,
/// which is the safe direction: `Here` holds pings rather than releasing
/// them.
pub(crate) fn snapshot(cx: &App) -> Presence {
    cx.try_global::<AppPresence>()
        .map_or(Presence::Here, |presence| presence.state)
}

#[cfg(test)]
struct TestPresenceSample(bool);

#[cfg(test)]
impl Global for TestPresenceSample {}

#[cfg(test)]
pub(crate) fn seed_for_test(state: Presence, app_active: bool, cx: &mut App) {
    init(cx);
    cx.global_mut::<AppPresence>().state = state;
    cx.set_global(TestPresenceSample(app_active));
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    async fn an_uninstalled_tracker_reports_presence_rather_than_absence(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert_eq!(snapshot(cx), Presence::Here);
            // Observing without the global installed must not panic.
            observe(cx);
            assert_eq!(snapshot(cx), Presence::Here);
        });
    }

    #[gpui::test]
    async fn install_is_idempotent_so_a_second_call_keeps_the_running_absence(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            init(cx);
            let since = Instant::now();
            cx.global_mut::<AppPresence>().state = Presence::Away { since };
            init(cx);
            assert_eq!(snapshot(cx), Presence::Away { since });
        });
    }
}
