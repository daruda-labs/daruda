//! Workspace-owned orchestrator session, independent of worktree tabs.

use std::path::PathBuf;

use daruda_store::accounts::AccountSelection;
use daruda_store::project::PaneCwd;
use gpui::{Context, Entity, Focusable as _, Window};

use super::Workspace;
use super::main_area::agent_chat_pane::view::AgentChatView;
use super::main_area::pane::{PaneContent, TabEntry};
use super::main_area::pane_tree::PaneId;

/// The session's owning handle. Visible panes only clone this view.
pub(in crate::workspace) struct OrchestratorChat {
    pub pane_id: PaneId,
    pub view: Entity<AgentChatView>,
    pub cwd: Option<PaneCwd>,
    pub account: AccountSelection,
    pub agent_id: String,
}

impl Workspace {
    /// What the status bar's orchestrator chip shows, or `None` for no chip
    /// (the feature is off, or another window hosts the session).
    ///
    /// The registry is what makes the last case possible: a second window has
    /// no slot of its own, so reading `orchestrator_chat` alone would have it
    /// claim "not started" about a session that is working.
    pub(in crate::workspace) fn orchestrator_chip_state(
        &self,
        cx: &Context<Self>,
    ) -> Option<super::status_bar::orchestrator_chip::OrchestratorChipState> {
        use super::status_bar::orchestrator_chip::OrchestratorChipState;

        if let Some(chat) = self.orchestrator_chat.as_ref() {
            let view = chat.view.read(cx);
            return Some(OrchestratorChipState::from_activity(
                view.activity_state(),
                &view.status,
            ));
        }
        // From the mirror `apply_config` keeps: `Config::resolved_agents()`
        // deep-clones the catalog, and this runs on every render.
        let config = crate::settings_store::SettingsStore::global(cx).user_arc();
        crate::orchestrator::config::resolve_from(&config.orchestrator, &self.agents)?;
        // By entity id, never by reading the registry's handle back: this runs
        // inside the render of the workspace it may be naming (pitfall 5).
        let hosted_elsewhere = crate::window_registry::WindowRegistry::orchestrator(cx)
            .is_some_and(|(_, host)| host.entity_id() != cx.entity_id());
        (!hosted_elsewhere).then_some(OrchestratorChipState::NotStarted)
    }

    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_orchestrator_for_shot(
        &mut self,
        show_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.insert_orchestrator_chat(
            self.agents[0].id.clone(),
            self.data_dir.join("orchestrator"),
            AccountSelection::SystemDefault,
            None,
            window,
            cx,
        );
        self.agent_chat_view(pane)
            .cloned()
            .unwrap()
            .update(cx, |view, cx| {
                view.seed_transcript(
                    super::main_area::agent_chat_pane::shot_transcript::sample_transcript(),
                    window,
                    cx,
                );
            });
        if show_tab {
            self.show_orchestrator_tab(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn seed_orchestrator_chat_pane(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        let pane = self.insert_orchestrator_chat(agent_id, cwd, account, briefing, window, cx);
        self.maybe_connect_agent_chat(pane, cx);
        pane
    }

    fn insert_orchestrator_chat(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PaneId {
        if let Some(chat) = &self.orchestrator_chat {
            return chat.pane_id;
        }
        let pane = self.create_agent_chat_pane(
            Some(PaneCwd::Local(cwd)),
            None,
            agent_id,
            None,
            window,
            cx,
        );
        let PaneContent::AgentChat(content) = pane.content else {
            unreachable!("the agent chat constructor returns an agent chat");
        };
        if let Some(briefing) = briefing {
            content
                .view
                .update(cx, |view, _| view.set_briefing(briefing));
        }
        self.orchestrator_chat = Some(OrchestratorChat {
            pane_id: pane.id,
            view: content.view,
            cwd: content.cwd,
            account,
            agent_id: content.agent_id,
        });
        cx.notify();
        pane.id
    }

    pub(in crate::workspace) fn is_orchestrator_pane(&self, id: PaneId) -> bool {
        self.orchestrator_chat
            .as_ref()
            .is_some_and(|chat| chat.pane_id == id)
    }

    pub(in crate::workspace) fn is_orchestrator_tab(&self, tab: &TabEntry) -> bool {
        self.orchestrator_chat
            .as_ref()
            .is_some_and(|chat| tab.layout.contains(chat.pane_id))
    }

    pub(in crate::workspace) fn orchestrator_tab_is_visible(&self) -> bool {
        self.main_area
            .runtimes
            .values()
            .any(|rt| rt.tabs.iter().any(|tab| self.is_orchestrator_tab(tab)))
    }

    /// Show the orchestrator's tab, or take it down if it is already up.
    ///
    /// `pub(crate)` for one caller — `orchestrator::start_or_toggle_from_chip`,
    /// which must start a session first and so cannot live on this type.
    pub(crate) fn toggle_orchestrator_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.orchestrator_tab_is_visible() {
            self.hide_orchestrator_tab(window, cx);
        } else if self.orchestrator_chat.is_some() && !self.show_orchestrator_tab(window, cx) {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::orchestrator_tab_unavailable(),
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("orchestrator.tab.unavailable")
                .build(),
                cx,
            );
        }
    }

    pub(in crate::workspace) fn show_orchestrator_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(chat) = self.orchestrator_chat.as_ref() else {
            return false;
        };
        let id = chat.pane_id;
        if self.active_lane_is_inaccessible() {
            return false;
        }
        if !self
            .active_runtime()
            .tabs
            .iter()
            .any(|tab| self.is_orchestrator_tab(tab))
        {
            let (view, cwd, account) = (chat.view.clone(), chat.cwd.clone(), chat.account);
            let agent_id = chat.agent_id.clone();
            self.wrap_existing_agent_chat_pane(id, view, cwd, account, agent_id);
        }
        let index = self
            .active_runtime()
            .tabs
            .iter()
            .position(|tab| self.is_orchestrator_tab(tab))
            .expect("inserted tab");
        self.active_runtime_mut().active_tab_index = index;
        // Remembered, not dropped: hiding restores it.
        self.orchestrator_zoom_to_restore = self.main_area.zoomed_pane_id.take();
        self.main_area.pane_drop_hover = None;
        self.set_focused_pane(id, window, cx);
        if !self.bottom_dock.read(cx).is_open {
            self.bottom_dock.update(cx, |dock, cx| {
                dock.toggle();
                cx.notify();
            });
        }
        self.activate_bottom_input(cx);
        self.apply_input_placeholder(window, cx);
        // Focus the composer directly: displaying the existing session must not reconnect it.
        self.terminal_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        self.main_area.pending_resize = true;
        cx.notify();
        true
    }

    /// Remove only the temporary tab; its view and draft remain owned by the workspace.
    pub(in crate::workspace) fn hide_orchestrator_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_orchestrator_tab(true, window, cx);
    }

