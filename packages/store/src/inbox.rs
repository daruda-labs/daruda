//! Durable claims on remote commands, recorded before their effects.
//!
//! History belongs to the shared bot stream, not a profile's channel config.
//! Local config removal never emits `ChannelRemoved`; that event is reserved
//! for an explicit external-stream reset. `Initialized` atomically imports all
//! legacy claims and records completion even for empty legacy inputs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use daruda_runlog::Machine;
pub use daruda_runlog::{CommitError, Committed};
use serde::{Deserialize, Serialize};

mod lease;
pub mod migrate;
mod store;

pub use lease::InboxLease;
pub use store::{InboxStore, InboxStoreError};

pub const INBOX_RING_CAPACITY: usize = 256;

/// A transport's stable stream within a bot's shared inbox, never a
/// profile-local channel configuration id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelKey(String);

impl ChannelKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboxId {
    Monotonic(i64),
    Opaque(String),
}

/// An atomic legacy import. Empty imports are still durably initialized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxSeed {
    #[serde(default)]
    pub monotonic: BTreeMap<ChannelKey, i64>,
    #[serde(default)]
    pub rings: BTreeMap<ChannelKey, VecDeque<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxState {
    #[serde(default)]
    initialized: bool,
    #[serde(default)]
    monotonic: BTreeMap<ChannelKey, i64>,
    #[serde(default)]
    rings: BTreeMap<ChannelKey, VecDeque<String>>,
}

impl InboxState {
    pub fn maximum(&self, channel: &ChannelKey) -> Option<i64> {
        self.monotonic.get(channel).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboxEvent {
    Initialized { seed: InboxSeed },
    Seeded { channel: ChannelKey, max: i64 },
    Claimed { channel: ChannelKey, id: InboxId },
    ChannelRemoved { channel: ChannelKey },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InboxRejected {
    #[error("remote command {id:?} was already claimed on {channel:?}")]
    Duplicate { channel: ChannelKey, id: InboxId },
    #[error("remote stream {channel:?} already has history")]
    HasHistory { channel: ChannelKey },
    #[error("inbox initialization requires a pristine, uninitialized inbox")]
    CannotInitialize,
    #[error("an imported ring exceeds its capacity or contains duplicate ids")]
    InvalidRing,
}

pub struct InboxMachine;

impl Machine for InboxMachine {
    type State = InboxState;
    type Event = InboxEvent;
    type Rejected = InboxRejected;

    fn check(state: &InboxState, event: &InboxEvent) -> Result<(), InboxRejected> {
        match event {
            InboxEvent::Initialized { seed } => {
                if state.initialized || !state.monotonic.is_empty() || !state.rings.is_empty() {
                    return Err(InboxRejected::CannotInitialize);
                }
                if seed.rings.values().any(|ring| {
                    ring.len() > INBOX_RING_CAPACITY
                        || ring.iter().collect::<BTreeSet<_>>().len() != ring.len()
                }) {
                    return Err(InboxRejected::InvalidRing);
                }
                Ok(())
            }
            InboxEvent::Seeded { channel, .. } => {
                if state.monotonic.contains_key(channel) || state.rings.contains_key(channel) {
                    Err(InboxRejected::HasHistory {
                        channel: channel.clone(),
                    })
                } else {
                    Ok(())
                }
            }
            InboxEvent::Claimed { channel, id } => {
                let duplicate = match id {
                    InboxId::Monotonic(id) => state.maximum(channel).is_some_and(|max| *id <= max),
                    InboxId::Opaque(id) => state
                        .rings
                        .get(channel)
                        .is_some_and(|ring| ring.contains(id)),
                };
                if duplicate {
                    Err(InboxRejected::Duplicate {
                        channel: channel.clone(),
                        id: id.clone(),
                    })
                } else {
                    Ok(())
                }
            }
            InboxEvent::ChannelRemoved { .. } => Ok(()),
        }
    }

    fn apply(state: &mut InboxState, event: &InboxEvent) {
        match event {
            InboxEvent::Initialized { seed } => {
                state.monotonic.clone_from(&seed.monotonic);
                state.rings.clone_from(&seed.rings);
                state.initialized = true;
            }
            InboxEvent::Seeded { channel, max } => {
                state.monotonic.insert(channel.clone(), *max);
            }
            InboxEvent::Claimed { channel, id } => match id {
                InboxId::Monotonic(id) => {
                    state.monotonic.insert(channel.clone(), *id);
                }
                InboxId::Opaque(id) => {
                    let ring = state.rings.entry(channel.clone()).or_default();
                    ring.push_back(id.clone());
                    while ring.len() > INBOX_RING_CAPACITY {
                        ring.pop_front();
                    }
                }
            },
            InboxEvent::ChannelRemoved { channel } => {
                state.monotonic.remove(channel);
                state.rings.remove(channel);
            }
        }
    }
}

#[cfg(test)]
mod tests;
