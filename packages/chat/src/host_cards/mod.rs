/// A transient request whose wording and identifiers belong to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCard {
    pub id: u64,
    pub summary: String,
    pub actions: Vec<HostCardAction>,
}

/// One host-defined choice, returned unchanged in `AgentChatEvent::HostCardChosen`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCardAction {
    pub id: String,
    pub label: String,
}

pub(crate) enum HostCardStatus {
    Pending,
    Chosen(String),
    Settled(String),
}

impl HostCardStatus {
    pub(crate) fn label(&self) -> Option<&str> {
        match self {
            Self::Pending => None,
            Self::Chosen(label) | Self::Settled(label) => Some(label),
        }
    }
}

pub(crate) struct HostCardEntry {
    pub(crate) card: HostCard,
    pub(crate) status: HostCardStatus,
}

#[derive(Default)]
pub(crate) struct HostCards {
    entries: Vec<HostCardEntry>,
}

impl HostCards {
    pub(crate) fn entries(&self) -> &[HostCardEntry] {
        &self.entries
    }

    pub(crate) fn push(&mut self, card: HostCard) {
        if self.entries.iter().any(|entry| entry.card.id == card.id) {
            return;
        }
        self.entries.push(HostCardEntry {
            card,
            status: HostCardStatus::Pending,
        });
    }

    pub(crate) fn choose(&mut self, id: u64, action_id: &str) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.card.id == id) else {
            return false;
        };
        if !matches!(entry.status, HostCardStatus::Pending) {
            return false;
        }
        let Some(action) = entry
            .card
            .actions
            .iter()
            .find(|action| action.id == action_id)
        else {
            return false;
        };
        entry.status = HostCardStatus::Chosen(action.label.clone());
        true
    }

    pub(crate) fn settle(&mut self, id: u64, result_label: String) {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.card.id == id) else {
            return;
        };
        entry.status = HostCardStatus::Settled(result_label);
        self.entries.retain(|entry| {
            entry.card.id == id || !matches!(entry.status, HostCardStatus::Settled(_))
        });
    }
}

#[cfg(test)]
mod tests;
