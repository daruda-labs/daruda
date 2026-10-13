mod close_guard;
mod focus;
mod pane_structure;

use gpui::{Context, Window};

use super::pane::TabEntry;
use super::pane_tree::{PaneId, PaneLayout};
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::images::release_pane_images;

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

/// Whether activating a pane also moves the user into it.
///
/// Splitting this out is what keeps "put this file on screen" from carrying
/// "and start typing at it": [`Workspace::focus_pane`] surfaces the bottom
/// dock's Input panel and takes keyboard focus, which is right for a pane the
/// user walked into and wrong for one a left-dock row is previewing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum PaneEntry {
    /// Keyboard focus and the bottom input follow the pane.
    Enter,
    /// The tab and the focused pane change; the user stays where they are.
    Preview,
}

/// What a caller is doing when it opens a file, over the two axes that
/// differ: who ends up with keyboard focus, and whether the tab is the
/// replaceable preview slot. zed carries these as `focus_item` and
/// `allow_preview`; only three of the four combinations are real, so naming
/// them keeps the fourth from being expressible.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum OpenIntent {
    /// A left-dock row the cursor is resting on: the file appears, the user
    /// stays in the panel, and the tab is the slot a later skim may reuse.
    Preview,
    /// A left-dock row the user pressed Enter on: the file keeps its tab, but
    /// the user still stays in the panel so the arrows keep working.
    Commit,
    /// Navigation into the file from anywhere else — a flow, an agent link, a
    /// skill: the pane takes focus and keeps its tab.
    Enter,
}

impl OpenIntent {
    /// Whether the user moves into the opened pane.
    pub(in crate::workspace) fn pane_entry(self) -> PaneEntry {
        match self {
            Self::Enter => PaneEntry::Enter,
            Self::Preview | Self::Commit => PaneEntry::Preview,
        }
    }

    /// Whether the opened tab becomes the replaceable preview slot.
    pub(in crate::workspace) fn claims_preview_slot(self) -> bool {
        matches!(self, Self::Preview)
    }
}

impl Workspace {
    pub(in crate::workspace) fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.insert_terminal_tab(None, window, cx);
    }

    /// Open a terminal tab in the active lane at `cwd` (`None` = the
    /// inherited default) and focus it. `None` when the lane cannot host one
    /// or the PTY failed to spawn (already reported).
    pub(in crate::workspace) fn insert_terminal_tab(
        &mut self,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PaneId> {
        // An inaccessible active lane renders the empty-state; spawning a tab
        // would root a PTY at the dead lane path and escape it. No-op. (A
        // workspace with no project at all still allows tabs.)
        if self.active_lane_is_inaccessible() {
            return None;
        }
        let cwd = cwd.or_else(|| self.default_cwd_for_new_pane());
        let pane = match self.create_pane_at(cwd, window, cx) {
            Ok(p) => p,
            Err(e) => {
                self.report_pane_error(
                    &crate::surface::strings::error::pane_context_new_tab(),
                    e,
                    cx,
                );
                return None;
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
        Some(pane_id)
    }

    /// Total open tabs across every lane runtime in the window (all
    /// projects, all lanes). The window-close decision reads this — not the
    /// active lane's count alone — so closing one lane's last tab never
    /// tears down the window while another lane still holds content.
    pub(in crate::workspace) fn total_open_tabs(&self) -> usize {
        self.main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.tabs.iter())
            .filter(|tab| !self.is_orchestrator_tab(tab))
            .count()
    }

    /// Drop every pane in the active lane's runtime and reset it to an empty
    /// [`LaneRuntime`](crate::workspace::LaneRuntime) (the "no tabs" empty
    /// state). Releases PTY tracking, activity counters, zoom, and drafts for
    /// the removed panes first (same cleanup as `close_tab_at`). The lane
    /// stays in the sidebar.
    pub(in crate::workspace) fn empty_active_lane_runtime(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_ids: Vec<PaneId> = self.active_runtime().panes.iter().map(|p| p.id).collect();
        if pane_ids
            .iter()
            .any(|&id| self.main_area.zoomed_pane_id == Some(id))
        {
            self.main_area.zoomed_pane_id = None;
        }
        self.main_area.pane_drop_hover = None;
        self.forget_panes(&pane_ids, cx);
        release_pane_images(&mut self.active_runtime_mut().panes, &pane_ids, window, cx);
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
        if self.is_orchestrator_tab(&self.active_runtime().tabs[index]) {
            self.hide_orchestrator_tab(window, cx);
            return;
        }
        if self.total_open_tabs() <= 1 {
            window.remove_window();
            return;
        }
        // The active lane's last tab. Close the window only when it is also
        // the window's last tab across every lane and project; otherwise empty
        // this lane in place (→ empty-state) and keep the window, so closing
        // one lane's content never tears down others parked in `runtimes`.
        if self.active_runtime().tabs.len() <= 1 {
            self.empty_active_lane_runtime(window, cx);
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
        self.forget_panes(&pane_ids, cx);
        release_pane_images(&mut self.active_runtime_mut().panes, &pane_ids, window, cx);
        self.active_runtime_mut()
            .panes
            .retain(|p| !pane_ids.contains(&p.id));

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
        self.activate_tab_as(index, PaneEntry::Enter, window, cx);
    }

    /// [`Self::activate_tab`], with `entry` deciding whether the user moves
    /// into the tab's pane or only sees it. See [`PaneEntry`].
    pub(in crate::workspace) fn activate_tab_as(
        &mut self,
        index: usize,
        entry: PaneEntry,
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
            if entry == PaneEntry::Enter {
                ws.focus_pane(focused, window, cx);
            }
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
    pub(super) fn restore_active_tab_by_id(&mut self, tab_id: u64) -> bool {
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

    // ---- Context-menu tab operations ----
    //
    // Close Other Tabs / Close Tabs to Right route through
    // `request_close_tabs_bulk`, whose dirty-prompt covers every dirty
    // pane in the closing set. Never loop over `close_tab_at`
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

    // ---- Split management ----

    // ---- Dirty-checked close entry points ----
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
pub(in crate::workspace) fn rebase_tab_history_after_removal(
    history: &mut Vec<usize>,
    removed_index: usize,
) {
    history.retain(|&i| i != removed_index);
    for idx in history.iter_mut() {
        if *idx > removed_index {
            *idx -= 1;
        }
    }
}
