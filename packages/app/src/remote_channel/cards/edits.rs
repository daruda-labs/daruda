//! Bounded settlement work retained until an identity can edit its messages.

use std::collections::VecDeque;
use std::time::Duration;
use uuid::Uuid;

pub(crate) const RETRY_INTERVAL: Duration = Duration::from_secs(15);
const PENDING_EDITS_CAP: usize = 64;

pub(crate) struct EditQueue<Receipt> {
    entries: VecDeque<Entry<Receipt>>,
}

struct Entry<Receipt> {
    id: Uuid,
    receipt: Receipt,
    label: String,
    state: State,
}

enum State {
    Pending,
    Editing,
}

impl<Receipt> Default for EditQueue<Receipt> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }
}

impl<Receipt: Clone> EditQueue<Receipt> {
    pub(crate) fn push(&mut self, receipt: Receipt, label: String) {
        self.entries.push_back(Entry {
            id: Uuid::new_v4(),
            receipt,
            label,
            state: State::Pending,
        });
        if self.entries.len() > PENDING_EDITS_CAP {
            self.entries.pop_front();
        }
    }

    pub(crate) fn claim(&mut self) -> Vec<(Uuid, Receipt, String)> {
        self.entries
            .iter_mut()
            .filter_map(|entry| {
                if matches!(entry.state, State::Editing) {
                    return None;
                }
                entry.state = State::Editing;
                Some((entry.id, entry.receipt.clone(), entry.label.clone()))
            })
            .collect()
    }

    pub(crate) fn contains(&self, id: Uuid) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    pub(crate) fn finish(&mut self, id: Uuid, edited: bool) {
        if edited {
            self.entries.retain(|entry| entry.id != id);
        } else if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
            entry.state = State::Pending;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_work_is_retried_once_and_success_releases_it() {
        let mut queue = EditQueue::default();
        queue.push("message", "Approved".into());
        let (id, receipt, label) = queue.claim().pop().unwrap();
        assert_eq!((receipt, label.as_str()), ("message", "Approved"));
        assert!(queue.claim().is_empty());
        queue.finish(id, false);
        assert_eq!(queue.claim().len(), 1);
        queue.finish(id, true);
        assert!(queue.claim().is_empty());
    }

    #[test]
    fn old_completions_cannot_remove_replacement_work_and_storage_is_bounded() {
        let mut queue = EditQueue::default();
        queue.push(0, "Old".into());
        let old = queue.claim()[0].0;
        for message in 1..=PENDING_EDITS_CAP {
            queue.push(message, "New".into());
        }
        queue.finish(old, true);
        assert!(!queue.contains(old));
        assert_eq!(queue.claim().len(), PENDING_EDITS_CAP);
    }
}
