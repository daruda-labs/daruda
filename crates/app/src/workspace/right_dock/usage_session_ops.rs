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
    /// it isn't already active. The Lane is guaranteed already open (the
    /// Usage tab only lists sessions matching an open Lane's cwd), so this
    /// never needs to open a new Project.
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
        if self.active != session.lane_ref {
            self.activate_lane(session.lane_ref, window, cx);
        }
        // Mirrors `open_agent_chat_pane_with_agent`'s guard: a lane whose
        // directory vanished since this list was built renders the
        // empty-state instead of a broken pane.
        if self.active_lane_is_inaccessible() {
            return;
        }

        let existing = self.active_runtime().panes.iter().find_map(|p| {
            let content = p.agent_chat_content()?;
            (content.view.read(cx).session_id.as_deref() == Some(session.session_id.as_str()))
                .then_some(p.id)
        });
        if let Some(pane_id) = existing {
            self.set_focused_pane(pane_id, window, cx);
            self.bump_activity(pane_id);
            self.focus_pane(pane_id, window, cx);
            cx.notify();
            return;
        }

        let pane = self.create_agent_chat_pane(
            Some(PaneCwd::Local(session.cwd)),
            Some(session.session_id),
            session.agent_id,
            session.title.map(|t| t.to_string()),
            window,
            cx,
        );
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
        if !self.bottom_dock.read(cx).is_open {
            self.bottom_dock.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }
}
