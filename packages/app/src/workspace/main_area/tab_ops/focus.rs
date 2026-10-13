//! Pane focus: the focused pane and its window label, dimming, click and
//! directional focus moves, and forgetting closed panes.

use crate::workspace::Workspace;
use crate::workspace::main_area::nav::{NavDirection, pane_in_direction};
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::{PaneId, collect_pane_rects};
use gpui::{Context, Window};

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
            ws.window_runtime.user_label = label.map(gpui::SharedString::from);
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
        // saved to its owner (`InputDock::owner`, the pane it was typed for), then
        // the incoming pane's draft is restored. Non-input panes leave the
        // text and owner in place. The `input_owner != Some(id)` guard matters
        // because `InputState::set_value` resets selection/scroll even for
        // identical text, which would jump the cursor mid-edit.
        if self.pane_consumes_bottom_input(id) && self.input_dock.owner != Some(id) {
            let current = self.input_dock.input.read(cx).value().to_string();
            if let Some(owner) = self.input_dock.owner {
                if current.is_empty() {
                    self.input_dock.drafts.remove(&owner);
                } else {
                    self.input_dock.drafts.insert(owner, current);
                }
            }
            let incoming = self.input_dock.drafts.get(&id).cloned().unwrap_or_default();
            let terminal_input = self.input_dock.input.clone();
            terminal_input.update(cx, |state, cx_state| {
                state.set_value(&incoming, window, cx_state);
            });
            self.input_dock.owner = Some(id);
        }
        self.active_runtime_mut().focused_pane_id = id;
        cx.notify();
        // Re-evaluate inactive-pane dim (focused pane goes full color, former
        // sibling dims).
        self.refresh_pane_dimming(cx);
        // Placeholder + panel activation stay on the `focus_pane` path; this
        // method only tracks the focused pane + swaps the draft.
    }

    /// Drop a removed pane's bottom-dock input draft and clear `InputDock::owner`
    /// when it pointed here, so the next focus does not save stale visible
    /// text back to a gone pane. Single site every pane-removal path (close
    /// pane / tab / lane / project) calls, keeping `InputDock::drafts` free of
    /// dead entries.
    /// Forget every per-pane record the workspace keeps for panes that are
    /// going away — tracking, unseen outcomes, focus recency, input draft.
    /// The one call each teardown (pane, tab, lane, project) makes, so none
    /// of them forgets a smaller set than the others.
    pub(in crate::workspace) fn forget_panes(
        &mut self,
        pane_ids: &[PaneId],
        cx: &mut Context<Self>,
    ) {
        self.release_pane_tracking(pane_ids, cx);
        for id in pane_ids {
            self.main_area.activity_counter.remove(id);
            self.forget_pane_input_draft(*id);
        }
    }

    pub(in crate::workspace) fn forget_pane_input_draft(&mut self, pane_id: PaneId) {
        self.input_dock.drafts.remove(&pane_id);
        if self.input_dock.owner == Some(pane_id) {
            self.input_dock.owner = None;
        }
    }

    /// True when `id` names a pane that reads the shared bottom-dock input:
    /// a Terminal (PTY stdin) or an AgentChat (ACP prompt). File and flow
    /// panes return `false`, so focusing them keeps the visible
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
                && (view.read(cx).dim_amount() - target).abs() > f32::EPSILON
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
    pub(in crate::workspace::main_area) fn set_menu_target_pane(
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
}
