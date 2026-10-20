use std::collections::{HashMap, VecDeque};

use crate::control::approval::ApprovalId;
use crate::remote_channel::bridge::PaneRef;

mod edits;
pub(crate) use edits::{EditQueue, RETRY_INTERVAL};

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

    fn cards(&self) -> impl Iterator<Item = &CardId> {
        self.cards.keys()
    }

    pub(crate) fn permissions(&self, pane: PaneRef, session: u64) -> Vec<CardId> {
        self.cards.keys().filter(|card| {
            matches!(card, CardId::Permission { pane: held, session: scope, .. } if *held == pane && *scope == session)
        }).cloned().collect()
    }
}

/// Settled messages remembered for tap-time cleanup. Enough to cover taps
/// racing a settlement edit, which arrive within seconds.
const SETTLED_MESSAGES_CAP: usize = 64;

/// One remote identity's cards: the ledger, and the identity whose messages its
/// receipts name. The identity is what can edit those messages and receive taps
/// on them (a bot and its recipient), not the session instance serving it, so a
/// reconnect, inbox recovery or credential rewrite with the same bot keeps
/// every record.
pub(crate) struct OwnedCards<Owner, Receipt, Message> {
    owner: Option<Owner>,
    ledger: CardLedger<Receipt>,
    /// Messages a settlement edit has taken over. A stale tap on one of these
    /// must not rewrite the settled result with a competing label.
    settled: VecDeque<Message>,
    pub(crate) edits: EditQueue<Receipt>,
}

impl<Owner, Receipt, Message> Default for OwnedCards<Owner, Receipt, Message> {
    fn default() -> Self {
        Self {
            owner: None,
            ledger: CardLedger::default(),
            settled: VecDeque::new(),
            edits: EditQueue::default(),
        }
    }
}

impl<Owner: PartialEq, Receipt, Message: PartialEq> OwnedCards<Owner, Receipt, Message> {
    /// Serve cards as `owner`. The same identity keeps every record. Another
    /// one can neither edit the old messages nor receive their taps, so the
    /// records are dropped and returned: the caller forgets their callback
    /// tokens, and a still-waiting request can be offered once to the new
    /// identity.
    pub(crate) fn bind(&mut self, owner: Owner) -> Vec<CardId> {
        if self.owner.as_ref() == Some(&owner) {
            return Vec::new();
        }
        let forgotten = self.ledger.cards().cloned().collect();
        *self = Self {
            owner: Some(owner),
            ..Self::default()
        };
        forgotten
    }

    /// End the identity: nobody serves these cards any more, so their records
    /// are dropped and returned for the caller to forget their tokens.
    pub(crate) fn retire(&mut self) -> Vec<CardId> {
        let forgotten = self.ledger.cards().cloned().collect();
        *self = Self::default();
        forgotten
    }

    pub(crate) fn owner(&self) -> Option<&Owner> {
        self.owner.as_ref()
    }

    /// The ledger, only while it belongs to `owner`.
    pub(crate) fn ledger_for(&mut self, owner: &Owner) -> Option<&mut CardLedger<Receipt>> {
        (self.owner.as_ref() == Some(owner)).then_some(&mut self.ledger)
    }

    pub(crate) fn ledger(&self) -> &CardLedger<Receipt> {
        &self.ledger
    }

    pub(crate) fn ledger_mut(&mut self) -> &mut CardLedger<Receipt> {
        &mut self.ledger
    }

    /// Record that a settlement edit owns `message`.
    pub(crate) fn note_settled(&mut self, message: Message) {
        if self.settled.contains(&message) {
            return;
        }
        self.settled.push_back(message);
        if self.settled.len() > SETTLED_MESSAGES_CAP {
            self.settled.pop_front();
        }
    }

    /// Whether a tap on `message` may strip its buttons: no settlement edit
    /// owns it, so nothing else will ever take the dead buttons off.
    pub(crate) fn untracked(&self, message: &Message) -> bool {
        !self.settled.contains(message)
    }
}

#[cfg(test)]
pub(crate) mod tests;
