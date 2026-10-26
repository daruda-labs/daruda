//! The phone-origin turn ledger and its first-response acknowledgement lifecycle.

use super::super::super::phone_turn::PhoneTurn;
use super::super::{AgentChatView, FirstResponseOutcome, PhoneAckEffect, PromptOrigin};

impl AgentChatView {
    /// Arms the Telegram turn ledger the instant a Telegram-origin prompt
    /// reaches the wire — called from both dispatch paths so it starts at the
    /// true dispatch point, never at enqueue time. No-op for
    /// [`PromptOrigin::InApp`].
    pub(in crate::view) fn arm_phone_turn_if(&mut self, origin: PromptOrigin) {
        if origin == PromptOrigin::Telegram {
            self.phone_turn_state = Some(PhoneTurn::start(
                std::time::Instant::now(),
                self.items.len(),
            ));
        }
    }

    /// Whether this pane is still waiting to produce the first phone-visible
    /// response for a Telegram-origin prompt.
    pub fn is_phone_turn_waiting(&self) -> bool {
        self.phone_turn_state
            .as_ref()
            .is_some_and(PhoneTurn::is_waiting)
    }

    /// Resolve the turn's first response if a completed text reply or first
    /// tool call has appeared since its prompt was echoed, recording it as
    /// sent. `None` when the turn is not the phone's, or nothing qualifying
    /// has arrived yet.
    pub(in crate::view) fn take_phone_first_response(&mut self) -> Option<FirstResponseOutcome> {
        self.phone_turn_state
            .as_mut()?
            .take_first_response(&self.items)
    }

    /// Finish the turn's wait at a terminal boundary. A final streaming text is
    /// resolved first (because `settle_turn` has just finalized it); otherwise
    /// the old immediate "received" ack is used as the fallback.
    pub(in crate::view) fn finish_phone_turn(&mut self) -> PhoneAckEffect {
        if let Some(outcome) = self.take_phone_first_response() {
            return PhoneAckEffect::Relay(outcome);
        }
        if self.answer_phone_turn_without_agent_text() {
            return PhoneAckEffect::Fallback;
        }
        PhoneAckEffect::None
    }

    /// Drop the ledger once a Stop has ended the turn. Nothing further is
    /// owed: the phone's `/stop` was answered on the spot, and a cancel-ack
    /// reports nothing new.
    pub(in crate::view) fn clear_phone_turn(&mut self) {
        self.phone_turn_state = None;
    }

    /// A permission card went to the phone before this fold, so the wait is
    /// answered without agent text — and the ledger stays open: the turn is
    /// still the phone's, and its completion is still owed.
    pub(in crate::view) fn answer_phone_turn_for_permission(&mut self) {
        // Whether this call answered is moot: the card already went out, so
        // no ack follows here either way.
        self.answer_phone_turn_without_agent_text();
    }

    /// End the turn at the completion boundary, where its phone conversation
    /// ends. Returns nothing on purpose: [`Self::phone_turn`] is how the
    /// ledger is read, and handing a value back here is what invites the
    /// read-and-thereby-retire pattern this split removed.
    pub fn end_phone_turn(&mut self) {
        self.phone_turn_state = None;
    }

    /// What the phone has been told, without ending the turn — the completion
    /// relay's question, asked before it knows whether it will send.
    pub fn phone_turn(&self) -> Option<&PhoneTurn> {
        self.phone_turn_state.as_ref()
    }

    /// Record that a waiting turn was answered with something carrying no
    /// agent text (the fixed ack, or a tool note). Returns whether there was a
    /// waiting turn to answer, so the caller sends that ack exactly once.
    fn answer_phone_turn_without_agent_text(&mut self) -> bool {
        self.phone_turn_state
            .as_mut()
            .is_some_and(|turn| turn.answer(None))
    }

    /// Periodic safety net for turns that stay silent for the first-response
    /// window. Returns true only once — the turn moves to answered, so later
    /// flush ticks do not repeat the fallback ack.
    pub fn take_phone_fallback_if_overdue(
        &mut self,
        now: std::time::Instant,
        timeout_secs: u64,
    ) -> bool {
        if !self
            .phone_turn_state
            .as_ref()
            .is_some_and(|turn| turn.is_overdue(now, timeout_secs))
        {
            return false;
        }
        self.answer_phone_turn_without_agent_text()
    }
}

#[cfg(test)]
mod tests;
