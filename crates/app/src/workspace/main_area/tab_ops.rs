use gpui::{Context, Window};

use super::nav::{NavDirection, pane_in_direction};
use super::pane::{PaneContent, TabEntry};
use super::pane_tree::{
    PaneId, PaneLayout, SplitDirection, collect_pane_rects, insert_node_at, insert_split_at,
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

/// Whether a tab switch leaves a trail in `tab_history`, which
/// `close_tab_at` pops to choose the next tab. Drag previews and their
/// unwind pass `Skip`: they are visual look-aheads, and recording them
/// would let an abandoned drag redirect a later close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabHistory {
    Record,
    Skip,
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
    /// single site that swaps the shared bottom-dock input draft. The
    /// `.cached()` left dock derives state from the focused pane, so every
    /// focus change must render the workspace to invalidate the dock.
    ///
    /// Takes `&mut Window` because the draft swap calls
    /// `InputState::set_value`, which needs a live window; every caller
    /// already holds the window, so a windowless `cx.update_window` re-entry
    /// would fail with "window not found".
    pub(in crate::workspace) fn set_focused_pane(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Swap the bottom-dock draft only when focus moves to a *different*
        // input-consuming pane (Terminal / AgentChat). The visible text is
        // saved to its owner (`input_owner`, the pane it was typed for), then
        // the incoming pane's draft is restored. Non-input panes leave the
        // text and owner in place. The `input_owner != Some(id)` guard matters
        // because `InputState::set_value` resets selection/scroll even for
        // identical text, which would jump the cursor mid-edit.
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
        // Re-evaluate inactive-pane dim (focused pane goes full color, former
        // sibling dims).
        self.refresh_pane_dimming(cx);
        // Placeholder + panel activation stay on the `focus_pane` path; this
        // method only tracks the focused pane + swaps the draft.
    }

    /// Drop a removed pane's bottom-dock input draft and clear `input_owner`
    /// when it pointed here, so the next focus does not save stale visible
    /// text back to a gone pane. Single site every pane-removal path (close
    /// pane / tab / lane / project) calls, keeping `input_drafts` free of
    /// dead entries.
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

    /// Push per-pane dim onto the active tab's terminal and agent-chat views:
    /// inactive panes blend toward gray, the focused pane stays full color
    /// (both alpha-preserving, so window translucency survives). Only dims a
    /// genuinely split tab (`leaf_count() > 1`) with no zoomed pane. Single
    /// update site for `set_dim_amount` (MVU one-way flow); notifies only the
    /// changed views (Pitfall #10: a `.cached()` view must be dirtied).
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
        if !self.set_menu_target_pane(id, window, cx) {
            return;
        }
        self.bump_activity(id);
        self.focus_pane(id, window, cx);
        cx.notify();
    }

    /// Move the **model** focus to `id` without touching keyboard focus.
    /// Returns `false` when it was already focused (nothing changed).
    ///
    /// This is the part of [`Self::focus_pane_on_click`] a right-click needs:
    /// `split_focused_pane_kind` / `toggle_zoom_pane` / `close_pane_by_id`
    /// all act on `focused_pane_id`, so the menu target must own it. The rest
    /// of the click path — `focus_pane`, which surfaces the bottom dock and
    /// lazily connects an idle Agent chat session — is deliberately excluded:
    /// opening a menu is not activating the pane, and the menu takes keyboard
    /// focus anyway.
    pub(in crate::workspace) fn set_menu_target_pane(
        &mut self,
        id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active_runtime().focused_pane_id == id {
            return false;
        }
        self.set_focused_pane(id, window, cx);
        if let Some(tab) = self.active_tab_mut() {
            tab.last_focused_pane = id;
        }
        true
    }

    pub(in crate::workspace) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // An inaccessible active lane renders the empty-state; spawning a tab
        // would root a PTY at the dead lane path and escape it. No-op. (A
        // workspace with no project at all still allows tabs.)
        if self.active_lane_is_inaccessible() {
            return;
        }
        let pane = match self.create_pane(window, cx) {
            Ok(p) => p,
            Err(e) => {
                self.report_pane_error(&crate::surface::strings::pane_context_new_tab(), e, cx);
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

    /// Total open tabs across every lane runtime in the window (all
    /// projects, all lanes). The window-close decision reads this — not the
    /// active lane's count alone — so closing one lane's last tab never
    /// tears down the window while another lane still holds content.
    pub(in crate::workspace) fn total_open_tabs(&self) -> usize {
        self.main_area
            .runtimes
            .values()
            .map(|rt| rt.tabs.len())
            .sum()
    }

    /// Drop every pane in the active lane's runtime and reset it to an empty
    /// [`LaneRuntime`](crate::workspace::LaneRuntime) (the "no tabs" empty
    /// state). Releases PTY tracking, activity counters, zoom, and drafts for
    /// the removed panes first (same cleanup as `close_tab_at`). The lane
    /// stays in the sidebar.
    pub(in crate::workspace) fn empty_active_lane_runtime(&mut self, cx: &mut Context<Self>) {
        let pane_ids: Vec<PaneId> = self.active_runtime().panes.iter().map(|p| p.id).collect();
        if pane_ids
            .iter()
            .any(|&id| self.main_area.zoomed_pane_id == Some(id))
        {
            self.main_area.zoomed_pane_id = None;
        }
        self.main_area.pane_drop_hover = None;
        self.release_pane_tracking(&pane_ids, cx);
        for id in &pane_ids {
            self.main_area.activity_counter.remove(id);
            self.forget_pane_input_draft(*id);
        }
        *self.active_runtime_mut() = crate::workspace::LaneRuntime::default();
        // Emptying a lane is a durable change that must survive restart, so
        // self-schedule the persist here: interactive close paths reach
        // `close_tab_at` outside any `mutate_durable` wrapper. The `cx.defer`
        // coalescing makes a nested call from a wrapped caller harmless.
        self.mutate_durable(cx, |_, _| {});
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
        // The active lane's last tab. Close the window only when it is also
        // the window's last tab across every lane and project; otherwise empty
        // this lane in place (→ empty-state) and keep the window, so closing
        // one lane's content never tears down others parked in `runtimes`.
        if self.active_runtime().tabs.len() <= 1 {
            if self.total_open_tabs() <= 1 {
                window.remove_window();
                return;
            }
            self.empty_active_lane_runtime(cx);
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
        rebase_tab_history_after_removal(&mut self.active_runtime_mut().tab_history, index);

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

    /// Shared core behind `activate_tab` and `switch_tab_for_drag_preview`:
    /// the guard, zoom/hover-clear, tab-history push + cap, and the
    /// `active_tab_index` write. Returns the new tab's `last_focused_pane`
    /// on a real switch, `None` on a no-op (out-of-range or already
    /// active) — callers branch on that to decide whether to focus/persist.
    fn switch_active_tab_index(&mut self, index: usize, history: TabHistory) -> Option<PaneId> {
        if index >= self.active_runtime().tabs.len()
            || index == self.active_runtime().active_tab_index
        {
            return None;
        }
        self.main_area.zoomed_pane_id = None;
        // Drop any in-flight drag hover so a stale half-fill overlay does
        // not linger on the newly-activated tab. The caller's notify covers it.
        self.main_area.pane_drop_hover = None;
        if history == TabHistory::Record {
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
        }
        self.active_runtime_mut().active_tab_index = index;
        Some(self.active_runtime().tabs[index].last_focused_pane)
    }

    pub(in crate::workspace) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(focused) = self.switch_active_tab_index(index, TabHistory::Record) else {
            return;
        };
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

    /// Tab-bar drag-reorder's hover-to-switch preview: swap the active tab
    /// without focusing the new tab's pane or persisting. No focus, no
    /// persist — the hover timer's `cx.update` re-entry only gives
    /// App-level access, not `&mut Window`, so `set_focused_pane` /
    /// `focus_pane` are unreachable here; and stealing OS keyboard focus
    /// mid-drag would be disruptive anyway. The switch is purely visual
    /// until `drop_tab_onto_bar` commits a real reorder through
    /// `mutate_durable`.
    pub(in crate::workspace) fn switch_tab_for_drag_preview(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        // Stash on the first hop only — later hops must not overwrite it, or
        // an abandoned drag would unwind to the last tab passed over instead
        // of where the drag started.
        let pre_drag = self
            .active_runtime()
            .tabs
            .get(self.active_runtime().active_tab_index)
            .map(|t| t.id);
        if self
            .switch_active_tab_index(index, TabHistory::Skip)
            .is_some()
        {
            if self.main_area.tab_preview_restore.is_none() {
                self.main_area.tab_preview_restore = pre_drag;
            }
            cx.notify();
        }
    }

    /// Re-activate a tab by id, for unwinding a drag preview. `false` when
    /// that tab is gone (closed mid-drag) or already active. Skips history
    /// for the same reason the preview hops do — the unwind is part of the
    /// look-ahead, not navigation the user performed.
    pub(in crate::workspace) fn restore_active_tab_by_id(&mut self, tab_id: u64) -> bool {
        let Some(index) = self
            .active_runtime()
            .tabs
            .iter()
            .position(|t| t.id == tab_id)
        else {
            return false;
        };
        self.switch_active_tab_index(index, TabHistory::Skip)
            .is_some()
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
    // Close Other Tabs / Close Tabs to Right route through
    // `request_close_tabs_bulk`, whose dirty-prompt covers every dirty
    // TaskEdit pane in the closing set. Never loop over `close_tab_at`
    // directly for a multi-tab close — that silently drops unsaved edits.

    pub(in crate::workspace) fn close_other_tabs(
        &mut self,
        tab_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let indices: Vec<usize> = (0..self.active_runtime().tabs.len())
            .rev()
            .filter(|&index| index != tab_index)
            .collect();
        self.request_close_tabs_bulk(indices, window, cx);
    }

    pub(in crate::workspace) fn close_tabs_to_right(
        &mut self,
        tab_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let indices: Vec<usize> = (tab_index + 1..self.active_runtime().tabs.len())
            .rev()
            .collect();
        self.request_close_tabs_bulk(indices, window, cx);
    }

    pub(in crate::workspace) fn move_tab_left(&mut self, tab_index: usize, cx: &mut Context<Self>) {
        if tab_index > 0 {
            self.move_tab(tab_index, tab_index - 1, cx);
        }
    }

    pub(in crate::workspace) fn move_tab_right(
        &mut self,
        tab_index: usize,
        cx: &mut Context<Self>,
    ) {
        if tab_index + 1 < self.active_runtime().tabs.len() {
            self.move_tab(tab_index, tab_index + 1, cx);
        }
    }

    pub(in crate::workspace) fn split_from_tab(
        &mut self,
        tab_index: usize,
        kind: NewPaneKind,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        // Reject an inaccessible lane or no real focused pane: without one the
        // `insert_split_at` loop below matches no tab, orphaning the new pane
        // in `panes` with no `TabEntry`.
        if self.active_lane_is_inaccessible() || !self.has_focused_pane() {
            return;
        }
        let new_pane = match kind {
            NewPaneKind::Terminal => match self.create_pane(window, cx) {
                Ok(p) => p,
                Err(e) => {
                    self.report_pane_error(&crate::surface::strings::pane_context_split(), e, cx);
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
                    .map(|v| v.read(cx).agent_id.clone())
                    .unwrap_or_else(|| {
                        resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref())
                    });
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

    // ---- Dirty-checked close entry points ----

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
                Some((id, pane.title(cx), is_draft))
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
                Some((id, pane.title(cx), is_draft))
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
    /// dirty-prompt before delegating to `close_pane_by_id`.
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
        let title = pane.title(cx);
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

/// Symmetric inverse of the tab_history shift in `close_tab_at`: a new tab
/// pushed into the list at `inserted_index` shifts every later slot up by
/// one, so every history entry `>= inserted_index` must move up by one to
/// keep pointing at the tab it originally referenced.
fn rebase_tab_history_after_insertion(history: &mut [usize], inserted_index: usize) {
    for idx in history {
        if *idx >= inserted_index {
            *idx += 1;
        }
    }
}

/// Shared by `close_tab_at` and `merge_tab_into_pane` — both remove one tab
/// by index. Drops any history entries pointing at the removed index and
/// shifts every higher index down by one so the remainder stay valid.
fn rebase_tab_history_after_removal(history: &mut Vec<usize>, removed_index: usize) {
    history.retain(|&i| i != removed_index);
    for idx in history.iter_mut() {
        if *idx > removed_index {
            *idx -= 1;
        }
    }
}
