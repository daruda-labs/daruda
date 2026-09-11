//! The one place that remembers how long daruda has been out of the
//! foreground.
//!
//! App-wide rather than per-`Workspace`: `NSApplication.isActive` is a fact
//! about the process, but `flush_deferred_telegram` runs once per open
//! window (`WindowRegistry::for_each_workspace`). State on `Workspace`
//! would give every window its own drifting copy of one shared truth.

use std::time::Instant;

use gpui::{App, Global};

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
}

/// Fold the current foreground state in. Called from every window's
/// activation edge, which makes the absence timestamp precise, and from the
/// deferred-flush pump, which is what actually guarantees correctness when
/// no window reports a transition (every window minimized, say).
///
/// A missing global is a no-op: test fixtures build workspaces without
/// `globals::init_all`, and a window activating then is not a presence fact
/// anyone reads.
pub(crate) fn observe(cx: &mut App) {
    if !cx.has_global::<AppPresence>() {
        return;
    }
    let (active, now) = (is_app_active(), Instant::now());
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
