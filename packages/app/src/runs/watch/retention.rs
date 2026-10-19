//! Keeps the ledger's per-key memory from growing with every run ever seen.
use super::state::{EndState, Membership, RunEntry, RunWatch};
use crate::runs::result::{RunKey, RunState};
use std::collections::HashSet;

/// Reported identities remembered after their result snapshots are evicted.
pub(super) const RETIRED_MAX: usize = 1000;
/// Reconstructible settled observations retained when nobody follows them.
pub(super) const OBSERVATIONS_MAX: usize = 1000;
const SWEEP_HEADROOM: usize = 256;

impl RunWatch {
    /// Amortize observation sweeps over new keys, independently of protected entries.
    pub(super) fn bound(&mut self) {
        if self.entries.len() > self.next_sweep.max(OBSERVATIONS_MAX) {
            self.forget_idle();
        }
    }

    /// Forget keys that hold nothing anyone can still receive or ask for, and
    /// the run locations of the flows among them. A key whose end was reported
    /// is kept, up to the cap, so a later read of it is not delivered again.
    pub(super) fn forget_idle(&mut self) {
        let previous_len = self.entries.len();
        let busy: HashSet<RunKey> = self
            .events
            .iter()
            .map(|event| event.snapshot.key.clone())
            .chain(
                self.entries
                    .keys()
                    .filter(|key| self.waiters.values().any(|waiter| waiter.names(key)))
                    .cloned(),
            )
            .collect();
        let mut observations: Vec<_> = self
            .entries
            .iter()
            .filter(|(key, entry)| !busy.contains(*key) && unfollowed_settled(key, entry))
            .map(|(key, entry)| (entry.revision, key.clone()))
            .collect();
        let excess = observations.len().saturating_sub(OBSERVATIONS_MAX);
        if excess > 0 {
            observations.sort_unstable_by_key(|(revision, _)| *revision);
            for (_, key) in observations.into_iter().take(excess) {
                if let Some(entry) = self.entries.get_mut(&key) {
                    entry.snapshot = None;
                }
            }
        }
        self.entries
            .retain(|key, entry| busy.contains(key) || !forgettable(entry));
        self.forget_oldest_retired(&busy);
        if self.entries.len() != previous_len {
            self.revisions += 1;
        }
        self.next_sweep = self.entries.len().saturating_add(SWEEP_HEADROOM);
        let entries = &self.entries;
        self.locations
            .retain(|id, _| entries.contains_key(&RunKey::Flow(id.clone())));
    }

    fn forget_oldest_retired(&mut self, busy: &HashSet<RunKey>) {
        let mut retired = Vec::new();
        for (key, entry) in &mut self.entries {
            if busy.contains(key) || entry.snapshot.is_some() || entry.end != EndState::Reported {
                entry.retired = None;
                continue;
            }
            let at = *entry.retired.get_or_insert_with(|| {
                self.retirements += 1;
                self.retirements
            });
            retired.push((at, key.clone()));
        }
        let excess = retired.len().saturating_sub(RETIRED_MAX);
        if excess == 0 {
            return;
        }
        retired.sort_unstable_by_key(|(at, _)| *at);
        for (_, key) in retired.into_iter().take(excess) {
            self.entries.remove(&key);
        }
    }
}

/// Settled disk/task facts can be reread; an unreported local flow end cannot.
fn unfollowed_settled(key: &RunKey, entry: &RunEntry) -> bool {
    entry.membership == Membership::Outside
        && entry.end != EndState::Held
        && entry.snapshot.as_ref().is_some_and(|snapshot| {
            !matches!(snapshot.state, RunState::Running { .. })
                && !(matches!(key, RunKey::Flow(_))
                    && entry.disk_episode.is_none()
                    && matches!(snapshot.state, RunState::Ended { .. }))
        })
}

/// Nothing retained, nothing held or reported, and outside the scope.
fn forgettable(entry: &RunEntry) -> bool {
    entry.snapshot.is_none()
        && matches!(entry.end, EndState::Open | EndState::Ended)
        && entry.membership != Membership::Scoped
}

#[cfg(test)]
mod tests;
