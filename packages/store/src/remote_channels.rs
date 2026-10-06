//! Bounded event deduplication history, isolated by profile and connection.

use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

const FILE_NAME: &str = "remote_channels.json";
const EVENTS_PER_CHANNEL: usize = 256;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteChannelState {
    processed: BTreeMap<String, VecDeque<String>>,
}

impl RemoteChannelState {
    pub fn contains(&self, channel: &str, event: &str) -> bool {
        self.processed
            .get(channel)
            .is_some_and(|events| events.iter().any(|id| id == event))
    }

    pub fn record(&mut self, channel: &str, event: String) {
        let events = self.processed.entry(channel.to_owned()).or_default();
        if !events.contains(&event) {
            events.push_back(event);
        }
        while events.len() > EVENTS_PER_CHANNEL {
            events.pop_front();
        }
    }

    pub fn retain_channels(&mut self, ids: &[String]) {
        self.processed.retain(|id, _| ids.contains(id));
    }

    pub fn load_in(dir: &Path) -> Self {
        match load_json_file("remote_channels", &dir.join(FILE_NAME)) {
            LoadOutcome::Parsed(state) => state,
            LoadOutcome::Missing | LoadOutcome::Corrupt => Self::default(),
        }
    }

    pub fn save_in(&self, dir: &Path) -> std::io::Result<()> {
        save_json_atomic(dir, &dir.join(FILE_NAME), self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplication_is_bounded_and_namespaced() {
        let mut state = RemoteChannelState::default();
        for n in 0..=EVENTS_PER_CHANNEL {
            state.record("slack", n.to_string());
        }
        assert!(!state.contains("slack", "0"));
        assert!(state.contains("slack", "1"));
        assert!(!state.contains("discord", "1"));
    }

    #[test]
    fn history_survives_restart_in_its_own_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let mut state = RemoteChannelState::default();
        state.record("work", "event-1".into());
        state.save_in(dir.path()).unwrap();
        assert_eq!(RemoteChannelState::load_in(dir.path()), state);
        assert_eq!(
            RemoteChannelState::load_in(other.path()),
            RemoteChannelState::default()
        );
    }
}
