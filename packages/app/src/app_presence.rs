//! The one place that decides whether the user is away from daruda.
//!
//! App-wide rather than per-`Workspace`: absence is one shared truth, while
//! the pump that samples it runs once per open window
//! (`WindowRegistry::for_each_workspace`).
//!
//! Owns the *judgement* only. A delivery channel calls [`is_away`] for a
//! `bool`; the thresholds and the two OS queries never reach its signature.

use std::time::{Duration, Instant};

use gpui::{App, Global, Window};

#[cfg(not(test))]
use crate::platform::attention::{is_app_active, system_idle_seconds};
use crate::platform::presence::{AwayRule, AwaySignal};

pub(crate) struct AppPresence {
    state: AwaySignal,
}

impl Global for AppPresence {}

/// Install the tracker, seeded from the live signals. Idempotent (the
/// `globals.rs` convention) so a second call cannot reset an absence already
/// in progress.
pub(crate) fn init(cx: &mut App) {
    if cx.has_global::<AppPresence>() {
        return;
    }
    let (active, idle) = sample(cx);
    let state = AwaySignal::HERE.observe(active, idle, Instant::now());
    cx.set_global(AppPresence { state });
    track_new_windows::<crate::ui::Root>(cx);
}

/// Root wraps every window daruda opens — workspace and settings alike.
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
        let state = snapshot(cx);
        let away_secs = state.away_secs(Instant::now());
        format!(
            "pid={} window_id={:?} window_active={} app_active={} away_secs={} idle_secs={}",
            std::process::id(),
            window.window_handle().window_id(),
            window.is_window_active(),
            away_secs.is_none(),
            crate::telegram::trace::opt(away_secs),
            crate::telegram::trace::opt(state.idle().map(|i| i.as_secs())),
        )
    });
}

/// Fold both absence signals in. Called on every window-activation edge, on
/// the periodic pump, and by [`is_away`] itself. A missing global is a no-op,
/// for fixtures without the app startup sequence.
///
/// The pump's call is load-bearing: absence is stamped only on an edge this
/// function observes, so an edge missed between pings would be stamped at ping
/// time with `away_secs = 0` and read as present. With no queue behind the
/// gate that ping is then dropped for good, not merely delayed — the pump
/// bounds the mis-dating to one tick.
pub(crate) fn observe(cx: &mut App) {
    if !cx.has_global::<AppPresence>() {
        return;
    }
    let (active, idle) = sample(cx);
    let now = Instant::now();
    let presence = cx.global_mut::<AppPresence>();
    presence.state = presence.state.observe(active, idle, now);
}

/// Whether the user is away right now.
///
/// Samples both signals first, so the answer is never a stale reading from
/// the last pump tick — the caller asks at the instant it must decide, and
/// gets an answer measured at that instant.
pub(crate) fn is_away(cx: &mut App) -> bool {
    observe(cx);
    snapshot(cx).is_away(rule(cx), Instant::now())
}

/// The tracked signal. Falls back to `HERE` when the global is absent, which
/// is the safe direction: present means a ping is dropped rather than sent to
/// a phone that may not want it.
pub(crate) fn snapshot(cx: &App) -> AwaySignal {
    cx.try_global::<AppPresence>()
        .map_or(AwaySignal::HERE, |presence| presence.state)
}

/// The user's definition of absence. Read live rather than mirrored onto a
/// field: one source, no sync path to keep in step, and `[presence]` picks up
/// live config reload for free. The missing-store fallback is for test
/// fixtures — every production caller reaches this through a live `Workspace`,
/// long after `SettingsStore::init`.
pub(crate) fn rule(cx: &App) -> AwayRule {
    let cfg = cx
        .try_global::<crate::settings_store::SettingsStore>()
        .map_or_else(daruda_config::PresenceConfig::default, |store| {
            store.user().presence.clone()
        });
    AwayRule {
        grace: Duration::from_secs(cfg.away_grace_secs),
        idle_bar: Duration::from_secs(cfg.away_idle_secs),
        foreground_idle_bar: Duration::from_secs(cfg.away_idle_foreground_secs),
    }
}

/// A negative idle reading is clamped to zero; an unavailable sensor stays `None`.
#[cfg(not(test))]
fn sample(_cx: &App) -> (bool, Option<Duration>) {
    let active = is_app_active();
    let idle = system_idle_seconds().map(|secs| Duration::from_secs_f64(secs.max(0.0)));
    (active, idle)
}

/// Test apps never query the host: concurrent HID queries can block in macOS.
/// An unseeded fixture starts present; overrides belong to that App alone.
#[cfg(test)]
fn sample(cx: &App) -> (bool, Option<Duration>) {
    cx.try_global::<TestPresenceSample>()
        .map_or((true, Some(Duration::ZERO)), |sample| {
            (sample.app_active, sample.idle)
        })
}

#[cfg(test)]
struct TestPresenceSample {
    app_active: bool,
    idle: Option<Duration>,
}

#[cfg(test)]
impl Global for TestPresenceSample {}

