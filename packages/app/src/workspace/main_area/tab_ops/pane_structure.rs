//! Pane-tree structure inside a tab: splitting, zooming, closing a pane, and
//! moving panes between tabs.

use super::{NewPaneKind, rebase_tab_history_after_insertion, rebase_tab_history_after_removal};
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::images::release_pane_images;
use crate::workspace::main_area::pane::TabEntry;
use crate::workspace::main_area::pane_tree::{
    PaneId, PaneLayout, SplitDirection, insert_node_at, insert_split_at, remove_pane_from_layout,
};
use gpui::{Context, Window};

impl Workspace {
    pub(in crate::workspace) fn split_from_tab(
        &mut self,
        tab_index: usize,
        kind: NewPaneKind,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .active_runtime()
            .tabs
            .get(tab_index)
            .is_none_or(|tab| self.is_orchestrator_tab(tab))
        {
            return;
        }
        if self.active_runtime().active_tab_index != tab_index {
            self.activate_tab(tab_index, window, cx);
        }
        self.split_focused_pane_kind(kind, direction, window, cx);
    }

    /// Toggle the zoom state for `pane_id`. When zoomed, only that pane is
    /// rendered (full-size); the rest of the split tree is hidden.
    pub(in crate::workspace) fn toggle_zoom_pane(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        // Zoom is meaningless without a real pane. A 0-pane lane (e.g. the
        // inaccessible empty-state) has `focused_pane_id` at its default,
        // which matches no pane — guard so we never stamp
        // `zoomed_pane_id = Some(<bogus id>)` and zoom an empty viewport.
        if !self.active_runtime().panes.iter().any(|p| p.id == pane_id) {
            return;
        }
        if self.main_area.zoomed_pane_id == Some(pane_id) {
            self.main_area.zoomed_pane_id = None;
        } else {
            self.main_area.zoomed_pane_id = Some(pane_id);
        }
        // A zoom change swaps the layout out from under any in-flight drag
        // hover; drop it so a stale half-fill overlay does not linger.
        self.main_area.pane_drop_hover = None;
        // Zoom toggles whether the tab counts as "split" for dimming: a
        // zoomed leaf is rendered alone and must show full color; on
        // unzoom the inactive siblings dim again.
        self.refresh_pane_dimming(cx);
        cx.notify();
    }

