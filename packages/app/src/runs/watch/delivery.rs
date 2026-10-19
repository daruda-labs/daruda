//! Reservation and acknowledgement of process-local delivery episodes.
use super::state::{EndState, Membership, RunEvent, RunWatch};
use crate::runs::result::{RunKey, RunSnapshot, RunState};
use std::collections::HashSet;

const BATCH_MAX: usize = 20;
const REPORTED_MAX: usize = 200;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct WaitId(u64);
#[derive(Debug)]
pub(crate) struct WaiterBusy;
impl std::fmt::Display for WaiterBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an omitted-handles run wait is already active")
    }
}
impl std::error::Error for WaiterBusy {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Delivery {
    Unreported,
    InFlight(WaitId),
    Reported,
}
/// Which keys a waiter answers for.
pub(super) enum WaiterScope {
    /// Every key in the orchestrator's scope, as it grows.
    Omitted,
    Explicit(HashSet<RunKey>),
}
pub(super) struct Waiter {
    scope: WaiterScope,
    pub(super) wake: smol::channel::Sender<()>,
}
impl Waiter {
    /// Whether the waiter named `key` itself, as opposed to covering it by scope.
    pub(super) fn names(&self, key: &RunKey) -> bool {
        matches!(&self.scope, WaiterScope::Explicit(keys) if keys.contains(key))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    Settled,
    TimedOut,
    Nothing,
}
pub(crate) struct WaitAnswer {
    pub outcome: WaitOutcome,
    pub ended: Vec<RunSnapshot>,
    pub blocked: Vec<RunSnapshot>,
    pub already_reported: Vec<RunSnapshot>,
    pub not_running: Vec<RunSnapshot>,
    pub pending: Vec<RunSnapshot>,
    pub more: usize,
}
impl RunWatch {
    pub(crate) fn open_wait(
        &mut self,
        keys: Option<Vec<RunKey>>,
    ) -> Result<(WaitId, smol::channel::Receiver<()>), WaiterBusy> {
        if keys.is_none()
            && self
                .waiters
                .values()
                .any(|w| matches!(w.scope, WaiterScope::Omitted))
        {
            return Err(WaiterBusy);
        }
        if let Some(keys) = &keys {
            for key in keys {
                self.enroll(key.clone());
            }
        }
        self.next_waiter += 1;
        let id = WaitId(self.next_waiter);
        let (wake, receiver) = smol::channel::bounded(1);
        self.waiters.insert(
            id,
            Waiter {
                scope: keys.map_or(WaiterScope::Omitted, |keys| {
                    WaiterScope::Explicit(keys.into_iter().collect())
                }),
                wake,
            },
        );
        Ok((id, receiver))
    }
    pub(crate) fn wait_is_live(&self, id: WaitId) -> bool {
        self.waiters.contains_key(&id)
    }
    pub(crate) fn covered(&self, key: &RunKey) -> bool {
        self.waiters.values().any(|w| match &w.scope {
            WaiterScope::Omitted => self.membership(key) == Membership::Scoped,
            WaiterScope::Explicit(keys) => keys.contains(key),
        })
    }
    pub(crate) fn unreported(&self) -> Vec<RunEvent> {
        self.events
            .iter()
            .filter(|event| event.delivery == Delivery::Unreported)
            .cloned()
            .collect()
    }
    /// The waiter remains live while its exact event reservations await the socket receipt.
    pub(crate) fn finish_wait(&mut self, id: WaitId, delivered: bool) {
        let Some(waiter) = self.waiters.remove(&id) else {
            return;
        };
        let mut reported = Vec::new();
        for event in &mut self.events {
            if event.delivery == Delivery::InFlight(id) {
                event.delivery = if delivered {
                    Delivery::Reported
                } else {
                    Delivery::Unreported
                };
                if delivered && matches!(event.snapshot.state, RunState::Ended { .. }) {
                    reported.push((event.snapshot.key.clone(), event.sequence));
                }
            }
        }
        for (key, sequence) in reported {
            if sequence >= self.floor(&key) {
                self.entry(&key).end = EndState::Reported;
            }
        }
        // A closed block or a terminal supersedes only an unreported blocked episode.
        let entries = &self.entries;
        self.events.retain(|event| {
            event.delivery != Delivery::Unreported
                || !matches!(event.snapshot.state, RunState::Running { .. })
                || entries
                    .get(&event.snapshot.key)
                    .and_then(|entry| entry.blocked_sequence())
                    == Some(event.sequence)
        });
        if let WaiterScope::Explicit(keys) = waiter.scope {
            for key in keys {
                self.release_enrollment(&key);
            }
        }
        self.prune();
        self.forget_idle();
        for event in self.unreported() {
            self.announce(event);
        }
        self.wake();
    }
    fn prune(&mut self) {
        let reported = self
            .events
            .iter()
            .filter(|e| e.delivery == Delivery::Reported)
            .count();
        let mut remove = reported.saturating_sub(REPORTED_MAX);
        let mut keys = HashSet::new();
        self.events.retain(|e| {
            if remove > 0 && e.delivery == Delivery::Reported {
                remove -= 1;
                keys.insert(e.snapshot.key.clone());
                false
            } else {
                true
            }
        });
        for key in keys {
            if !self.events.iter().any(|e| e.snapshot.key == key)
                && let Some(entry) = self.entries.get_mut(&key)
                && entry.end == EndState::Reported
            {
                entry.snapshot = None;
            }
        }
        // A disk/history query may rehydrate a previously evicted reported handle.
        let mut reported_snapshots: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.snapshot.is_some() && entry.end == EndState::Reported)
            .filter_map(|(key, _)| {
                let last = self
                    .events
                    .iter()
                    .rposition(|event| &event.snapshot.key == key);
                let protected = self.events.iter().any(|event| {
                    &event.snapshot.key == key && event.delivery != Delivery::Reported
                });
                (!protected).then(|| (key.clone(), last))
            })
            .collect();
        reported_snapshots.sort_by_key(|(_, last)| *last);
        let excess = reported_snapshots.len().saturating_sub(REPORTED_MAX);
        for (key, _) in reported_snapshots.into_iter().take(excess) {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.snapshot = None;
            }
        }
    }

