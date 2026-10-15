//! Read-only legacy event history, keyed by profile-local connection ids.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;

pub(crate) const FILE_NAME: &str = "remote_channels.json";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteChannelState {
    processed: BTreeMap<String, VecDeque<String>>,
}

impl RemoteChannelState {
    pub(crate) fn events(&self, channel: &str) -> impl Iterator<Item = &str> {
        self.processed
            .get(channel)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    pub fn contains(&self, channel: &str, event: &str) -> bool {
        self.processed
            .get(channel)
            .is_some_and(|events| events.iter().any(|id| id == event))
    }

    pub fn load_in(dir: &Path) -> Self {
        crate::inbox::migrate::read_legacy(&dir.join(FILE_NAME))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_history_retains_its_config_namespaces() {
        let state: RemoteChannelState =
            serde_json::from_slice(br#"{"processed":{"slack":["1"]}}"#).unwrap();
        assert_eq!(state.processed["slack"], VecDeque::from(["1".to_owned()]));
        assert!(!state.contains("discord", "1"));
    }
}
