use gpui::{Context, Window};

use crate::workspace::Workspace;
use super::pane_tree::{PaneId, PaneLayout, SplitDirection, collect_pane_rects, insert_split_at, remove_pane_from_layout};
use super::nav::{NavDirection, pane_in_direction};
use super::pane::{PaneContent, TabEntry};

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

        self.main_area.panes.push(pane);
        self.main_area.tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });

        self.main_area.tab_history.push(self.main_area.active_tab_index);
        self.main_area.active_tab_index = self.main_area.tabs.len() - 1;
        self.main_area.focused_pane_id = pane_id;
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
        if index >= self.main_area.tabs.len() {
            return;
        }
        // Last tab in the active worktree → close this window. Other
        // windows (and the app itself under `QuitMode::Default`) stay
        // alive; the user reopens projects via File > New Window /
        // Open… / Open Recent. If the project had other worktrees
        // parked in `inactive_worktree_runtimes`, their PTYs drop
        // with the workspace entity when the window is removed —
        // that's fine for now, a future phase could offer a confirm.
        if self.main_area.tabs.len() <= 1 {
            window.remove_window();
            return;
        }

        let tab = self.main_area.tabs.remove(index);
        let pane_ids = tab.layout.pane_ids();
        if pane_ids.iter().any(|&id| self.main_area.zoomed_pane_id == Some(id)) {
            self.main_area.zoomed_pane_id = None;
        }
        for id in &pane_ids {
            self.claude.pty_tracker.unregister(*id);
        }
        self.main_area.panes.retain(|p| !pane_ids.contains(&p.id));
        for id in &pane_ids {
            self.main_area.activity_counter.remove(id);
        }

        // Adjust history for the removed tab: drop direct references to it
        // and shift all higher indices down by one so they remain valid.
        self.main_area.tab_history.retain(|&i| i != index);
        for idx in &mut self.main_area.tab_history {
            if *idx > index {
                *idx -= 1;
            }
        }

        // Choose next active tab. Prefer the most-recent history entry;
        // fall back to the nearest neighbor when history is empty.
        let new_active = if self.main_area.active_tab_index == index {
            // Pop history to find a valid destination. Entries that fall
            // outside the current tab count (e.g. pointing at a tab that was
            // already closed in the same session) are silently discarded;
            // they can never be navigated to anyway.
            let mut found = None;
            while let Some(prev) = self.main_area.tab_history.pop() {
                if prev < self.main_area.tabs.len() {
                    found = Some(prev);
                    break;
                }
            }
            found.unwrap_or_else(|| index.min(self.main_area.tabs.len() - 1))
        } else if self.main_area.active_tab_index > index {
            self.main_area.active_tab_index - 1
        } else {
            self.main_area.active_tab_index
        };
        self.main_area.active_tab_index = new_active;

        if let Some(tab) = self.main_area.tabs.get(self.main_area.active_tab_index) {
            self.main_area.focused_pane_id = tab.last_focused_pane;
            self.bump_activity(self.main_area.focused_pane_id);
            self.focus_pane(self.main_area.focused_pane_id, window, cx);
        }
        cx.notify();
    }

    pub(in crate::workspace) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index < self.main_area.tabs.len() && index != self.main_area.active_tab_index {
            self.main_area.zoomed_pane_id = None;
            // Skip consecutive duplicates (A→B→A→B toggling should not fill history).
            if self.main_area.tab_history.last() != Some(&self.main_area.active_tab_index) {
                self.main_area.tab_history.push(self.main_area.active_tab_index);
            }
            // Cap size to bound memory use across long sessions.
            const TAB_HISTORY_CAP: usize = 64;
            if self.main_area.tab_history.len() > TAB_HISTORY_CAP {
                self.main_area.tab_history
                    .drain(..self.main_area.tab_history.len() - TAB_HISTORY_CAP);
            }
            self.main_area.active_tab_index = index;
            let focused = self.main_area.tabs[index].last_focused_pane;
            self.main_area.focused_pane_id = focused;
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
        if from == to || from >= self.main_area.tabs.len() || to >= self.main_area.tabs.len() {
            return;
        }
        let tab = self.main_area.tabs.remove(from);
        self.main_area.tabs.insert(to, tab);
        if self.main_area.active_tab_index == from {
            self.main_area.active_tab_index = to;
        } else {
            // Adjust active index for the shift.
            let a = self.main_area.active_tab_index;
            self.main_area.active_tab_index = if from < a && to >= a {
                a - 1
            } else if from > a && to <= a {
                a + 1
            } else {
                a
            };
        }
        // Adjust history indices to track the reorder.
        for idx in &mut self.main_area.tab_history {
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
        self.main_area.activity_counter.insert(pane_id, self.main_area.activity_tick);
    }

    pub(in crate::workspace) fn focus_next_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.main_area.tabs.get_mut(self.main_area.active_tab_index) else {
            return;
        };
        if let Some(next) = tab.layout.next_pane(self.main_area.focused_pane_id) {
            self.main_area.focused_pane_id = next;
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
        let Some(tab) = self.main_area.tabs.get_mut(self.main_area.active_tab_index) else {
            return;
        };
        if let Some(prev) = tab.layout.prev_pane(self.main_area.focused_pane_id) {
            self.main_area.focused_pane_id = prev;
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
        let Some(tab) = self.main_area.tabs.get(self.main_area.active_tab_index) else {
            return;
        };
        let (w, h) = self.main_area.last_viewport.unwrap_or((1.0, 1.0));
        let mut rects = Vec::new();
        collect_pane_rects(&tab.layout, 0.0, 0.0, w, h, &mut rects);
        if let Some(target) =
            pane_in_direction(&rects, self.main_area.focused_pane_id, dir, &self.main_area.activity_counter)
        {
            self.main_area.focused_pane_id = target;
            if let Some(t) = self.main_area.tabs.get_mut(self.main_area.active_tab_index) {
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
        if self.main_area.zoomed_pane_id == Some(pane_id) {
            self.main_area.zoomed_pane_id = None;
        } else {
            self.main_area.zoomed_pane_id = Some(pane_id);
        }
        cx.notify();
    }

    // ---- Split management ----

    pub(in crate::workspace) fn split_focused_pane(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_pane = match self.create_pane(window, cx) {
            Ok(p) => p,
            Err(e) => {
                self.report_pane_error("split", e, cx);
                return;
            }
        };
        let new_pane_id = new_pane.id;
        self.main_area.panes.push(new_pane);

        let focused = self.main_area.focused_pane_id;
        for tab in &mut self.main_area.tabs {
            if insert_split_at(&mut tab.layout, focused, direction, new_pane_id) {
                tab.last_focused_pane = new_pane_id;
                break;
            }
        }

        self.main_area.focused_pane_id = new_pane_id;
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
            .main_area.tabs
            .iter()
            .position(|t| t.layout.pane_ids().contains(&pane_id))
        else {
            return;
        };
        let leaf_count = self.main_area.tabs[tab_index].layout.leaf_count();
        if leaf_count <= 1 {
            self.close_tab_at(tab_index, window, cx);
            return;
        }

        let next_focus = self.main_area.tabs[tab_index]
            .layout
            .prev_pane(pane_id)
            .unwrap_or_else(|| self.main_area.tabs[tab_index].layout.first_leaf());

        if self.main_area.zoomed_pane_id == Some(pane_id) {
            self.main_area.zoomed_pane_id = None;
        }
        remove_pane_from_layout(&mut self.main_area.tabs[tab_index].layout, pane_id);
        self.claude.pty_tracker.unregister(pane_id);
        self.main_area.panes.retain(|p| p.id != pane_id);
        self.main_area.activity_counter.remove(&pane_id);

        if tab_index == self.main_area.active_tab_index {
            self.main_area.focused_pane_id = next_focus;
            self.bump_activity(next_focus);
            self.focus_pane(next_focus, window, cx);
        }
        self.main_area.tabs[tab_index].last_focused_pane = next_focus;
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    pub(in crate::workspace) fn close_focused_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_pane(self.main_area.focused_pane_id, window, cx);
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
            .filter_map(|&i| self.main_area.tabs.get(i))
            .flat_map(|tab| tab.layout.pane_ids().into_iter())
            .filter_map(|id| {
                let pane = self.main_area.panes.iter().find(|p| p.id == id)?;
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

        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            crate::surface::strings::TAB_CLOSE_BATCH_HEADING,
            Some(&detail),
            &[
                crate::surface::strings::TAB_CLOSE_BATCH_SAVE_ALL,
                crate::surface::strings::TAB_CLOSE_BATCH_DISCARD_ALL,
                crate::surface::strings::TASK_EDIT_CANCEL,
            ],
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
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
        let Some(tab) = self.main_area.tabs.get(index) else {
            return;
        };

        let dirty: Vec<(PaneId, gpui::SharedString, bool)> = tab
            .layout
            .pane_ids()
            .iter()
            .filter_map(|&id| {
                let pane = self.main_area.panes.iter().find(|p| p.id == id)?;
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

        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            crate::surface::strings::TAB_CLOSE_BATCH_HEADING,
            Some(&detail),
            &[
                crate::surface::strings::TAB_CLOSE_BATCH_SAVE_ALL,
                crate::surface::strings::TAB_CLOSE_BATCH_DISCARD_ALL,
                crate::surface::strings::TASK_EDIT_CANCEL,
            ],
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
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
        let Some(pane) = self.main_area.panes.iter().find(|p| p.id == pane_id) else {
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
            crate::surface::strings::TASK_EDIT_DISCARD_DRAFT_PROMPT.to_string()
        } else {
            format!(
                "{}{}{}",
                crate::surface::strings::TASK_EDIT_SAVE_PROMPT_PREFIX,
                title,
                crate::surface::strings::TASK_EDIT_SAVE_PROMPT_SUFFIX,
            )
        };

        let save_label = if is_draft {
            crate::surface::strings::TASK_EDIT_SAVE_DRAFT
        } else {
            crate::surface::strings::TASK_EDIT_SAVE
        };
        let buttons = [
            save_label,
            crate::surface::strings::TASK_EDIT_DISCARD,
            crate::surface::strings::TASK_EDIT_CANCEL,
        ];

        let receiver = window.prompt(gpui::PromptLevel::Warning, &heading, None, &buttons, cx);

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
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
