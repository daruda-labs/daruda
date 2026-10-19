//! Storage is independent of windows and orchestrator sessions.
use super::delivery::{Delivery, WaitId, Waiter};
use crate::runs::result::{FlowRunLocation, RunKey, RunSnapshot, RunState, WaitingOn};
use std::collections::HashMap;

/// Whether the delivery ledger records a key's episodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Membership {
    /// Observed only: no waiter or notice can receive its episodes.
    #[default]
    Outside,
    /// Named by an explicit waiter without joining the omitted-handles scope.
    Enrolled,
    /// Started by the orchestrator in this process.
    Scoped,
}

/// How far the current episode's end has travelled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum EndState {
    #[default]
    Open,
    /// Ended, but announced only once its owner releases the enriched result.
    Held,
    Ended,
    /// The current episode's end was handed to an agent.
    Reported,
}

/// An open block and the ledger episode that announced it, if one was recorded.
#[derive(Clone, Debug)]
pub(super) struct Block {
    on: WaitingOn,
    pub(super) sequence: Option<u64>,
}

/// Everything the ledger knows about one run key.
#[derive(Default)]
pub(super) struct RunEntry {
    pub(super) membership: Membership,
    pub(super) snapshot: Option<RunSnapshot>,
    pub(super) end: EndState,
    pub(super) block: Option<Block>,
    /// Bumped by every lifecycle change so a stale background read loses.
    revision: u64,
    /// The last issued episode number; never reset, so identities stay unique.
    last_sequence: u64,
    /// The first episode number of the current run; lower ones are history.
    pub(super) floor: u64,
    pub(super) disk_episode: Option<u64>,
    pub(super) local_sequence: Option<u64>,
}

impl RunEntry {
    fn changed(&mut self) {
        self.revision += 1;
    }

    pub(super) fn blocked_sequence(&self) -> Option<u64> {
        self.block.as_ref().and_then(|block| block.sequence)
    }
}

/// The token a held end is released with; a later episode makes it stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeldEnd {
    key: RunKey,
    floor: u64,
}

