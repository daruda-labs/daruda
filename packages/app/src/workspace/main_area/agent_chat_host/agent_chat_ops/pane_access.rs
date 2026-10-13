//! Agent-chat pane accessors: the view, theme parameters, cwd / agent / account
//! fields, and the lane a pane belongs to.

use crate::workspace::Workspace;
use crate::workspace::main_area::pane::Pane;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::AgentChatView;
use daruda_store::project::PaneCwd;
use gpui::{Context, Entity};

impl Workspace {
    /// The (syntax theme, is-light) pair the Markdown/diff reconcilers read.
    /// `is_light` judges the pane's actual paint surface (`agent_chat_bg`),
    /// not the UI theme's light/dark bit, so diffs stay legible even when the
    /// terminal preset and UI theme disagree.
    pub(in crate::workspace::main_area::agent_chat_host) fn agent_chat_theme_params(
        &self,
        cx: &Context<Self>,
    ) -> (String, bool) {
        let is_light = crate::ui::theme::agent_chat_syntax_is_light(cx);
        (self.mirrors.syntax_theme.clone(), is_light)
    }

    /// True when `pane_id` is an Agent chat pane — lets the bottom-dock input
    /// route prompts to the session instead of a PTY.
    pub(in crate::workspace) fn is_agent_chat_pane(&self, pane_id: PaneId) -> bool {
        self.active_runtime()
            .panes
            .iter()
            .any(|p| p.id == pane_id && p.agent_chat_view().is_some())
    }

    /// Look up an AgentChat pane's view entity by id across every lane's
    /// panes, not just the active one — the event pump looks the view up on
    /// every ACP event, and a lane switch only re-points `self.active`; an
    /// active-lane-only lookup would break the pump's loop on a parked lane
    /// and drop that session's responses forever. Pane ids are
    /// workspace-global, so scanning every runtime is unambiguous.
    pub(in crate::workspace) fn agent_chat_view(
        &self,
        pane_id: PaneId,
    ) -> Option<&Entity<AgentChatView>> {
        self.every_agent_chat()
            .find(|(id, _)| *id == pane_id)
            .map(|(_, view)| view)
    }

    /// Keep both the owning slot and any visible wrapper's cwd cache current.
    pub(in crate::workspace::main_area::agent_chat_host) fn update_agent_chat_cwd(
        &mut self,
        pane_id: PaneId,
        cwd: PaneCwd,
    ) {
        if let Some(chat) = self
            .orchestrator
            .chat
            .as_mut()
            .filter(|chat| chat.pane_id == pane_id)
        {
            chat.cwd = Some(cwd.clone());
        }
        if let Some(content) = self
            .main_area
            .pane_mut(pane_id)
            .and_then(Pane::agent_chat_content_mut)
        {
            content.cwd = Some(cwd);
        }
    }

    /// Keep both wrapper caches of `pane_id`'s agent in step with its view —
    /// the one place a live pane is re-pointed at another agent.
    pub(in crate::workspace::main_area::agent_chat_host) fn update_agent_chat_agent_id(
        &mut self,
        pane_id: PaneId,
        agent_id: String,
    ) {
        if let Some(chat) = self
            .orchestrator
            .chat
            .as_mut()
            .filter(|chat| chat.pane_id == pane_id)
        {
            chat.agent_id = agent_id.clone();
        }
        if let Some(content) = self
            .main_area
            .pane_mut(pane_id)
            .and_then(Pane::agent_chat_content_mut)
        {
            content.agent_id = agent_id;
        }
    }

    /// The lane that owns `pane_id`, found by the same cross-lane scan as
    /// [`Self::agent_chat_view`] (a pane's owning lane never changes, but a
    /// lane switch only re-points `self.active`, so this must not be
    /// active-lane-only either). Lets a connect resolve *this* lane's session
    /// host even for a parked pane, rather than trusting a persisted
    /// `PaneCwd::Remote` that carries no host of its own — see
    /// `connect_agent_chat`.
    pub(in crate::workspace) fn lane_ref_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<daruda_store::project::LaneRef> {
        if self.is_orchestrator_pane(pane_id) {
            return None;
        }
        self.main_area
            .runtimes
            .iter()
            .find(|(_, rt)| rt.panes.iter().any(|p| p.id == pane_id))
            .map(|(lane_ref, _)| *lane_ref)
    }

    /// The AgentChat pane's `AccountSelection`. Same cross-lane scan as
    /// [`Self::agent_chat_view`], for the same reason. Falls back to
    /// `SystemDefault` when the pane is gone or isn't an AgentChat pane.
    pub(in crate::workspace) fn agent_chat_account_selection(
        &self,
        pane_id: PaneId,
    ) -> daruda_store::accounts::AccountSelection {
        if let Some(chat) = self
            .orchestrator
            .chat
            .as_ref()
            .filter(|chat| chat.pane_id == pane_id)
        {
            return chat.account;
        }
        self.main_area
            .pane(pane_id)
            .and_then(Pane::agent_chat_content)
            .map(|ac| ac.account)
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault)
    }
}
