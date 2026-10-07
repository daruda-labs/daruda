//! Every seam a test outside the chat module has into the view's
//! internals, listed in one place so the coupling stays visible. A
//! production caller uses `host_surface` instead.

use daruda_acp::ChatItem;
use gpui::ListState;

use super::super::fold::FoldState;
use super::super::pane_choice::PaneChoice;
use super::super::rows::RenderRow;
use super::super::rows::tail::TailWindow;
use super::super::session_config::SessionConfig;
use super::super::transcript_defaults::TranscriptDefaults;
use super::{ActivityTracker, AgentChatView, AgentSessionStatus, AssetCache, ChatContentWidth};
use crate::transcript::display_filter::DisplayFilter;

// Restated here so a per-file scan sees the impl is test-only.
#[cfg(test)]
impl AgentChatView {
    pub(in crate::workspace) fn assets_for_test(&self) -> &AssetCache {
        &self.assets
    }

    pub(in crate::workspace) fn activity_for_test(&self) -> &ActivityTracker {
        &self.activity
    }

    pub(in crate::workspace) fn activity_mut_for_test(&mut self) -> &mut ActivityTracker {
        &mut self.activity
    }

    pub(in crate::workspace) fn items_mut_for_test(&mut self) -> &mut Vec<ChatItem> {
        &mut self.items
    }

    pub(in crate::workspace) fn session_config_mut_for_test(&mut self) -> &mut SessionConfig {
        &mut self.session_config
    }

    pub(in crate::workspace) fn fold_for_test(&self) -> &FoldState {
        &self.fold
    }

    pub(in crate::workspace) fn list_state_for_test(&self) -> &ListState {
        &self.list_state
    }

    pub(in crate::workspace) fn rows_for_test(&self) -> &[RenderRow] {
        &self.rows
    }

    pub(in crate::workspace) fn render_count_for_test(&self) -> u32 {
        self.render_count.get()
    }

    pub(in crate::workspace) fn selection_drag_active_for_test(&self) -> bool {
        self.selection_drag_active
    }

    pub(in crate::workspace) fn defaults_for_test(&self) -> &TranscriptDefaults {
        &self.defaults
    }

    pub(in crate::workspace) fn content_width_for_test(&self) -> ChatContentWidth {
        self.content_width
    }

    pub(in crate::workspace) fn tail_steps_for_test(&self) -> PaneChoice<TailWindow> {
        self.tail_steps
    }

    pub(in crate::workspace) fn tail_calls_for_test(&self) -> PaneChoice<TailWindow> {
        self.tail_calls
    }

    pub(in crate::workspace) fn display_filter_for_test(&self) -> PaneChoice<DisplayFilter> {
        self.display_filter
    }

    pub(in crate::workspace) fn picked_mode_id_for_test(&self) -> Option<&str> {
        self.picked_mode_id.as_deref()
    }

    pub(in crate::workspace) fn picked_model_id_for_test(&self) -> Option<&str> {
        self.picked_model_id.as_deref()
    }

    pub(in crate::workspace) fn set_status_for_test(&mut self, status: AgentSessionStatus) {
        self.status = status;
    }

    pub(in crate::workspace) fn set_items_for_test(&mut self, items: Vec<ChatItem>) {
        self.items = items;
    }

    pub(in crate::workspace) fn set_session_id_for_test(&mut self, id: Option<String>) {
        self.session_id = id;
    }

    pub(in crate::workspace) fn set_agent_id_for_test(&mut self, id: String) {
        self.agent_id = id;
    }

    /// The width a pane lays out at, without marking it the user's choice.
    pub(in crate::workspace) fn set_content_width_for_test(&mut self, width: ChatContentWidth) {
        self.content_width = width;
    }
}
