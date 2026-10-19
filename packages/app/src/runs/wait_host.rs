//! Foreground access to process-owned waiter reservations.
use super::{
    RunsHost,
    result::{RunKey, RunSnapshot, RunState},
    watch::{RunEvent, WaitAnswer, WaitId, WaiterBusy},
};
use gpui::{App, BorrowAppContext as _};
impl RunsHost {
    pub(crate) fn open_run_wait(
        keys: Option<Vec<RunKey>>,
        cx: &mut App,
    ) -> Result<(WaitId, smol::channel::Receiver<()>), WaiterBusy> {
        Self::ensure_installed(cx);
        cx.update_global::<Self, _>(|host, _| host.watch.open_wait(keys))
    }
    pub(crate) fn run_revision(key: &RunKey, cx: &App) -> u64 {
        cx.try_global::<Self>()
            .map(|host| host.watch.revision(key))
            .unwrap_or_default()
    }
    /// A lifecycle change while the background read was pending wins over the staged read.
    pub(crate) fn seed_run_wait_if_current(snapshot: RunSnapshot, revision: u64, cx: &mut App) {
        if Self::run_revision(&snapshot.key, cx) == revision {
            Self::seed_run_wait(snapshot, cx);
        }
    }
    pub(crate) fn seed_run_observation_if_current(
        observation: super::result_host::QueryObservation,
        revision: u64,
        cx: &mut App,
    ) {
        if Self::run_revision(&observation.snapshot.key, cx) != revision {
            return;
        }
        if let Some(episode) = observation.disk_episode {
            cx.update_global::<Self, _>(|host, _| {
                host.watch.observe_disk(observation.snapshot, episode)
            });
        } else {
            Self::seed_run_wait(observation.snapshot, cx);
        }
    }
    pub(crate) fn seed_run_wait(snapshot: RunSnapshot, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| {
            if host
                .watch
                .get(&snapshot.key)
                .is_some_and(|held| matches!(held.state, RunState::Ended { .. }))
            {
                host.watch.enrich(snapshot);
            } else if matches!(snapshot.state, RunState::Running { .. }) {
                host.watch.observe_waiting(snapshot);
            } else {
                host.watch.retain(snapshot);
            }
        });
    }
    pub(crate) fn poll_run_wait(id: WaitId, timed_out: bool, cx: &mut App) -> Option<WaitAnswer> {
        cx.update_global::<Self, _>(|host, _| host.watch.poll_wait(id, timed_out))
    }
    pub(crate) fn run_wait_is_live(id: WaitId, cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|host| host.watch.wait_is_live(id))
    }
    pub(crate) fn finish_run_wait(id: WaitId, delivered: bool, cx: &mut App) {
        cx.update_global::<Self, _>(|host, _| host.watch.finish_wait(id, delivered));
    }
    /// Notice delivery rechecks these process-owned facts immediately before sending.
    pub(crate) fn unreported_run_events(cx: &App) -> Vec<RunEvent> {
        cx.try_global::<Self>()
            .map(|host| host.watch.unreported())
            .unwrap_or_default()
    }
    pub(crate) fn run_wait_covers(key: &RunKey, cx: &App) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|host| host.watch.covered(key))
    }
}
#[cfg(test)]
mod tests;
