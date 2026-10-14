//! Keep the computer awake while agents work, on every platform.
//!
//! One app-wide Global owns the OS sleep lease and re-decides whenever an
//! agent status, a window, or the settings change. The decision itself is
//! the GPUI-free [`decide`].

mod decide;

use std::time::{Duration, Instant};

use daruda_config::KeepAwakeMode;
use gpui::{App, Global};

use crate::platform::power::{self, SleepLease};
use crate::settings_store::SettingsStore;
use crate::window_registry::WindowRegistry;
use decide::{Caps, Decision, Observation, SessionClocks, SessionKey};

/// How long a failed hold waits before it is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What the keep-awake policy is doing right now, for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum KeepAwakeStatus {
    #[default]
    Inactive,
    Active {
        working: usize,
        waiting: usize,
    },
    /// The OS refused the last lease change; retried after a pause.
    Failed,
}

struct KeepAwake {
    lease: Box<dyn SleepLease>,
    clocks: SessionClocks,
    status: KeepAwakeStatus,
    retry_after: Option<Instant>,
    queued: bool,
    /// Re-apply the lease at the next decision even if it looks unchanged.
    reassert: bool,
    /// A failure was logged and no lease change has succeeded since, so a
    /// lease that keeps failing is logged once rather than on every retry.
    failure_reported: bool,
    #[cfg(test)]
    failure_reports: usize,
}

impl Global for KeepAwake {}

/// Install the policy over this OS's sleep lease.
pub(crate) fn install(cx: &mut App) {
    install_with(power::backend(), cx);
}

/// Install the policy over `lease`; tests pass a fake.
pub(crate) fn install_with(lease: Box<dyn SleepLease>, cx: &mut App) {
    cx.set_global(KeepAwake {
        lease,
        clocks: SessionClocks::default(),
        status: KeepAwakeStatus::Inactive,
        retry_after: None,
        queued: false,
        reassert: false,
        failure_reported: false,
        #[cfg(test)]
        failure_reports: 0,
    });
    cx.observe_global::<SettingsStore>(refresh).detach();
    refresh(cx);
}

/// Re-decide once every window has settled. Deferred and coalesced like the
/// Dock badge: a status change can fire inside a workspace's own update, and
/// the decision reads every workspace. A no-op before [`install`].
pub(crate) fn refresh(cx: &mut App) {
    let Some(keep_awake) = cx.try_global::<KeepAwake>() else {
        return;
    };
    if keep_awake.queued {
        return;
    }
    cx.global_mut::<KeepAwake>().queued = true;
    cx.defer(run);
}

/// Re-apply the current lease even if nothing changed — Windows drops the
/// thread's execution state across a suspend.
#[cfg(windows)]
pub(crate) fn reassert(cx: &mut App) {
    if !cx.has_global::<KeepAwake>() {
        return;
    }
    cx.global_mut::<KeepAwake>().reassert = true;
    refresh(cx);
}

/// The policy's current state; `Inactive` before [`install`].
pub(crate) fn status(cx: &App) -> KeepAwakeStatus {
    cx.try_global::<KeepAwake>()
        .map_or(KeepAwakeStatus::Inactive, |keep_awake| keep_awake.status)
}

/// How many lease failures were logged.
#[cfg(test)]
pub(crate) fn failure_reports(cx: &App) -> usize {
    cx.try_global::<KeepAwake>()
        .map_or(0, |keep_awake| keep_awake.failure_reports)
}

/// The configured mode; `Off` for fixtures without a settings store.
pub(crate) fn mode(cx: &App) -> KeepAwakeMode {
    cx.try_global::<SettingsStore>()
        .map_or(KeepAwakeMode::Off, |store| store.user().desktop.keep_awake)
}

fn run(cx: &mut App) {
    // Cleared first so a refresh fired while observing queues another pass.
    cx.global_mut::<KeepAwake>().queued = false;
    let (mode, caps) = match cx.try_global::<SettingsStore>() {
        Some(store) => {
            let desktop = &store.user().desktop;
            (desktop.keep_awake, Caps::from_config(desktop))
        }
        None => (KeepAwakeMode::Off, Caps::from_config(&Default::default())),
    };
    let observations = if mode == KeepAwakeMode::Off {
        Vec::new()
    } else {
        observe(cx)
    };
    let now = Instant::now();
    let keep_awake = cx.global_mut::<KeepAwake>();
    let decision = decide::decide(mode, observations, &mut keep_awake.clocks, caps, now);
    let before = keep_awake.status;
    keep_awake.apply(decision, now);
    if keep_awake.status != before {
        // The status bar of every window shows it.
        WindowRegistry::for_each_workspace(cx, |_, _, cx| cx.notify());
    }
}

/// Every agent session in every window.
fn observe(cx: &mut App) -> Vec<Observation> {
    let mut observations = Vec::new();
    WindowRegistry::for_each_workspace(cx, |ws, _, cx| {
        let workspace = ws.uuid();
        observations.extend(
            ws.session_statuses(cx)
                .into_iter()
                .map(|(session, status)| Observation {
                    key: SessionKey { workspace, session },
                    status,
                }),
        );
    });
    observations
}

impl KeepAwake {
    /// Bring the lease in line with `decision`, recording the outcome.
    fn apply(&mut self, decision: Decision, now: Instant) {
        let reassert = std::mem::take(&mut self.reassert);
        if !decision.active {
            // The pause only gates holds; an inactive decision ends it.
            self.retry_after = None;
        }
        if reassert || self.lease.is_held() != decision.active {
            let waiting_out_a_failure = self.retry_after.is_some_and(|retry| now < retry);
            if decision.active && waiting_out_a_failure {
                return;
            }
            if let Err(error) = self.lease.set_active(decision.active) {
                self.report_once(&error);
                self.retry_after = Some(now + RETRY_AFTER);
                self.status = KeepAwakeStatus::Failed;
                return;
            }
            self.retry_after = None;
            self.failure_reported = false;
        }
        self.status = if decision.active {
            KeepAwakeStatus::Active {
                working: decision.working,
                waiting: decision.waiting,
            }
        } else {
            KeepAwakeStatus::Inactive
        };
    }

    /// Log the first failure since the last successful lease change; the
    /// retries that follow stay silent.
    fn report_once(&mut self, error: &power::PowerError) {
        if std::mem::replace(&mut self.failure_reported, true) {
            return;
        }
        #[cfg(test)]
        {
            self.failure_reports += 1;
        }
        crate::platform::report_error("desktop.power.lease", "Sleep lease update failed", error);
    }
}
