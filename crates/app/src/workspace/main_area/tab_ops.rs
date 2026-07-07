use gpui::{Context, Window};

use super::nav::{NavDirection, pane_in_direction};
use super::pane::{PaneContent, TabEntry};
use super::pane_tree::{
    PaneId, PaneLayout, SplitDirection, collect_pane_rects, insert_split_at,
    remove_pane_from_layout,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;

/// What content a newly split-off pane should hold. Keeps the split entry
/// point free of a `bool`/`Option` flag pair (an invalid state would be
/// unrepresentable): each variant maps to exactly one pane constructor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum NewPaneKind {
    Terminal,
    AgentChat,
}

impl Workspace {
    /// Set the user-visible window title (Window > Edit Window Title…).
    /// `None` clears the override so the title falls back to the
    /// auto-derived `"<pane title> — <cwd>"` string at the next render.
    pub(in crate::workspace) fn set_window_label(
        &mut self,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable(cx, |ws, _| {
            ws.window_user_label = label.map(gpui::SharedString::from);
        });
        cx.notify();
    }

    /// Focus-change chokepoint for `main_area.focused_pane_id`, and the
    /// single site that swaps the shared bottom-dock input draft. The left
    /// dock derives `focused_file_selection` and `agent_active_session_id`
    /// from the focused pane and is `.cached()`, so every focus change must
    /// render the workspace; the render staging diff then invalidates the
    /// dock. `restore_from_disk` writes the field directly during initial
    /// construction (`cx.new`) — no prior cache exists at that point, so
    /// no explicit notify is needed.
    ///
    /// Takes `&mut Window` because the draft swap calls
    /// `InputState::set_value`, which needs a live window. A windowless
    /// re-entry via `window_handle` is not an option here: every caller
    /// already holds the window (it is `.take()`n from its slot for the
    /// in-flight update), so `cx.update_window(self.window_handle, …)`
    /// would fail with "window not found" and the restore would silently
    /// no-op (same reason `apply_input_placeholder` exists next to the
    /// windowless `refresh_terminal_input_placeholder`).
    pub(in crate::workspace) fn set_focused_pane(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Swap the bottom-dock draft only when focus moves to a *different*
        // pane that consumes the shared input (Terminal / AgentChat). The
        // visible text is saved to its owner — the pane it was typed for,
        // tracked by `input_owner`, not the outgoing `focused_pane_id` — then
        // the incoming pane's draft is restored (empty when it has none).
        // Non-input panes (File / TaskEdit) leave the text and owner in place:
        // they do not read the bottom input.
        //
        // Re-focusing the pane that already owns the visible text is a no-op:
        // `InputState::set_value` clears the selection and scroll offset even
        // for identical text, so swapping to the same draft would jump the
        // cursor to the start mid-edit (reachable by closing a background tab
        // while typing).
        if self.pane_consumes_bottom_input(id) && self.input_owner != Some(id) {
            let current = self.terminal_input.read(cx).value().to_string();
            if let Some(owner) = self.input_owner {
                if current.is_empty() {
                    self.input_drafts.remove(&owner);
                } else {
                    self.input_drafts.insert(owner, current);
                }
            }
            let incoming = self.input_drafts.get(&id).cloned().unwrap_or_default();
            let terminal_input = self.terminal_input.clone();
            terminal_input.update(cx, |state, cx_state| {
                state.set_value(&incoming, window, cx_state);
            });
            self.input_owner = Some(id);
        }
        self.active_runtime_mut().focused_pane_id = id;
        cx.notify();
        // Re-evaluate inactive-pane dim: the focused pane drops to full
        // color and its former-focused sibling dims (or, after a split /
        // close, the whole tab's dim state is recomputed).
        self.refresh_pane_dimming(cx);
        // The bottom input's placeholder + panel activation stay on the
        // `focus_pane` path (which this is paired with on interactive entry),
        // not here: this method only tracks the focused pane + swaps the draft,
        // keeping the focus-vs-placeholder concerns separated.
    }

