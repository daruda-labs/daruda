//! Storage is independent of windows and orchestrator sessions.
use super::delivery::{Delivery, WaitId, Waiter};
use crate::runs::result::{FlowRunLocation, RunKey, RunSnapshot, RunState, WaitingOn};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct RunWatch {
    pub(super) scope: HashSet<RunKey>,
    pub(in crate::runs) event_sender: Option<smol::channel::Sender<RunEvent>>,
    pub(super) enrolled: HashSet<RunKey>,
    pub(super) snapshots: HashMap<RunKey, RunSnapshot>,
    locations: HashMap<String, FlowRunLocation>,
    pub(super) disk_episodes: HashMap<RunKey, u64>,
    pub(super) local_sequences: HashMap<RunKey, u64>,
    waiting: HashMap<RunKey, WaitingOn>,
    pub(super) blocked_sequence: HashMap<RunKey, u64>,
    pub(super) events: Vec<RunEvent>,
    next_sequence: HashMap<RunKey, u64>,
    pub(super) waiters: HashMap<WaitId, Waiter>,
    pub(super) next_waiter: u64,
    pub(super) current_floor: HashMap<RunKey, u64>,
    ended: HashSet<RunKey>,
    revisions: HashMap<RunKey, u64>,
    pub(super) terminal_reported: HashSet<RunKey>,
}
/// A process-local episode with a stable per-key identity.
/// The channel may announce it again when waiter coverage is withdrawn.
#[derive(Clone, Debug)]
pub(crate) struct RunEvent {
    pub(crate) sequence: u64,
    pub(crate) snapshot: RunSnapshot,
    pub(super) delivery: Delivery,
}
impl RunWatch {
    /// A resumed flow can end again with the same external run handle.
    pub(crate) fn revision(&self, key: &RunKey) -> u64 {
        self.revisions.get(key).copied().unwrap_or_default()
    }
    fn changed(&mut self, key: &RunKey) {
        *self.revisions.entry(key.clone()).or_default() += 1;
    }
    pub(crate) fn begin(&mut self, key: &RunKey) {
        self.disk_episodes.remove(key);
        self.local_sequences.remove(key);
        self.changed(key);
        self.current_floor.insert(
            key.clone(),
            self.next_sequence.get(key).copied().unwrap_or_default() + 1,
        );
        self.ended.remove(key);
        self.terminal_reported.remove(key);
        self.snapshots.remove(key);
        self.clear_waiting(key);
        self.wake();
    }
    pub(crate) fn waiting_on(&self, key: &RunKey) -> Option<&WaitingOn> {
        self.waiting.get(key)
    }
    #[cfg(test)]
    pub(crate) fn pending(&self, key: &RunKey) -> Option<&RunEvent> {
        self.events
            .iter()
            .rev()
            .find(|event| &event.snapshot.key == key)
    }
    fn clear_waiting(&mut self, key: &RunKey) {
        self.waiting.remove(key);
        self.blocked_sequence.remove(key);
        self.events.retain(|event| {
            &event.snapshot.key != key
                || !matches!(event.snapshot.state, RunState::Running { .. })
                || event.delivery != Delivery::Unreported
        });
    }
    fn push_event(&mut self, snapshot: RunSnapshot) {
        if !self.scope.contains(&snapshot.key) && !self.enrolled.contains(&snapshot.key) {
            return;
        }
        let sequence = self.next_sequence.entry(snapshot.key.clone()).or_default();
        *sequence += 1;
        if matches!(snapshot.state, RunState::Running { .. }) {
            self.blocked_sequence
                .insert(snapshot.key.clone(), *sequence);
        }
        let event = RunEvent {
            sequence: *sequence,
            snapshot,
            delivery: Delivery::Unreported,
        };
        self.events.push(event.clone());
        if let Some(sender) = &self.event_sender {
            // SILENT-OK: the receiver closes only when the app/effects are replaced.
            let _ = sender.try_send(event);
        }
        self.wake();
    }
    pub(crate) fn observe_waiting(&mut self, snapshot: RunSnapshot) {
        let RunState::Running { waiting_on, .. } = &snapshot.state else {
            return;
        };
        let key = &snapshot.key;
        if self.snapshots.get(key) == Some(&snapshot) {
            return;
        }
        self.changed(key);
        match waiting_on {
            Some(waiting) => {
                let was_open = self.waiting.insert(key.clone(), waiting.clone()).is_some();
                if !was_open {
                    self.push_event(snapshot.clone());
                } else if let Some(event) = self.events.iter_mut().rev().find(|event| {
                    &event.snapshot.key == key
                        && event.delivery == Delivery::Unreported
                        && matches!(event.snapshot.state, RunState::Running { .. })
                }) {
                    event.snapshot = snapshot.clone();
                }
            }
            None => self.clear_waiting(key),
        }
        self.snapshots.insert(snapshot.key.clone(), snapshot);
        self.wake();
    }
    pub(crate) fn locate(&mut self, location: FlowRunLocation) {
        if let Some(id) = location.run_dir.file_name().and_then(|id| id.to_str()) {
            self.locations.insert(id.into(), location);
        }
    }
    pub(crate) fn location(&self, id: &str) -> Option<&FlowRunLocation> {
        self.locations.get(id)
    }
    pub(crate) fn register(&mut self, key: RunKey) {
        self.scope.insert(key.clone());
        self.enroll(key);
    }
    pub(super) fn enroll(&mut self, key: RunKey) {
        if self.enrolled.insert(key.clone())
            && let Some(snapshot) = self.snapshots.get(&key).cloned()
            && matches!(
                snapshot.state,
                RunState::Ended { .. }
                    | RunState::Running {
                        waiting_on: Some(_),
                        ..
                    }
            )
        {
            self.push_event(snapshot);
        }
    }
    pub(crate) fn scope(&self) -> Vec<RunKey> {
        let mut keys: Vec<_> = self.scope.iter().cloned().collect();
        keys.sort_by_key(|key| match key {
            RunKey::Flow(id) => format!("flow:{id}"),
            RunKey::Task(exec) => format!("task:{}/{}", exec.task_id, exec.execution_id),
        });
        keys
    }
    pub(crate) fn retained_observation(&self, key: &RunKey) -> Option<(RunSnapshot, u64)> {
        self.get(key)
            .cloned()
            .map(|snapshot| (snapshot, self.revision(key)))
    }
    pub(crate) fn get(&self, key: &RunKey) -> Option<&RunSnapshot> {
        self.snapshots.get(key)
    }
    /// Text enrichment is not another lifecycle event or an outcome change.
    pub(crate) fn enrich(&mut self, snapshot: RunSnapshot) {
        let RunState::Ended {
            text: Some(text), ..
        } = snapshot.state
        else {
            return;
        };
        self.changed(&snapshot.key);
        if let Some(current) = self.snapshots.get_mut(&snapshot.key)
            && let RunState::Ended {
                text: current_text, ..
            } = &mut current.state
        {
            *current_text = Some(text.clone());
        }
        for event in &mut self.events {
            if event.snapshot.key == snapshot.key
                && event.delivery == Delivery::Unreported
                && event.sequence
                    >= self
                        .current_floor
                        .get(&snapshot.key)
                        .copied()
                        .unwrap_or_default()
                && let RunState::Ended {
                    text: current_text, ..
                } = &mut event.snapshot.state
            {
                *current_text = Some(text.clone());
            }
        }
    }
    pub(crate) fn retain(&mut self, snapshot: RunSnapshot) {
        self.disk_episodes.remove(&snapshot.key);
        self.changed(&snapshot.key);
        if matches!(snapshot.state, RunState::Ended { .. }) {
            let already_ended = !self.ended.insert(snapshot.key.clone());
            self.clear_waiting(&snapshot.key);
            if !already_ended {
                self.push_event(snapshot.clone());
            }
        }
        self.snapshots.insert(snapshot.key.clone(), snapshot);
        self.wake();
    }
    pub(super) fn wake(&self) {
        for waiter in self.waiters.values() {
            let _ = waiter.wake.try_send(());
        }
    }
}
#[cfg(test)]
mod tests {}
