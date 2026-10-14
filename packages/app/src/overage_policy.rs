//! `[usage] stop_on_overage` as a flag flow runs can read off the UI thread.
//!
//! A run's engine thread has no `App`, so the setting is mirrored into one
//! shared `AtomicBool` every run's `OverageStop` holds — a toggle mid-run
//! reaches the runs already going.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use daruda_flow::runner::OverageStop;
use gpui::{App, Global};

/// `[usage] stop_on_overage` as the settings store holds it — the one read
/// path on the UI thread.
pub(crate) fn stop_on_overage_enabled(cx: &App) -> bool {
    crate::settings_store::SettingsStore::global(cx)
        .user()
        .usage
        .stop_on_overage
}

#[derive(Default)]
pub(crate) struct OveragePolicy {
    enabled: Arc<AtomicBool>,
}

impl Global for OveragePolicy {}

impl OveragePolicy {
    /// The one update site of the mirror: installs the global if missing and
    /// copies the current setting into it. Called at startup, by the
    /// app-wide settings observer, and by workspace construction for tests.
    /// A test that skips `register_settings_observer` calls this after
    /// changing the setting.
    pub(crate) fn sync(cx: &mut App) {
        let enabled = stop_on_overage_enabled(cx);
        // Installed once, then written through the shared atomic: a
        // `default_global` borrow would notify global observers every time.
        if !cx.has_global::<Self>() {
            cx.set_global(Self::default());
        }
        cx.global::<Self>().set(enabled);
    }

    /// A fresh switch for one run, reading the live setting.
    pub(crate) fn stop_for_run(cx: &App) -> OverageStop {
        cx.global::<Self>().stop()
    }

    fn set(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    fn stop(&self) -> OverageStop {
        OverageStop::new(self.enabled.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{RateLimitView, RateLimitWindow};

    fn billed() -> RateLimitView {
        RateLimitView {
            window: Some(RateLimitWindow::FiveHour),
            resets_at: None,
            is_using_overage: true,
        }
    }

    #[test]
    fn setting_the_policy_reaches_a_run_started_before() {
        let policy = OveragePolicy::default();
        let running = policy.stop();
        assert!(!running.trip(&billed()), "off by default");
        policy.set(true);
        assert!(running.trip(&billed()));
    }

    #[test]
    fn turning_it_off_mid_run_stops_tripping() {
        let policy = OveragePolicy::default();
        policy.set(true);
        let running = policy.stop();
        policy.set(false);
        assert!(!running.trip(&billed()));
    }
}