    /// The split kind the keyboard split shortcuts (Cmd+D / Cmd+Shift+D) use:
    /// splitting an agent-chat pane spawns another agent chat, everything else
    /// (terminal, file, task) spawns a terminal. The right-click menu offers
    /// the explicit 2×2 matrix; this is only the shortcut default, keyed to the
    /// focused pane's own kind.
    pub(in crate::workspace) fn focused_pane_split_kind(&self) -> NewPaneKind {
        let focused = self.active_runtime().focused_pane_id;
        let is_agent_chat = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused)
            .is_some_and(|p| p.agent_chat_view().is_some());
        if is_agent_chat {
            NewPaneKind::AgentChat
        } else {
            NewPaneKind::Terminal
        }
    }

    /// Split the focused pane, filling the new leaf with `kind`. The layout
    /// tree is content-agnostic (a leaf is just a [`PaneId`]), so terminal and
    /// agent-chat splits share the same `insert_split_at` path; only the pane
    /// constructor and the agent-chat-specific bottom-dock reveal differ.
    pub(in crate::workspace) fn split_focused_pane_kind(
        &mut self,
        kind: NewPaneKind,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Reject an inaccessible lane or no real focused pane: without one the
        // `insert_split_at` loop below matches no tab, orphaning the new pane
        // in `panes` with no `TabEntry`.
        if self.active_lane_is_inaccessible()
            || !self.has_focused_pane()
            || self.is_orchestrator_pane(self.active_runtime().focused_pane_id)
        {
            return;
        }
        let new_pane = match kind {
            NewPaneKind::Terminal => match self.create_pane(window, cx) {
                Ok(p) => p,
                Err(e) => {
                    self.report_pane_error(
                        &crate::surface::strings::error::pane_context_split(),
                        e,
                        cx,
                    );
                    return;
                }
            },
            NewPaneKind::AgentChat => {
                // The session roots at the active lane's cwd (same source as
                // `open_agent_chat_pane`). `local_cwd`/`remote_cwd` are both
                // `None` when there is no active lane; `create_new_agent_chat_pane`
                // then parks the pane in `AgentSessionStatus::Error` rather
                // than connecting.
                let (local_cwd, remote_cwd, session_host) = self.active_lane_cwds();
                // Inherit the focused pane's agent (splitting a `codex` chat
                // opens another `codex`); fall back to the session-sticky
                // default only if its agent id can't be read.
                let focused = self.active_runtime().focused_pane_id;
                let agent_id = self
                    .agent_chat_view(focused)
                    .map(|v| v.read(cx).agent_id().to_owned())
                    .unwrap_or_else(|| self.open_agent_id(None));
                self.create_new_agent_chat_pane(
                    agent_id,
                    local_cwd,
                    remote_cwd,
                    session_host,
                    window,
                    cx,
                )
            }
        };
        let new_pane_id = new_pane.id;
        self.active_runtime_mut().panes.push(new_pane);

        let focused = self.active_runtime().focused_pane_id;
        for tab in &mut self.active_runtime_mut().tabs {
            if insert_split_at(&mut tab.layout, focused, direction, new_pane_id, false) {
                tab.last_focused_pane = new_pane_id;
                break;
            }
        }

        // Agent chat's prompt input lives in the bottom dock; reveal it and
        // mark the pane active before `focus_pane` moves keyboard focus there.
        // Mirrors `open_agent_chat_pane`'s reveal + lazy connect.
        if matches!(kind, NewPaneKind::AgentChat) {
            if !self.docks.bottom.read(cx).is_open {
                self.docks.bottom.update(cx, |d, cx| {
                    d.toggle();
                    cx.notify();
                });
                self.main_area.pending_resize = true;
            }
            self.bump_activity(new_pane_id);
        }

        self.set_focused_pane(new_pane_id, window, cx);
        self.focus_pane(new_pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn close_pane_by_id(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.layout.pane_ids().contains(&pane_id))
        else {
            return;
        };
        let leaf_count = self.active_runtime().tabs[tab_index].layout.leaf_count();
        if leaf_count <= 1 {
            self.close_tab_at(tab_index, window, cx);
            return;
        }

        let next_focus = self.active_runtime().tabs[tab_index]
            .layout
            .prev_pane(pane_id)
            .unwrap_or_else(|| self.active_runtime().tabs[tab_index].layout.first_leaf());

        if self.main_area.zoomed_pane_id == Some(pane_id) {
            self.main_area.zoomed_pane_id = None;
        }
        // A pane removal invalidates any in-flight drop-hover target.
        self.main_area.pane_drop_hover = None;
        remove_pane_from_layout(
            &mut self.active_runtime_mut().tabs[tab_index].layout,
            pane_id,
        );
        self.forget_panes(&[pane_id], cx);
        release_pane_images(&mut self.active_runtime_mut().panes, &[pane_id], window, cx);
        self.active_runtime_mut().panes.retain(|p| p.id != pane_id);

        if tab_index == self.active_runtime().active_tab_index {
            self.set_focused_pane(next_focus, window, cx);
            self.bump_activity(next_focus);
            self.focus_pane(next_focus, window, cx);
        }
        self.active_runtime_mut().tabs[tab_index].last_focused_pane = next_focus;
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Detach `pane_id` out of its current tab's split tree and re-parent it
    /// as the sole leaf of a brand-new tab inserted at `insert_at`. Mirrors
    /// the removal half of `close_pane_by_id` exactly (leaf_count guard,
    /// `prev_pane`/`first_leaf` fallback, `pane_drop_hover` reset,
    /// `remove_pane_from_layout`), but the pane stays alive — it is
    /// re-parented, not destroyed, so `release_pane_tracking` and the
    /// `panes` retain are skipped. Returns `false` when there is nothing to
    /// detach (inaccessible lane, zoomed pane, unknown pane, or a tab that
    /// is already a single leaf).
    pub(in crate::workspace) fn detach_pane_to_new_tab(
        &mut self,
        pane_id: PaneId,
        insert_at: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active_lane_is_inaccessible() || self.main_area.zoomed_pane_id.is_some() {
            return false;
        }
        let Some(tab_index) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.layout.pane_ids().contains(&pane_id))
        else {
            return false;
        };
        if self.active_runtime().tabs[tab_index].layout.leaf_count() <= 1 {
            // Nothing to detach — mirrors close_pane_by_id's leaf_count guard.
            return false;
        }

        let next_focus = self.active_runtime().tabs[tab_index]
            .layout
            .prev_pane(pane_id)
            .unwrap_or_else(|| self.active_runtime().tabs[tab_index].layout.first_leaf());

        self.main_area.pane_drop_hover = None;
        remove_pane_from_layout(
            &mut self.active_runtime_mut().tabs[tab_index].layout,
            pane_id,
        );
        self.active_runtime_mut().tabs[tab_index].last_focused_pane = next_focus;
        if tab_index == self.active_runtime().active_tab_index {
            self.set_focused_pane(next_focus, window, cx);
            self.focus_pane(next_focus, window, cx);
        }

        let new_tab_id = self.alloc_id();
        let insert_at = insert_at.min(self.active_runtime().tabs.len());
        self.active_runtime_mut().tabs.insert(
            insert_at,
            TabEntry {
                id: new_tab_id,
                layout: PaneLayout::Pane(pane_id),
                last_focused_pane: pane_id,
                user_label: None,
            },
        );
        rebase_tab_history_after_insertion(&mut self.active_runtime_mut().tab_history, insert_at);
        if self.active_runtime().active_tab_index >= insert_at {
            self.active_runtime_mut().active_tab_index += 1;
        }

        self.activate_tab(insert_at, window, cx);
        self.resize_all_tabs(window, cx);
        true
    }

    /// Merge `source_tab_id`'s whole tab (its layout subtree, which may
    /// itself be a multi-leaf split) in next to `target_pane_id` inside the
    /// currently-active tab, on the half `direction`/`before` describe.
    /// Returns `false` on a no-op: inaccessible lane, unknown `source_tab_id`,
    /// dragging a tab onto its own (already active) content, or a
    /// `target_pane_id` not present in the active tab.
    ///
    /// Deliberately checks `target_pane_id`'s presence in the active tab's
    /// *live* layout before removing the source tab from `tabs` — mirroring
    /// `rearrange_pane`'s "verify target exists, then mutate" precedent in
    /// `pane_tree.rs` — rather than removing first and restoring on a failed
    /// `insert_node_at`. `insert_node_at` hands back only a `bool`, not the
    /// unconsumed subtree, on failure: if the source tab were removed first,
    /// a failed insert would have already dropped `source.layout` with no
    /// way to reconstruct the removed `TabEntry` for a restore. Checking
    /// containment first, before anything is mutated, makes a vanished
    /// target (the pane closed between hover and drop) a true no-op — and,
    /// because this whole function runs synchronously with nothing able to
    /// mutate the active tab's layout in between, the `insert_node_at` call
    /// below is then guaranteed to succeed.
    pub(in crate::workspace) fn merge_tab_into_pane(
        &mut self,
        source_tab_id: u64,
        target_pane_id: PaneId,
        direction: SplitDirection,
        before: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active_lane_is_inaccessible() {
            return false;
        }
        let Some(src_index) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.id == source_tab_id)
        else {
            return false;
        };
        let active_index = self.active_runtime().active_tab_index;
        if self.is_orchestrator_tab(&self.active_runtime().tabs[src_index])
            || self.is_orchestrator_pane(target_pane_id)
        {
            return false;
        }
        if src_index == active_index {
            return false; // dragged tab is the one currently showing this content
        }
        let Some(active_tab) = self.active_runtime().tabs.get(active_index) else {
            return false;
        };
        if !active_tab.layout.contains(target_pane_id) {
            return false; // target pane closed between hover and drop
        }

        let source = self.active_runtime_mut().tabs.remove(src_index);
        rebase_tab_history_after_removal(&mut self.active_runtime_mut().tab_history, src_index);
        if self.active_runtime().active_tab_index > src_index {
            self.active_runtime_mut().active_tab_index -= 1;
        }
        let active_index = self.active_runtime().active_tab_index;
        let last_focused = source.last_focused_pane;

        let inserted = insert_node_at(
            &mut self.active_runtime_mut().tabs[active_index].layout,
            target_pane_id,
            direction,
            source.layout,
            before,
        );
        debug_assert!(
            inserted,
            "target_pane_id was verified present in the active tab's live layout \
             just above, and nothing between that check and this call can mutate it"
        );
        if !inserted {
            return false;
        }

        self.active_runtime_mut().tabs[active_index].last_focused_pane = last_focused;
        self.set_focused_pane(last_focused, window, cx);
        self.focus_pane(last_focused, window, cx);
        self.resize_all_tabs(window, cx);
        true
    }

    pub(in crate::workspace) fn close_focused_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_pane(self.active_runtime().focused_pane_id, window, cx);
    }
}
