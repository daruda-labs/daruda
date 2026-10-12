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
use super::super::rows::tail::TailWindow;
use super::super::transcript_defaults::TranscriptDefaults;
use super::{ActivitySpan, AgentChatView, AgentSessionStatus, TurnOutcome};

// Restated here so a per-file scan sees the impl is test-only.
#[cfg(any(test, feature = "test-support"))]
impl AgentChatView {
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