#[derive(Default)]
pub(crate) struct RunWatch {
    pub(super) entries: HashMap<RunKey, RunEntry>,
    pub(in crate::runs) event_sender: Option<smol::channel::Sender<RunEvent>>,
    locations: HashMap<String, FlowRunLocation>,
    pub(super) events: Vec<RunEvent>,
    pub(super) waiters: HashMap<WaitId, Waiter>,
    pub(super) next_waiter: u64,
}
/// A process-local episode with a stable per-key identity.
/// The channel may announce it again when waiter coverage is withdrawn.
#[derive(Clone, Debug)]
pub(crate) struct RunEvent {
    pub(crate) sequence: u64,
    pub(crate) snapshot: RunSnapshot,
    pub(super) delivery: Delivery,
}
#[cfg(test)]
impl RunEvent {
    pub(crate) fn for_test(sequence: u64, snapshot: RunSnapshot) -> Self {
        Self {
            sequence,
            snapshot,
            delivery: Delivery::Unreported,
        }
    }
}
impl RunWatch {
    pub(super) fn entry(&mut self, key: &RunKey) -> &mut RunEntry {
        self.entries.entry(key.clone()).or_default()
    }
    pub(super) fn membership(&self, key: &RunKey) -> Membership {
        self.entries
            .get(key)
            .map(|entry| entry.membership)
            .unwrap_or_default()
    }
    pub(super) fn floor(&self, key: &RunKey) -> u64 {
        self.entries.get(key).map_or(0, |entry| entry.floor)
    }
    /// A resumed flow can end again with the same external run handle.
    pub(crate) fn revision(&self, key: &RunKey) -> u64 {
        self.entries.get(key).map_or(0, |entry| entry.revision)
    }
    pub(crate) fn begin(&mut self, key: &RunKey) {
        let entry = self.entry(key);
        entry.disk_episode = None;
        entry.local_sequence = None;
        entry.changed();
        entry.floor = entry.last_sequence + 1;
        entry.end = EndState::Open;
        entry.snapshot = None;
        self.clear_waiting(key);
        self.wake();
    }
    pub(crate) fn waiting_on(&self, key: &RunKey) -> Option<&WaitingOn> {
        self.entries.get(key)?.block.as_ref().map(|block| &block.on)
    }
    #[cfg(test)]
    pub(crate) fn pending(&self, key: &RunKey) -> Option<&RunEvent> {
        self.events
            .iter()
            .rev()
            .find(|event| &event.snapshot.key == key)
    }
    fn clear_waiting(&mut self, key: &RunKey) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.block = None;
        }
        self.events.retain(|event| {
            &event.snapshot.key != key
                || !matches!(event.snapshot.state, RunState::Running { .. })
                || event.delivery != Delivery::Unreported
        });
    }
    fn push_event(&mut self, snapshot: RunSnapshot) {
        let Some(entry) = self.entries.get_mut(&snapshot.key) else {
            return;
        };
        if entry.membership == Membership::Outside {
            return;
        }
        entry.last_sequence += 1;
        let sequence = entry.last_sequence;
        if let RunState::Running {
            waiting_on: Some(on),
            ..
        } = &snapshot.state
        {
            let block = entry.block.get_or_insert_with(|| Block {
                on: on.clone(),
                sequence: None,
            });
            block.sequence = Some(sequence);
        }
        let event = RunEvent {
            sequence,
            snapshot,
            delivery: Delivery::Unreported,
        };
        self.events.push(event.clone());
        self.announce(event);
        self.wake();
    }
    /// Only scoped episodes can become notices; enrolled ones wait for their waiter.
    pub(super) fn announce(&self, event: RunEvent) {
        if self.membership(&event.snapshot.key) != Membership::Scoped {
            return;
        }
        if let Some(sender) = &self.event_sender {
            // SILENT-OK: the receiver closes only when the app/effects are replaced.
            let _ = sender.try_send(event);
        }
    }
    pub(crate) fn observe_waiting(&mut self, snapshot: RunSnapshot) {
        let RunState::Running { waiting_on, .. } = &snapshot.state else {
            return;
        };
        let key = snapshot.key.clone();
        let entry = self.entries.entry(key.clone()).or_default();
        if entry.snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        entry.changed();
        match waiting_on {
            Some(waiting) => {
                if let Some(block) = &mut entry.block {
                    block.on = waiting.clone();
                    if let Some(event) = self.events.iter_mut().rev().find(|event| {
                        event.snapshot.key == key
                            && event.delivery == Delivery::Unreported
                            && matches!(event.snapshot.state, RunState::Running { .. })
                    }) {
                        event.snapshot = snapshot.clone();
                    }
                } else {
                    entry.block = Some(Block {
                        on: waiting.clone(),
                        sequence: None,
                    });
                    self.push_event(snapshot.clone());
                }
            }
            None => self.clear_waiting(&key),
        }
        self.entry(&key).snapshot = Some(snapshot);
        self.wake();
    }
    pub(crate) fn locate(&mut self, location: FlowRunLocation) {
        if let RunKey::Flow(id) = RunKey::flow_of(&location.run_dir) {
            self.locations.insert(id, location);
        }
    }
    pub(crate) fn location(&self, id: &str) -> Option<&FlowRunLocation> {
        self.locations.get(id)
    }
    pub(crate) fn register(&mut self, key: RunKey) {
        let was = self.membership(&key);
        self.entry(&key).membership = Membership::Scoped;
        if was == Membership::Outside {
            self.announce_cached(&key);
        }
    }
    pub(super) fn enroll(&mut self, key: RunKey) {
        if self.membership(&key) == Membership::Outside {
            self.entry(&key).membership = Membership::Enrolled;
            self.announce_cached(&key);
        }
    }
    /// A key joining the ledger turns its retained, unreported state into an episode.
    fn announce_cached(&mut self, key: &RunKey) {
        let Some(entry) = self.entries.get(key) else {
            return;
        };
        let Some(snapshot) = entry.snapshot.clone() else {
            return;
        };
        let announce = match snapshot.state {
            RunState::Ended { .. } => entry.end == EndState::Ended,
            RunState::Running {
                waiting_on: Some(_),
                ..
            } => true,
            _ => false,
        };
        if announce {
            self.push_event(snapshot);
        }
    }
    /// Leave the ledger once no explicit waiter names the key. Unreported episodes
    /// are dropped: no one can receive them, and a later enrollment recreates them.
    pub(super) fn release_enrollment(&mut self, key: &RunKey) {
        if self.membership(key) != Membership::Enrolled
            || self.waiters.values().any(|waiter| waiter.names(key))
            || self.events.iter().any(|event| {
                &event.snapshot.key == key && matches!(event.delivery, Delivery::InFlight(_))
            })
        {
            return;
        }
        let entry = self.entry(key);
        entry.membership = Membership::Outside;
        if let Some(block) = &mut entry.block {
            block.sequence = None;
        }
        self.events
            .retain(|event| &event.snapshot.key != key || event.delivery != Delivery::Unreported);
    }
    pub(crate) fn scope(&self) -> Vec<RunKey> {
        let mut keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.membership == Membership::Scoped)
            .map(|(key, _)| key.clone())
            .collect();
        keys.sort_by_key(|key| match key {
            RunKey::Flow(id) => format!("flow:{id}"),
            RunKey::Task(exec) => format!("task:{}/{}", exec.task_id, exec.execution_id),
        });
        keys
    }
    pub(crate) fn get(&self, key: &RunKey) -> Option<&RunSnapshot> {
        self.entries.get(key)?.snapshot.as_ref()
    }
    #[cfg(test)]
    pub(super) fn retained_count(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.snapshot.is_some())
            .count()
    }
    /// Text enrichment is not another lifecycle event or an outcome change.
    pub(crate) fn enrich(&mut self, snapshot: RunSnapshot) {
        let RunState::Ended {
            text: Some(text), ..
        } = snapshot.state
        else {
            return;
        };
        let entry = self.entry(&snapshot.key);
        entry.changed();
        let floor = entry.floor;
        if let Some(current) = &mut entry.snapshot
            && let RunState::Ended {
                text: current_text, ..
            } = &mut current.state
        {
            *current_text = Some(text.clone());
        }
        for event in &mut self.events {
            if event.snapshot.key == snapshot.key
                && event.delivery == Delivery::Unreported
                && event.sequence >= floor
                && let RunState::Ended {
                    text: current_text, ..
                } = &mut event.snapshot.state
            {
                *current_text = Some(text.clone());
            }
        }
    }
    /// Retain an observation. An end already held with identical content is
    /// no change, so in-flight reads and waiters are left alone.
    pub(crate) fn retain(&mut self, snapshot: RunSnapshot) {
        let ended = matches!(snapshot.state, RunState::Ended { .. });
        let entry = self.entry(&snapshot.key);
        if ended
            && matches!(entry.end, EndState::Ended | EndState::Reported)
            && entry.snapshot.as_ref() == Some(&snapshot)
        {
            return;
        }
        entry.disk_episode = None;
        entry.changed();
        if ended {
            let first = matches!(entry.end, EndState::Open | EndState::Held);
            if first {
                entry.end = EndState::Ended;
            }
            self.clear_waiting(&snapshot.key);
            if first {
                self.push_event(snapshot.clone());
            }
        }
        let key = snapshot.key.clone();
        self.entry(&key).snapshot = Some(snapshot);
        self.wake();
    }
    pub(super) fn wake(&self) {
        for waiter in self.waiters.values() {
            // SILENT-OK: a full channel already holds a wake-up; a closed one has no waiter.
            let _ = waiter.wake.try_send(());
        }
    }
}
// The flow pump holds a live end while it reads the run's report off-thread.
impl RunWatch {
    /// Retain an end whose announcement waits for [`Self::release_end`], so no
    /// waiter or notice receives it before its owner finishes enriching it.
    /// A later [`Self::retain`] of an end also releases it.
    pub(crate) fn hold_end(&mut self, snapshot: RunSnapshot) -> HeldEnd {
        let key = snapshot.key.clone();
        let entry = self.entry(&key);
        entry.disk_episode = None;
        entry.changed();
        if entry.end == EndState::Open {
            entry.end = EndState::Held;
        }
        let floor = entry.floor;
        entry.snapshot = Some(snapshot);
        self.clear_waiting(&key);
        self.wake();
        HeldEnd { key, floor }
    }
    /// The held end and its release token, while it waits for enrichment.
    pub(crate) fn held_end(&self, key: &RunKey) -> Option<(RunSnapshot, HeldEnd)> {
        let entry = self.entries.get(key)?;
        if entry.end != EndState::Held {
            return None;
        }
        let held = HeldEnd {
            key: key.clone(),
            floor: entry.floor,
        };
        Some((entry.snapshot.clone()?, held))
    }
    /// Announce a held end with its enriched content; stale tokens do nothing.
    pub(crate) fn release_end(&mut self, held: HeldEnd, enriched: RunSnapshot) {
        let entry = self.entry(&held.key);
        if entry.floor != held.floor || entry.end != EndState::Held || enriched.key != held.key {
            return;
        }
        entry.end = EndState::Ended;
        entry.changed();
        entry.snapshot = Some(enriched.clone());
        self.push_event(enriched);
        self.wake();
    }
}
#[cfg(test)]
mod tests;
