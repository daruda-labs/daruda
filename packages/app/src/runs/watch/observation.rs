//! Disk observations can advance a foreign flow only at a durable resume boundary.
use super::state::RunWatch;
use crate::runs::result::{RunKey, RunSnapshot, RunState};

#[derive(Clone, Copy)]
pub(crate) enum LocalEnd {
    Unconfirmed,
    RecordedThrough(u64),
}

impl LocalEnd {
    pub(crate) fn superseded_by(self, resumed_at: u64) -> bool {
        matches!(self, Self::RecordedThrough(sequence) if resumed_at > sequence)
    }
}

impl RunWatch {
    pub(crate) fn record_flow_sequence(&mut self, key: RunKey, sequence: u64) {
        self.entry(&key).local_sequence = Some(sequence);
    }

    pub(crate) fn local_end(&self, key: &RunKey) -> Option<LocalEnd> {
        let entry = self.entries.get(key)?;
        (entry.disk_episode.is_none()
            && entry
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| matches!(snapshot.state, RunState::Ended { .. })))
        .then(|| {
            entry
                .local_sequence
                .map_or(LocalEnd::Unconfirmed, LocalEnd::RecordedThrough)
        })
    }

    pub(crate) fn observe_disk(&mut self, snapshot: RunSnapshot, episode: u64) {
        let key = snapshot.key.clone();
        let previous = self.entries.get(&key).and_then(|entry| entry.disk_episode);
        if previous.is_some_and(|previous| episode < previous) {
            return;
        }
        let resumed_local = self
            .local_end(&key)
            .is_some_and(|authority| authority.superseded_by(episode));
        if previous.is_some_and(|previous| episode > previous) || resumed_local {
            self.begin(&key);
        }
        if previous == Some(episode)
            && self
                .get(&key)
                .is_some_and(|held| matches!(held.state, RunState::Ended { .. }))
        {
            self.enrich(snapshot);
        } else if matches!(snapshot.state, RunState::Running { .. }) {
            self.observe_waiting(snapshot);
        } else {
            self.retain(snapshot);
        }
        self.entry(&key).disk_episode = Some(episode);
    }
}

#[cfg(test)]
mod tests;