    pub(crate) fn poll_wait(&mut self, id: WaitId, timed_out: bool) -> Option<WaitAnswer> {
        let waiter = self.waiters.get(&id)?;
        let omitted = matches!(waiter.scope, WaiterScope::Omitted);
        let keys: Vec<RunKey> = match &waiter.scope {
            WaiterScope::Omitted => self.scope(),
            WaiterScope::Explicit(keys) => keys.iter().cloned().collect(),
        };
        let wanted: HashSet<&RunKey> = keys.iter().collect();
        let mut eligible: Vec<_> = self
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                wanted.contains(&e.snapshot.key)
                    && e.delivery == Delivery::Unreported
                    && (omitted
                        || (e.sequence >= self.floor(&e.snapshot.key))
                            && self
                                .events
                                .iter()
                                .rev()
                                .find(|other| other.snapshot.key == e.snapshot.key)
                                .is_some_and(|last| last.sequence == e.sequence))
            })
            .map(|(i, e)| (i, matches!(e.snapshot.state, RunState::Running { .. })))
            .collect();
        eligible.sort_by_key(|(_, blocked)| *blocked);
        let more = eligible.len().saturating_sub(BATCH_MAX);
        let mut answer = WaitAnswer {
            outcome: WaitOutcome::Settled,
            ended: vec![],
            blocked: vec![],
            already_reported: vec![],
            not_running: vec![],
            pending: vec![],
            more,
        };
        let selected: HashSet<_> = eligible
            .iter()
            .take(BATCH_MAX)
            .map(|(i, _)| self.events[*i].snapshot.key.clone())
            .collect();
        let mut in_flight = false;
        for key in &keys {
            let floor = self.floor(key);
            let last = self
                .events
                .iter()
                .rev()
                .find(|e| &e.snapshot.key == key && e.sequence >= floor);
            in_flight |= last.is_some_and(|e| matches!(e.delivery, Delivery::InFlight(_)));
            let entry = self.entries.get(key);
            let end = entry.map(|entry| entry.end).unwrap_or_default();
            let snapshot = entry.and_then(|entry| entry.snapshot.as_ref());
            if end == EndState::Held {
                // The owner is still enriching this end; it is announced on release.
                in_flight = true;
                continue;
            }
            if !omitted
                && end == EndState::Reported
                && let Some(snapshot) = snapshot
                && matches!(snapshot.state, RunState::Ended { .. })
            {
                answer.already_reported.push(snapshot.clone());
                continue;
            }
            if let Some(snapshot) = snapshot {
                match snapshot.state {
                    RunState::Running { .. } if !selected.contains(key) => {
                        answer.pending.push(snapshot.clone())
                    }
                    RunState::Unknown | RunState::Gone | RunState::Unreadable { .. }
                        if !omitted =>
                    {
                        answer.not_running.push(snapshot.clone())
                    }
                    _ => {}
                }
            } else if end != EndState::Reported {
                // A registered start may be observed before its first projection.
                in_flight = true;
            }
        }
        if eligible.is_empty() {
            let running = !answer.pending.is_empty() || in_flight;
            if running && !timed_out {
                return None;
            }
            answer.outcome = if running {
                WaitOutcome::TimedOut
            } else if omitted {
                WaitOutcome::Nothing
            } else {
                WaitOutcome::Settled
            };
        }
        for (i, blocked) in eligible.into_iter().take(BATCH_MAX) {
            let event = &mut self.events[i];
            event.delivery = Delivery::InFlight(id);
            if blocked {
                answer.blocked.push(event.snapshot.clone());
            } else {
                answer.ended.push(event.snapshot.clone());
            }
        }
        Some(answer)
    }
}
#[cfg(test)]
mod tests;
