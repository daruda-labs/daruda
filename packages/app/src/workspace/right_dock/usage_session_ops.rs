//! Restore a past ACP session (from the Usage tab's recent-sessions table)
//! into a new pane, or focus it if it's already open.

use daruda_store::project::PaneCwd;
use gpui::{Context, Window};

use crate::workspace::Workspace;
use crate::workspace::layout::snap::RestorableSession;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::PaneLayout;

impl Workspace {
    /// Restore `session` into a new pane, switching to its Lane first if
    /// it isn't already active. The Lane is guaranteed already known to this
    /// window (the Usage tab only lists sessions matching the active Lane when
    /// the snapshot was built), so this never needs to open a new Project.
    ///
    /// Focusing the pane triggers `connect_agent_chat`'s lazy ACP
    /// `session/load` resume. Whether the target agent adapter actually
    /// supports resuming isn't knowable ahead of time (no per-agent
    /// capability cache exists before a session connects) — an
    /// unsupported/failed load already falls back to a fresh session with
    /// a `Notice`, the same behavior cold-restore relies on. No special
    /// gating is needed here.
    pub(in crate::workspace) fn restore_session(
        &mut self,
        session: RestorableSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.restore_session_with_flow_origin(session, None, window, cx);
    }

    pub(in crate::workspace) fn restore_session_with_flow_origin(
        &mut self,
        session: RestorableSession,
        origin: Option<crate::workspace::flow_navigation::RunOrigin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active != session.lane_ref {
            self.activate_lane(session.lane_ref, window, cx);
        }
        // Mirrors `open_agent_chat_pane_with_agent`'s guard: a lane whose
        // directory vanished since this list was built renders the
        // empty-state instead of a broken pane.
        if self.active_lane_is_inaccessible() {
            return;
        }

        let exact = origin.is_some();
        let existing = self.active_runtime().panes.iter().find_map(|p| {
            let content = p.agent_chat_content()?;
            let view = content.view.read(cx);
            (view.session_id() == Some(session.session_id.as_str())
                && (!exact
                    || (!view.is_read_only()
                        && content.agent_id == session.agent_id
                        && content.account == session.account
                        && content
                            .cwd
                            .as_ref()
                            .and_then(PaneCwd::as_local)
                            .is_some_and(|cwd| daruda_core::path::same_path(cwd, &session.cwd)))))
            .then_some(p.id)
        });
        if let Some(pane_id) = existing {
            if origin.is_some()
                && let Some(chat) = self
                    .main_area
                    .pane_mut(pane_id)
                    .and_then(|pane| pane.agent_chat_content_mut())
            {
                chat.restore =
                    crate::workspace::main_area::pane::SessionRestore::Exact { flow: origin };
            }
            if let Some(tab) = self
                .active_runtime()
                .tabs
                .iter()
                .position(|tab| tab.layout.contains(pane_id))
            {
                self.activate_tab(tab, window, cx);
            }
            self.set_focused_pane(pane_id, window, cx);
            self.mutate_durable(cx, |_, _| {});
            self.bump_activity(pane_id);
            self.focus_pane(pane_id, window, cx);
            cx.notify();
            return;
        }

        let RestorableSession {
            session_id,
            agent_id,
            account,
            title,
            cwd,
            ..
        } = session;
        let mut pane = self.create_agent_chat_pane(
            Some(PaneCwd::Local(cwd)),
            Some(session_id),
            agent_id,
            title.map(|t| t.to_string()),
            window,
            cx,
        );
        if let Some(content) = pane.agent_chat_content_mut() {
            content.account = account;
            if origin.is_some() {
                content.restore =
                    crate::workspace::main_area::pane::SessionRestore::Exact { flow: origin };
            }
        }
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.active_runtime_mut().panes.push(pane);
        self.active_runtime_mut().tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        let cur_tab = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tab_history.push(cur_tab);
        let last_tab = self.active_runtime().tabs.len() - 1;
        self.active_runtime_mut().active_tab_index = last_tab;
        if !self.docks.bottom.read(cx).is_open {
            self.docks.bottom.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }
}
