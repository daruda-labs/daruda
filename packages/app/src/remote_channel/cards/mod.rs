use std::collections::HashMap;

use crate::control::approval::ApprovalId;
use crate::remote_channel::bridge::PaneRef;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CardId {
    Approval(ApprovalId),
    FlowAsk {
        run_dir: std::path::PathBuf,
        ask_id: u64,
    },
    Permission {
        pane: PaneRef,
        session: u64,
        request: u64,
    },
}

#[derive(Clone)]
pub(crate) struct SendClaim {
    pub(crate) card: CardId,
    attempt: uuid::Uuid,
}

pub(crate) struct CardLedger<Receipt> {
    cards: HashMap<CardId, Entry<Receipt>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CardCompletion<Receipt> {
    Ignored,
    Delivered,
    Unconfirmed,
    Edit { receipt: Receipt, label: String },
}

struct Entry<Receipt> {
    attempt: uuid::Uuid,
    state: SendState<Receipt>,
}

enum SendState<Receipt> {
    Queued,
    Sending,
    Settled(String),
    Sent(Receipt),
    Attempted,
}

impl<Receipt> Default for CardLedger<Receipt> {
    fn default() -> Self {
        Self {
            cards: HashMap::new(),
        }
    }
}

impl<Receipt> CardLedger<Receipt> {
    pub(crate) fn contains(&self, card: &CardId) -> bool {
        self.cards.contains_key(card)
    }

    pub(crate) fn claim(&mut self, card: CardId) -> Option<SendClaim> {
        if self.contains(&card) {
            return None;
        }
        let attempt = uuid::Uuid::new_v4();
        self.cards.insert(
            card.clone(),
            Entry {
                attempt,
                state: SendState::Queued,
            },
        );
        Some(SendClaim { card, attempt })
    }

    pub(crate) fn begin(&mut self, claim: &SendClaim) -> bool {
        let Some(entry) = self.cards.get_mut(&claim.card) else {
            return false;
        };
        if entry.attempt != claim.attempt || !matches!(entry.state, SendState::Queued) {
            return false;
        }
        entry.state = SendState::Sending;
        true
    }

    pub(crate) fn owns(&self, claim: &SendClaim) -> bool {
        self.cards
            .get(&claim.card)
            .is_some_and(|entry| entry.attempt == claim.attempt)
    }

    #[cfg(test)]
    pub(crate) fn is_sending(&self, claim: &SendClaim) -> bool {
        self.cards.get(&claim.card).is_some_and(|entry| {
            entry.attempt == claim.attempt && matches!(entry.state, SendState::Sending)
        })
    }

    pub(crate) fn release(&mut self, claim: &SendClaim) -> bool {
        if self.owns(claim) {
            self.cards.remove(&claim.card);
            true
        } else {
            false
        }
    }

    pub(crate) fn complete(
        &mut self,
        claim: &SendClaim,
        receipt: Option<Receipt>,
    ) -> CardCompletion<Receipt> {
        if !self.owns(claim) {
            return CardCompletion::Ignored;
        }
        let Some(mut entry) = self.cards.remove(&claim.card) else {
            return CardCompletion::Ignored;
        };
        match entry.state {
            SendState::Settled(label) => match receipt {
                Some(receipt) => CardCompletion::Edit { receipt, label },
                None => CardCompletion::Unconfirmed,
            },
            SendState::Sending => {
                let completion = match receipt {
                    Some(receipt) => {
                        entry.state = SendState::Sent(receipt);
                        CardCompletion::Delivered
                    }
                    None => {
                        entry.state = SendState::Attempted;
                        CardCompletion::Unconfirmed
                    }
                };
                self.cards.insert(claim.card.clone(), entry);
                completion
            }
            _ => {
                self.cards.insert(claim.card.clone(), entry);
                CardCompletion::Ignored
            }
        }
    }

    pub(crate) fn settle(&mut self, card: &CardId, label: String) -> Option<(Receipt, String)> {
        let mut entry = self.cards.remove(card)?;
        match entry.state {
            SendState::Sent(receipt) => Some((receipt, label)),
            SendState::Sending => {
                entry.state = SendState::Settled(label);
                self.cards.insert(card.clone(), entry);
                None
            }
            SendState::Settled(_) => {
                self.cards.insert(card.clone(), entry);
                None
            }
            SendState::Queued | SendState::Attempted => None,
        }
    }

    pub(crate) fn permissions(&self, pane: PaneRef, session: u64) -> Vec<CardId> {
        self.cards.keys().filter(|card| {
            matches!(card, CardId::Permission { pane: held, session: scope, .. } if *held == pane && *scope == session)
        }).cloned().collect()
    }
}

#[cfg(test)]
pub(crate) mod tests;