    /// Take the orchestrator's tab down because the user is switching
    /// worktrees.
    ///
    /// Focus is deliberately not restored: focusing an idle chat connects it,
    /// and the lane switch refocuses its own pane afterwards anyway.
    pub(in crate::workspace) fn close_orchestrator_tab_for_lane_change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_orchestrator_tab(false, window, cx);
    }

    fn remove_orchestrator_tab(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.orchestrator_chat.as_ref().map(|chat| chat.pane_id) else {
            return;
        };
        let was_focused = self.active_runtime().focused_pane_id == id;
        for rt in self.main_area.runtimes.values_mut() {
            let Some(index) = rt.tabs.iter().position(|tab| tab.layout.contains(id)) else {
                continue;
            };
            rt.tabs.remove(index);
            rt.panes.retain(|pane| pane.id != id);
            super::main_area::tab_ops::rebase_tab_history_after_removal(&mut rt.tab_history, index);
            rt.active_tab_index = if rt.active_tab_index == index {
                rt.tab_history
                    .pop()
                    .filter(|i| *i < rt.tabs.len())
                    .unwrap_or(index)
            } else {
                rt.active_tab_index - usize::from(rt.active_tab_index > index)
            }
            .min(rt.tabs.len().saturating_sub(1));
            if rt.focused_pane_id == id {
                rt.focused_pane_id = rt
                    .tabs
                    .get(rt.active_tab_index)
                    .map_or(0, |tab| tab.last_focused_pane);
            }
        }
        // Put back the zoom the tab stood down, never the tab's own pane.
        self.main_area.zoomed_pane_id = self
            .orchestrator_zoom_to_restore
            .take()
            .filter(|zoomed| *zoomed != id);
        self.main_area.pane_drop_hover = None;
        if was_focused {
            let focused = self.active_runtime().focused_pane_id;
            if restore_focus && self.has_focused_pane() {
                self.set_focused_pane(focused, window, cx);
                self.focus_pane(focused, window, cx);
            } else {
                if self.input_owner == Some(id) {
                    self.input_drafts
                        .insert(id, self.terminal_input.read(cx).value().to_string());
                    self.input_owner = None;
                    self.terminal_input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                if restore_focus {
                    self.focus_handle.focus(window, cx);
                }
            }
        }
        self.main_area.pending_resize = true;
        cx.notify();
    }

    /// Session maintenance includes the hidden slot and never counts its tab twice.
    pub(in crate::workspace) fn every_agent_chat(
        &self,
    ) -> impl Iterator<Item = (PaneId, &Entity<AgentChatView>)> {
        self.lane_agent_chats().chain(
            self.orchestrator_chat
                .iter()
                .map(|chat| (chat.pane_id, &chat.view)),
        )
    }

    /// The cached agent and cwd of the chat `pane_id` names, found over the
    /// same set as [`Self::every_agent_chat`] without touching a view.
    pub(in crate::workspace) fn agent_chat_identity(
        &self,
        pane_id: PaneId,
    ) -> Option<(&str, Option<&PaneCwd>)> {
        let lane = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter(|pane| pane.id == pane_id)
            .find_map(|pane| pane.agent_chat_content())
            .map(|content| (content.agent_id.as_str(), content.cwd.as_ref()));
        lane.or_else(|| {
            self.orchestrator_chat
                .as_ref()
                .filter(|chat| chat.pane_id == pane_id)
                .map(|chat| (chat.agent_id.as_str(), chat.cwd.as_ref()))
        })
    }

    /// Worktree badges and listings exclude the orchestrator, even while visible.
    pub(in crate::workspace) fn lane_agent_chats(
        &self,
    ) -> impl Iterator<Item = (PaneId, &Entity<AgentChatView>)> {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter(|pane| !self.is_orchestrator_pane(pane.id))
            .filter_map(|pane| pane.agent_chat_view().map(|view| (pane.id, view)))
    }

    pub(crate) fn orchestrator_chat_pane(&self) -> Option<crate::telegram::bridge::PaneRef> {
        Some(crate::telegram::bridge::PaneRef {
            workspace: self.uuid(),
            pane: self.orchestrator_chat.as_ref()?.pane_id,
        })
    }

    #[cfg(test)]
    pub(crate) fn seed_orchestrator_chat_pane_unrevealed_for_test(
        &mut self,
        agent_id: String,
        cwd: PathBuf,
        account: AccountSelection,
        briefing: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        Some(self.insert_orchestrator_chat(agent_id, cwd, account, briefing, window, cx))
    }
}

#[cfg(test)]
#[path = "tests/orchestrator_hosting.rs"]
mod tests;
