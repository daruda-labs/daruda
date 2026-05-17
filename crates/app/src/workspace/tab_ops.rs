use gpui::{Context, Window};

use super::Workspace;
use super::layout::{PaneId, PaneLayout, collect_pane_rects};
use super::nav::{NavDirection, pane_in_direction};
use super::pane::TabEntry;

impl Workspace {
    /// Set the user-visible window title (Window > Edit Window Title…).
    /// `None` clears the override so the title falls back to the
    /// auto-derived `"<pane title> — <cwd>"` string at the next render.
    pub(in crate::workspace) fn set_window_label(
        &mut self,
        label: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.window_user_label = label.map(gpui::SharedString::from);
        self.mark_dirty_and_save(cx);
        cx.notify();
    }

    pub(in crate::workspace) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = match self.create_pane(window, cx) {
            Ok(p) => p,
            Err(e) => {
                self.report_pane_error("new tab", e, cx);
                return;
            }
        };
        let pane_id = pane.id;
        let tab_id = self.alloc_id();

        self.panes.push(pane);
        self.tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });

        self.tab_history.push(self.active_tab_index);
        self.active_tab_index = self.tabs.len() - 1;
        self.focused_pane_id = pane_id;
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
        if index >= self.tabs.len() {
            return;
        }
        // Last tab in the active worktree → close this window. Other
        // windows (and the app itself under `QuitMode::Default`) stay
        // alive; the user reopens projects via File > New Window /
        // Open… / Open Recent. If the project had other worktrees
        // parked in `inactive_worktree_runtimes`, their PTYs drop
        // with the workspace entity when the window is removed —
        // that's fine for now, a future phase could offer a confirm.
        if self.tabs.len() <= 1 {
            window.remove_window();
            return;
        }

        let tab = self.tabs.remove(index);
        let pane_ids = tab.layout.pane_ids();
        if pane_ids.iter().any(|&id| self.zoomed_pane_id == Some(id)) {
            self.zoomed_pane_id = None;
        }
        for id in &pane_ids {
            self.claude.pty_tracker.unregister(*id);
        }
        self.panes.retain(|p| !pane_ids.contains(&p.id));
        for id in &pane_ids {
            self.activity_counter.remove(id);
        }

        // Adjust history for the removed tab: drop direct references to it
        // and shift all higher indices down by one so they remain valid.
        self.tab_history.retain(|&i| i != index);
        for idx in &mut self.tab_history {
            if *idx > index {
                *idx -= 1;
            }
        }

        // Choose next active tab. Prefer the most-recent history entry;
        // fall back to the nearest neighbor when history is empty.
        let new_active = if self.active_tab_index == index {
            // Pop history to find a valid destination. Entries that fall
            // outside the current tab count (e.g. pointing at a tab that was
            // already closed in the same session) are silently discarded;
            // they can never be navigated to anyway.
            let mut found = None;
            while let Some(prev) = self.tab_history.pop() {
                if prev < self.tabs.len() {
                    found = Some(prev);
                    break;
                }
            }
            found.unwrap_or_else(|| index.min(self.tabs.len() - 1))
        } else if self.active_tab_index > index {
            self.active_tab_index - 1
        } else {
            self.active_tab_index
        };
        self.active_tab_index = new_active;

        if let Some(tab) = self.tabs.get(self.active_tab_index) {
            self.focused_pane_id = tab.last_focused_pane;
            self.bump_activity(self.focused_pane_id);
            self.focus_pane(self.focused_pane_id, window, cx);
        }
        cx.notify();
    }

    pub(in crate::workspace) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index < self.tabs.len() && index != self.active_tab_index {
            self.zoomed_pane_id = None;
            // Skip consecutive duplicates (A→B→A→B toggling should not fill history).
            if self.tab_history.last() != Some(&self.active_tab_index) {
                self.tab_history.push(self.active_tab_index);
            }
            // Cap size to bound memory use across long sessions.
            const TAB_HISTORY_CAP: usize = 64;
            if self.tab_history.len() > TAB_HISTORY_CAP {
                self.tab_history
                    .drain(..self.tab_history.len() - TAB_HISTORY_CAP);
            }
            self.active_tab_index = index;
            let focused = self.tabs[index].last_focused_pane;
            self.focused_pane_id = focused;
            self.bump_activity(focused);
            self.focus_pane(focused, window, cx);
            self.mark_dirty_and_save(cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn move_tab(
        &mut self,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        if self.active_tab_index == from {
            self.active_tab_index = to;
        } else {
            // Adjust active index for the shift.
            let a = self.active_tab_index;
            self.active_tab_index = if from < a && to >= a {
                a - 1
            } else if from > a && to <= a {
                a + 1
            } else {
                a
            };
        }
        // Adjust history indices to track the reorder.
        for idx in &mut self.tab_history {
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
        self.activity_tick += 1;
        self.activity_counter.insert(pane_id, self.activity_tick);
    }

    pub(in crate::workspace) fn focus_next_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_index) else {
            return;
        };
        if let Some(next) = tab.layout.next_pane(self.focused_pane_id) {
            self.focused_pane_id = next;
            tab.last_focused_pane = next;
            self.focus_pane(next, window, cx);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn focus_prev_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_index) else {
            return;
        };
        if let Some(prev) = tab.layout.prev_pane(self.focused_pane_id) {
            self.focused_pane_id = prev;
            tab.last_focused_pane = prev;
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
        let Some(tab) = self.tabs.get(self.active_tab_index) else {
            return;
        };
        let (w, h) = self.last_viewport.unwrap_or((1.0, 1.0));
        let mut rects = Vec::new();
        collect_pane_rects(&tab.layout, 0.0, 0.0, w, h, &mut rects);
        if let Some(target) =
            pane_in_direction(&rects, self.focused_pane_id, dir, &self.activity_counter)
        {
            self.focused_pane_id = target;
            if let Some(t) = self.tabs.get_mut(self.active_tab_index) {
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
        if self.zoomed_pane_id == Some(pane_id) {
            self.zoomed_pane_id = None;
        } else {
            self.zoomed_pane_id = Some(pane_id);
        }
        cx.notify();
    }
}