#[cfg(test)]
pub(crate) fn seed_for_test(
    state: AwaySignal,
    app_active: bool,
    idle: Option<Duration>,
    cx: &mut App,
) {
    cx.set_global(TestPresenceSample { app_active, idle });
    init(cx);
    cx.global_mut::<AppPresence>().state = state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    async fn an_uninstalled_tracker_reports_presence_rather_than_absence(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assert_eq!(snapshot(cx), AwaySignal::HERE);
            // Observing without the global installed must not panic.
            observe(cx);
            assert_eq!(snapshot(cx), AwaySignal::HERE);
        });
    }

    #[gpui::test]
    async fn install_is_idempotent_so_a_second_call_keeps_the_running_absence(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(TestPresenceSample {
                app_active: false,
                idle: Some(Duration::from_secs(300)),
            });
            init(cx);
            assert!(snapshot(cx).away_secs(Instant::now()).is_some());
            assert_eq!(snapshot(cx).idle(), Some(Duration::from_secs(300)));

            let started = Instant::now() - Duration::from_secs(30);
            let running = AwaySignal::HERE.observe(false, Some(Duration::from_secs(300)), started);
            cx.global_mut::<AppPresence>().state = running;

            init(cx);
            assert_eq!(
                snapshot(cx),
                running,
                "a second install must not restart an absence already in progress"
            );
        });
    }

    #[gpui::test]
    fn test_samples_are_isolated_and_unseeded_apps_start_present(cx: &mut TestAppContext) {
        let other = cx.new_app();
        cx.update(|cx| {
            let started = Instant::now() - Duration::from_secs(120);
            seed_for_test(
                AwaySignal::HERE.observe(false, Some(Duration::from_secs(300)), started),
                false,
                Some(Duration::from_secs(300)),
                cx,
            );
            assert!(is_away(cx));
        });
        other.update(|cx| {
            init(cx);
            assert_eq!(snapshot(cx), AwaySignal::HERE);
            assert!(!is_away(cx));
            assert_eq!(snapshot(cx), AwaySignal::HERE);
        });
        cx.update(|cx| assert!(is_away(cx)));
    }

    #[gpui::test]
    fn an_unavailable_test_sensor_stays_unavailable_on_install_and_observe(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_global(TestPresenceSample {
                app_active: false,
                idle: None,
            });
            init(cx);
            assert!(snapshot(cx).away_secs(Instant::now()).is_some());
            assert_eq!(snapshot(cx).idle(), None);

            let started = Instant::now() - Duration::from_secs(120);
            seed_for_test(
                AwaySignal::HERE.observe(false, None, started),
                false,
                None,
                cx,
            );
            assert!(is_away(cx));
            assert_eq!(snapshot(cx).idle(), None);

            cx.set_global(TestPresenceSample {
                app_active: true,
                idle: Some(Duration::ZERO),
            });
            assert!(!is_away(cx));
            assert_eq!(snapshot(cx), AwaySignal::HERE);
        });
    }

    /// The single-source-of-truth claim: `rule()` reads the live store rather
    /// than a mirrored copy, so a non-default `[presence]` reaches the gate
    /// with no sync step. The fixture fallback is asserted first so the test
    /// cannot pass by both sides being the default.
    #[gpui::test]
    async fn the_rule_is_read_live_from_the_settings_store(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let defaults = daruda_config::PresenceConfig::default();
            assert_eq!(
                rule(cx),
                AwayRule {
                    grace: Duration::from_secs(defaults.away_grace_secs),
                    idle_bar: Duration::from_secs(defaults.away_idle_secs),
                    foreground_idle_bar: Duration::from_secs(defaults.away_idle_foreground_secs),
                },
                "no store installed: the config defaults stand in"
            );

            let mut config = daruda_config::Config::default();
            config.presence.away_grace_secs = 3;
            config.presence.away_idle_secs = 7;
            config.presence.away_idle_foreground_secs = 11;
            assert_ne!(config.presence.away_grace_secs, defaults.away_grace_secs);
            crate::settings_store::SettingsStore::init(cx);
            cx.global_mut::<crate::settings_store::SettingsStore>()
                .set_user_for_testing(config);

            assert_eq!(
                rule(cx),
                AwayRule {
                    grace: Duration::from_secs(3),
                    idle_bar: Duration::from_secs(7),
                    foreground_idle_bar: Duration::from_secs(11),
                }
            );
        });
    }

    #[gpui::test]
    async fn is_away_reports_the_verdict_for_both_signals(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let started = Instant::now() - Duration::from_secs(120);
            // Blurred long enough, but the machine is still being used.
            seed_for_test(
                AwaySignal::HERE.observe(false, Some(Duration::from_secs(2)), started),
                false,
                Some(Duration::from_secs(2)),
                cx,
            );
            assert!(!is_away(cx));

            // Same absence, now with the input gone quiet.
            seed_for_test(
                AwaySignal::HERE.observe(false, Some(Duration::from_secs(300)), started),
                false,
                Some(Duration::from_secs(300)),
                cx,
            );
            assert!(is_away(cx));
        });
    }
}
