//! Keeps the ledger's per-key memory from growing with every run ever seen.
use super::state::{EndState, Membership, RunEntry, RunWatch};
use crate::runs::result::{RunKey, RunState};
use std::collections::HashSet;

/// Keys past this many drop the settled observations nobody follows; those
/// are re-read from disk or the task log when next asked for. Also the most
/// reported keys remembered after their results were evicted.
pub(super) const ENTRIES_MAX: usize = 1000;

impl RunWatch {
    /// Called where entries are added, so the cap holds without a sweep timer.
    pub(super) fn bound(&mut self) {
        if self.entries.len() > ENTRIES_MAX {
            self.forget_idle();
        }
    }

    /// Forget keys that hold nothing anyone can still receive or ask for, and
    /// the run locations of the flows among them. A key whose end was reported
    /// is kept, up to the cap, so a later read of it is not delivered again.
    pub(super) fn forget_idle(&mut self) {
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
        if self.entries.len() > ENTRIES_MAX {
            for (key, entry) in &mut self.entries {
                if !busy.contains(key) && unfollowed_settled(entry) {
                    entry.snapshot = None;
                }
            }
        }
        self.entries
            .retain(|key, entry| busy.contains(key) || !forgettable(entry));
        self.forget_oldest_retired(&busy);
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
        let excess = retired.len().saturating_sub(ENTRIES_MAX);
        if excess == 0 {
            return;
        }
        retired.sort_unstable_by_key(|(at, _)| *at);
        for (_, key) in retired.into_iter().take(excess) {
            self.entries.remove(&key);
        }
    }
}

/// An observed end no waiter or notice can receive.
fn unfollowed_settled(entry: &RunEntry) -> bool {
    entry.membership == Membership::Outside
        && entry.end != EndState::Held
        && entry
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !matches!(snapshot.state, RunState::Running { .. }))
}

/// Nothing retained, nothing held or reported, and outside the scope. The
/// episode numbers go with it: no event of the key remains to collide with.
fn forgettable(entry: &RunEntry) -> bool {
    entry.snapshot.is_none()
        && matches!(entry.end, EndState::Open | EndState::Ended)
        && entry.membership != Membership::Scoped
}

#[cfg(test)]
mod tests;