    /// Drop a removed pane's bottom-dock input draft and, when it was the
    /// pane whose text is currently shown (`input_owner`), clear that
    /// pointer so the next input-pane focus does not save the stale visible
    /// text back to a pane that no longer exists. The single place every
    /// pane-removal path (close pane / close tab / remove lane / close
    /// project) forgets a draft, so `input_drafts` never keeps entries for
    /// panes that are gone. Empty-string drafts are never stored, so a
    /// missing entry is a no-op `remove`.
    pub(in crate::workspace) fn forget_pane_input_draft(&mut self, pane_id: PaneId) {
        self.input_drafts.remove(&pane_id);
        if self.input_owner == Some(pane_id) {
            self.input_owner = None;
        }
    }

    /// True when `id` names a pane that reads the shared bottom-dock input:
    /// a Terminal (PTY stdin) or an AgentChat (ACP prompt). File and
    /// TaskEdit panes return `false`, so focusing them keeps the visible
    /// draft (and its owner) untouched. Scans the active runtime's panes;
    /// unknown ids yield `false`.
    pub(in crate::workspace) fn pane_consumes_bottom_input(&self, id: PaneId) -> bool {
        self.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == id)
            .is_some_and(|p| {
                matches!(
                    p.content,
                    PaneContent::Terminal(_) | PaneContent::AgentChat(_)
                )
            })
    }

    /// Push per-pane dim onto the active tab's terminal *and* agent-chat views:
    /// inactive panes blend toward gray (iTerm2-style), the focused pane stays
    /// full color. Terminals dim their own cell colors (`set_dim_amount`); agent
    /// chat panes carry the same amount and blend each rendered color through
    /// `AgentChatView::dim` — both alpha-preserving, so the window translucency
    /// survives. Only dims when the active tab is actually split
    /// (`leaf_count() > 1`) and no pane is zoomed; a lone pane (or a zoomed
    /// leaf) is never dimmed. Single update site for either `set_dim_amount` —
    /// the MVU one-way-data-flow rule. Notifies only the views whose amount
    /// changed (Pitfall #10: the `.cached()` view must be dirtied to repaint).
    pub(in crate::workspace) fn refresh_pane_dimming(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
        else {
            return;
        };
        let split = tab.layout.leaf_count() > 1 && self.main_area.zoomed_pane_id.is_none();
        let focused = self.active_runtime().focused_pane_id;
        let pane_ids = tab.layout.pane_ids();

        for pane_id in pane_ids {
            let target = if split && pane_id != focused {
                crate::workspace::render::INACTIVE_PANE_DIM_AMOUNT
            } else {
                0.0
            };
            let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
                continue;
            };
            // Terminal and agent-chat panes are the two dimmable kinds; file /
            // task panes have no dim. Clone the view out so the `panes` borrow
            // ends before `update` re-enters.
            if let Some(view) = pane.terminal_view().cloned() {
                if (view.read(cx).dim_amount() - target).abs() > f32::EPSILON {
                    view.update(cx, |v, cx| {
                        v.set_dim_amount(target);
                        cx.notify();
                    });
                }
            } else if let Some(view) = pane.agent_chat_view().cloned()
                && (view.read(cx).dim_amount - target).abs() > f32::EPSILON
            {
                view.update(cx, |v, cx| {
                    v.set_dim_amount(target);
                    cx.notify();
                });
            }
        }
    }

    /// Click-to-focus body for the pane root's mouse-down listener
    /// (`render_layout` dispatches here one-line, per the MVU rules).
    pub(in crate::workspace) fn focus_pane_on_click(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_runtime().focused_pane_id == id {
            return;
        }
        self.set_focused_pane(id, window, cx);
        if let Some(tab) = self.active_tab_mut() {
            tab.last_focused_pane = id;
        }
        self.bump_activity(id);
        self.focus_pane(id, window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // An inaccessible active lane renders the empty-state; spawning a
        // tab here would root a PTY at the dead lane path (falling back to
        // `$HOME`) and escape that empty-state. No-op — the empty-state's
        // Remove is the only action offered for such a lane. (A workspace
        // with no project at all is not affected — that path still allows
        // tabs.)
        if self.active_lane_is_inaccessible() {
            return;
        }
        let pane = match self.create_pane(window, cx) {
            Ok(p) => p,
            Err(e) => {
                self.report_pane_error("new tab", e, cx);
                return;
            }
        };
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
        self.set_focused_pane(pane_id, window, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn close_tab_at(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.active_runtime().tabs.len() {
            return;
        }
        // Last tab in the active lane → close this window. Other
        // windows (and the app itself under `QuitMode::Default`) stay
        // alive; the user reopens projects via File > New Window /
        // Open… / Open Recent. If the project had other lanes
        // parked in `runtimes`, their PTYs drop with the workspace
        // entity when the window is removed — that's fine for now, a
        // future phase could offer a confirm.
        if self.active_runtime().tabs.len() <= 1 {
            window.remove_window();
            return;
        }

        let tab = self.active_runtime_mut().tabs.remove(index);
        let pane_ids = tab.layout.pane_ids();
        if pane_ids
            .iter()
            .any(|&id| self.main_area.zoomed_pane_id == Some(id))
        {
            self.main_area.zoomed_pane_id = None;
        }
        self.release_pane_tracking(&pane_ids, cx);
        self.active_runtime_mut()
            .panes
            .retain(|p| !pane_ids.contains(&p.id));
        for id in &pane_ids {
            self.main_area.activity_counter.remove(id);
            self.forget_pane_input_draft(*id);
        }

        // Adjust history for the removed tab: drop direct references to it
        // and shift all higher indices down by one so they remain valid.
        self.active_runtime_mut()
            .tab_history
            .retain(|&i| i != index);
        for idx in &mut self.active_runtime_mut().tab_history {
            if *idx > index {
                *idx -= 1;
            }
        }

        // Choose next active tab. Prefer the most-recent history entry;
        // fall back to the nearest neighbor when history is empty.
        let new_active = if self.active_runtime().active_tab_index == index {
            // Pop history to find a valid destination. Entries that fall
            // outside the current tab count (e.g. pointing at a tab that was
            // already closed in the same session) are silently discarded;
            // they can never be navigated to anyway.
            let mut found = None;
            while let Some(prev) = self.active_runtime_mut().tab_history.pop() {
                if prev < self.active_runtime().tabs.len() {
                    found = Some(prev);
                    break;
                }
            }
            found.unwrap_or_else(|| index.min(self.active_runtime().tabs.len() - 1))
        } else if self.active_runtime().active_tab_index > index {
            self.active_runtime().active_tab_index - 1
        } else {
            self.active_runtime().active_tab_index
        };
        self.active_runtime_mut().active_tab_index = new_active;

        if let Some(focused) = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
            .map(|tab| tab.last_focused_pane)
        {
            self.set_focused_pane(focused, window, cx);
            self.bump_activity(focused);
            self.focus_pane(focused, window, cx);
        }
        cx.notify();
    }

    pub(in crate::workspace) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index < self.active_runtime().tabs.len()
            && index != self.active_runtime().active_tab_index
        {
            self.main_area.zoomed_pane_id = None;
            // Drop any in-flight drag hover so a stale half-fill overlay does
            // not linger on the newly-activated tab. The notify below covers it.
            self.main_area.pane_drop_hover = None;
            // Skip consecutive duplicates (A→B→A→B toggling should not fill history).
            if self.active_runtime().tab_history.last()
                != Some(&self.active_runtime().active_tab_index)
            {
                let cur_tab = self.active_runtime().active_tab_index;
                self.active_runtime_mut().tab_history.push(cur_tab);
            }
            // Cap size to bound memory use across long sessions.
            const TAB_HISTORY_CAP: usize = 64;
            if self.active_runtime().tab_history.len() > TAB_HISTORY_CAP {
                let drain_to = self.active_runtime().tab_history.len() - TAB_HISTORY_CAP;
                self.active_runtime_mut().tab_history.drain(..drain_to);
            }
            self.active_runtime_mut().active_tab_index = index;
            let focused = self.active_runtime().tabs[index].last_focused_pane;
            // Switching tabs changes the focused pane — route through the
            // chokepoint so the bottom-dock draft swaps to the new tab's
            // pane. `set_focused_pane` also runs `refresh_pane_dimming`.
            self.set_focused_pane(focused, window, cx);
            self.mutate_durable_in(window, cx, |ws, window, cx| {
                ws.bump_activity(focused);
                ws.focus_pane(focused, window, cx);
            });
            cx.notify();
        }
    }

    pub(in crate::workspace) fn move_tab(
        &mut self,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if from == to
            || from >= self.active_runtime().tabs.len()
            || to >= self.active_runtime().tabs.len()
        {
            return;
        }
        let tab = self.active_runtime_mut().tabs.remove(from);
        self.active_runtime_mut().tabs.insert(to, tab);
        if self.active_runtime().active_tab_index == from {
            self.active_runtime_mut().active_tab_index = to;
        } else {
            // Adjust active index for the shift.
            let a = self.active_runtime().active_tab_index;
            self.active_runtime_mut().active_tab_index = if from < a && to >= a {
                a - 1
            } else if from > a && to <= a {
                a + 1
            } else {
                a
            };
        }
        // Adjust history indices to track the reorder.
        for idx in &mut self.active_runtime_mut().tab_history {
            if *idx == from {
                *idx = to;
            } else if from < to && *idx > from && *idx <= to {
                *idx -= 1;
            } else if from > to && *idx >= to && *idx < from {
                *idx += 1;
            }
        }
        cx.notify();
    }

    pub(in crate::workspace) fn bump_activity(&mut self, pane_id: PaneId) {
        self.main_area.activity_tick += 1;
        self.main_area
            .activity_counter
            .insert(pane_id, self.main_area.activity_tick);
    }

    pub(in crate::workspace) fn focus_next_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.active_runtime().focused_pane_id;
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if let Some(next) = tab.layout.next_pane(focused) {
            tab.last_focused_pane = next;
            self.set_focused_pane(next, window, cx);
            self.focus_pane(next, window, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn focus_prev_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focused = self.active_runtime().focused_pane_id;
        let Some(tab) = self.active_tab_mut() else {
            return;
        };
        if let Some(prev) = tab.layout.prev_pane(focused) {
            tab.last_focused_pane = prev;
            self.set_focused_pane(prev, window, cx);
            self.focus_pane(prev, window, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn focus_pane_in_direction(
        &mut self,
        dir: NavDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
        else {
            return;
        };
        let (w, h) = self.main_area.last_viewport.unwrap_or((1.0, 1.0));
        let mut rects = Vec::new();
        collect_pane_rects(&tab.layout, 0.0, 0.0, w, h, &mut rects);
        if let Some(target) = pane_in_direction(
            &rects,
            self.active_runtime().focused_pane_id,
            dir,
            &self.main_area.activity_counter,
        ) {
            self.set_focused_pane(target, window, cx);
            if let Some(t) = self.active_tab_mut() {
                t.last_focused_pane = target;
            }
            self.bump_activity(target);
            self.focus_pane(target, window, cx);
            cx.notify();
        }
    }

    // ---- Context-menu tab operations ----
    //
    // The bulk close paths (Close Other Tabs / Close Tabs to Right)
    // now route through `request_close_tabs_bulk` so the R-25 batch
    // prompt covers every dirty TaskEdit pane in the closing set.
    // The previous `close_other_tabs` / `close_tabs_to_right` helpers
    // were unconditional loops over `close_tab_at` and silently
    // dropped unsaved edits — they have been removed entirely.

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

    // ---- Split management ----

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
        // Reject when the active lane is inaccessible OR there is no real
        // focused pane (e.g. the empty-state of an inaccessible lane).
        // Without a focused pane the `insert_split_at` loop below would
        // match no tab, leaving the new pane orphaned in `panes` with no
        // `TabEntry` referencing it.
        if self.active_lane_is_inaccessible() || !self.has_focused_pane() {
            return;
        }
        let new_pane = match kind {
            NewPaneKind::Terminal => match self.create_pane(window, cx) {
                Ok(p) => p,
                Err(e) => {
                    self.report_pane_error("split", e, cx);
                    return;
                }
            },
            NewPaneKind::AgentChat => {
                // The session roots at the active lane's cwd (same source as
                // `open_agent_chat_pane`). `cwd` is `None` when there is no
                // active lane; `create_agent_chat_pane` then parks the pane in
                // `AgentSessionStatus::Error` rather than connecting.
                let cwd = self.active_lane().map(|w| w.path.clone());
                // The split was chosen *because* the focused pane is an
                // agent-chat pane, so inherit its agent — splitting a `codex`
                // chat must open another `codex`, not reset to the catalog
                // default. Fall back to the session-sticky default only if the
                // source agent id can't be read.
                let focused = self.active_runtime().focused_pane_id;
                let agent_id = self
                    .agent_chat_view(focused)
                    .map(|v| v.read(cx).agent_id.clone())
                    .unwrap_or_else(|| {
                        resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref())
                    });
                self.create_agent_chat_pane(cwd, None, agent_id, None, window, cx)
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

        // Agent chat's prompt input lives in the bottom dock; reveal it (and
        // mark the pane active) before `focus_pane` activates the input panel
        // and moves keyboard focus there. Mirrors `open_agent_chat_pane`'s
        // bottom-dock reveal + lazy connect (`focus_pane` →
        // `maybe_connect_agent_chat`); the agent, though, is inherited from the
        // source pane above rather than the session default `open_*` uses.
        if matches!(kind, NewPaneKind::AgentChat) {
            if !self.bottom_dock.read(cx).is_open {
                self.bottom_dock.update(cx, |d, cx| {
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
        self.release_pane_tracking(&[pane_id], cx);
        self.active_runtime_mut().panes.retain(|p| p.id != pane_id);
        self.main_area.activity_counter.remove(&pane_id);
        self.forget_pane_input_draft(pane_id);

        if tab_index == self.active_runtime().active_tab_index {
            self.set_focused_pane(next_focus, window, cx);
            self.bump_activity(next_focus);
            self.focus_pane(next_focus, window, cx);
        }
        self.active_runtime_mut().tabs[tab_index].last_focused_pane = next_focus;
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn close_focused_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_pane(self.active_runtime().focused_pane_id, window, cx);
    }

    // ---- Dirty-checked close entry points (R-25) ----

    /// Batch close prompt covering every tab in `indices`. Walks each
    /// tab's panes for `is_dirty` and presents one summary modal.
    /// `indices` must be in descending order.
    pub(in crate::workspace) fn request_close_tabs_bulk(
        &mut self,
        indices: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty: Vec<(PaneId, gpui::SharedString, bool)> = indices
            .iter()
            .filter_map(|&i| self.active_runtime().tabs.get(i))
            .flat_map(|tab| tab.layout.pane_ids().into_iter())
            .filter_map(|id| {
                let pane = self.active_runtime().panes.iter().find(|p| p.id == id)?;
                if !pane.is_dirty(cx) {
                    return None;
                }
                let is_draft = matches!(
                    &pane.content,
                    PaneContent::TaskEditPane(te) if te.task_id.is_none()
                );
                Some((id, pane.title(), is_draft))
            })
            .collect();

        if dirty.is_empty() {
            for i in &indices {
                self.close_tab_at(*i, window, cx);
            }
            return;
        }

        let detail = dirty
            .iter()
            .map(|(_, t, draft)| {
                if *draft {
                    format!("• {} (new task)", t)
                } else {
                    format!("• {}", t)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt_heading = crate::surface::strings::tab_close_batch_heading();
        let prompt_save = crate::surface::strings::tab_close_batch_save_all();
        let prompt_discard = crate::surface::strings::tab_close_batch_discard_all();
        let prompt_cancel = crate::surface::strings::task_edit_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &prompt_heading,
            Some(&detail),
            &[
                prompt_save.as_str(),
                prompt_discard.as_str(),
                prompt_cancel.as_str(),
            ],
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    this.commit_dirty_panes_with_failure_toast(&dirty, cx);
                    for i in &indices {
                        this.close_tab_at(*i, window, cx);
                    }
                }
                1 => {
                    for i in &indices {
                        this.close_tab_at(*i, window, cx);
                    }
                }
                _ => {} // Cancel
            });
        })
        .detach();
    }

    /// Batch close prompt for a whole tab. Walks every dirty pane in
    /// `index` and presents a single 3-button prompt — Save all /
    /// Discard all / Cancel.
    pub(in crate::workspace) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_runtime().tabs.get(index) else {
            return;
        };

        let dirty: Vec<(PaneId, gpui::SharedString, bool)> = tab
            .layout
            .pane_ids()
            .iter()
            .filter_map(|&id| {
                let pane = self.active_runtime().panes.iter().find(|p| p.id == id)?;
                if !pane.is_dirty(cx) {
                    return None;
                }
                let is_draft = matches!(
                    &pane.content,
                    PaneContent::TaskEditPane(te) if te.task_id.is_none()
                );
                Some((id, pane.title(), is_draft))
            })
            .collect();

        if dirty.is_empty() {
            self.close_tab_at(index, window, cx);
            return;
        }

        let detail = dirty
            .iter()
            .map(|(_, t, draft)| {
                if *draft {
                    format!("• {} (new task)", t)
                } else {
                    format!("• {}", t)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt_heading = crate::surface::strings::tab_close_batch_heading();
        let prompt_save = crate::surface::strings::tab_close_batch_save_all();
        let prompt_discard = crate::surface::strings::tab_close_batch_discard_all();
        let prompt_cancel = crate::surface::strings::task_edit_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &prompt_heading,
            Some(&detail),
            &[
                prompt_save.as_str(),
                prompt_discard.as_str(),
                prompt_cancel.as_str(),
            ],
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    this.commit_dirty_panes_with_failure_toast(&dirty, cx);
                    this.close_tab_at(index, window, cx);
                }
                1 => this.close_tab_at(index, window, cx),
                _ => {} // Cancel
            });
        })
        .detach();
    }

    /// Public close entry point that walks pane content through the
    /// R-25 dirty-prompt before delegating to `close_pane_by_id`.
    pub(in crate::workspace) fn request_close_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };

        if !pane.is_dirty(cx) {
            self.close_pane_by_id(pane_id, window, cx);
            return;
        }

        let is_draft = matches!(
            &pane.content,
            PaneContent::TaskEditPane(te) if te.task_id.is_none()
        );
        let title = pane.title();
        let can_save = pane.can_save(cx);

        let heading: String = if is_draft {
            crate::surface::strings::task_edit_discard_draft_prompt().to_string()
        } else {
            format!(
                "{}{}{}",
                crate::surface::strings::task_edit_save_prompt_prefix(),
                title,
                crate::surface::strings::task_edit_save_prompt_suffix(),
            )
        };

        let save_label = if is_draft {
            crate::surface::strings::task_edit_save_draft()
        } else {
            crate::surface::strings::task_edit_save()
        };
        let btn_discard = crate::surface::strings::task_edit_discard();
        let btn_cancel = crate::surface::strings::task_edit_cancel();
        let buttons = [
            save_label.as_str(),
            btn_discard.as_str(),
            btn_cancel.as_str(),
        ];

        let receiver = window.prompt(gpui::PromptLevel::Warning, &heading, None, &buttons, cx);

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                // can_save=false means the form is invalid. Leave the pane open.
                0 if can_save => this.save_task_edit_pane(pane_id, false, window, cx),
                0 => {}
                1 => this.close_pane_by_id(pane_id, window, cx),
                _ => {} // Cancel
            });
        })
        .detach();
    }
}
