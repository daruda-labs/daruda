//! The seams the app's own tests drive a pane through, listed in one place so
//! the coupling stays visible. Each takes or returns plain values: a host test
//! sets up or observes a state, it never holds the view's internals. A
//! production caller uses `host_surface` instead; the chat's own tests reach
//! the fields directly.

use daruda_acp::{ChatItem, PermissionItem};
use daruda_transcript::display_filter::DisplayFilter;
use daruda_transcript::fold_mode::FoldMode;
use gpui::Context;

use super::super::pane_choice::PaneChoice;
use super::super::phone_turn::PhoneTurn;
use super::super::rows::tail::TailWindow;
use super::super::transcript_defaults::TranscriptDefaults;
use super::{
    ActivitySpan, AgentChatView, AgentSessionStatus, PromptId, PromptOrigin, QueuedPrompt,
    TurnOutcome,
};

// Restated here so a per-file scan sees the impl is test-only.
#[cfg(any(test, feature = "test-support"))]
impl AgentChatView {
    /// Test-only hook: mark a prompt turn in flight (as `send_prompt_text`
    /// does) — `Turn` is module-private, so tests drive it through this.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_turn_in_flight(&mut self) {
        self.queue.turn.start(std::time::Instant::now());
    }

    /// Test-only hook: return the turn to idle (as `settle_turn` does).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_turn_idle(&mut self) {
        self.queue.turn.finish();
    }

    /// Test-only hook: whether the turn is idle (no prompt in flight).
    #[cfg(any(test, feature = "test-support"))]
    pub fn turn_is_idle(&self) -> bool {
        !self.queue.turn.is_in_flight()
    }

    /// Test-only hook: whether a Stop's cancel window is still open.
    #[cfg(test)]
    pub(super) fn turn_awaiting_cancel_ack(&self) -> bool {
        self.queue.turn.awaiting_cancel_ack()
    }

    /// Test-only hook: arm the phone turn without needing a live ACP handle.
    #[cfg(any(test, feature = "test-support"))]
    pub fn start_phone_turn_for_test(&mut self, started_at: std::time::Instant) {
        self.phone_turn_state = Some(PhoneTurn::start(started_at, self.items.len()));
    }

    /// Test-only hook: resolve the turn's first response exactly as the relay
    /// path does, so a test can reach `Answered` without an ACP session.
    /// Returns whether anything resolved.
    #[cfg(any(test, feature = "test-support"))]
    pub fn take_phone_first_response_for_test(&mut self) -> bool {
        self.take_phone_first_response().is_some()
    }

    /// Push `depth` placeholder prompts, for the queue-cap tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fill_queue_for_test(&mut self, depth: usize) {
        while self.queued_prompt_count() < depth {
            let id = PromptId(self.queue.next_prompt_id);
            self.queue.next_prompt_id += 1;
            self.queue.pending_prompts.push(QueuedPrompt {
                id,
                text: String::new(),
                attachments: Vec::new(),
                origin: PromptOrigin::InApp,
                task: None,
            });
        }
    }

    pub fn host_cards_for_test(&self) -> Vec<(u64, Option<String>)> {
        self.host_cards
            .entries()
            .iter()
            .map(|entry| (entry.card.id, entry.status.label().map(str::to_owned)))
            .collect()
    }

    /// How many times the view has rendered — what a cache-scoping test
    /// counts to tell a targeted notify from a missed one.
    pub fn render_count_for_test(&self) -> u32 {
        self.render_count.get()
    }

    /// The defaults the pane was last seeded with.
    pub fn defaults_for_test(&self) -> TranscriptDefaults {
        self.defaults
    }

    pub fn tail_steps_for_test(&self) -> PaneChoice<TailWindow> {
        self.tail_steps
    }

    pub fn tail_calls_for_test(&self) -> PaneChoice<TailWindow> {
        self.tail_calls
    }

    pub fn display_filter_for_test(&self) -> PaneChoice<DisplayFilter> {
        self.display_filter
    }

    pub fn fold_mode_for_test(&self) -> PaneChoice<FoldMode> {
        self.fold.mode_choice()
    }

    /// How many rows are on screen, folded-away ones left out.
    pub fn visible_row_count_for_test(&self) -> usize {
        self.rows.iter().filter(|row| !row.hidden).count()
    }

    pub fn set_status_for_test(&mut self, status: AgentSessionStatus) {
        self.status = status;
    }

    pub fn set_session_id_for_test(&mut self, id: Option<String>) {
        self.session_id = id;
    }

    pub fn set_agent_id_for_test(&mut self, id: String) {
        self.agent_id = id;
    }

    /// A run that has ended with `outcome` but not yet settled: the state
    /// the next activity tick turns into one completion.
    pub fn owe_completion_for_test(&mut self, outcome: TurnOutcome) {
        self.activity.span = ActivitySpan::Busy {
            started_at: std::time::Instant::now(),
        };
        self.activity.pending_completion = Some(outcome);
    }

    /// When the subagent under `parent` last reported — backdate it to stand
    /// in for a quiescence window that has run out.
    pub fn set_subagent_activity_for_test(&mut self, parent: &str, at: std::time::Instant) {
        self.activity
            .subagent_last_activity
            .insert(parent.to_owned(), at);
    }

    /// Whether a completion is still waiting for the pane to settle.
    pub fn owes_completion_for_test(&self) -> bool {
        self.activity.pending_completion.is_some()
    }

    /// Put an unanswered permission card on screen, as an agent asking would.
    pub fn hold_permission_card_for_test(&mut self, card: PermissionItem, cx: &mut Context<Self>) {
        self.pending_permissions.insert(card.id);
        self.seed_items_for_test([ChatItem::Permission(card)], cx);
    }

    /// How many permission requests the phone has been told about and that
    /// are still outstanding.
    pub fn permissions_told_to_phone_count_for_test(&self) -> usize {
        self.permissions_told_to_phone.len()
    }
}
