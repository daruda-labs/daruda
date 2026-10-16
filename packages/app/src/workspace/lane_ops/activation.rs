//! Lane activation: switching by index or ref, same-lane re-probe, and
//! re-pointing the right dock at an inaccessible lane.

use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window};

use crate::lane::availability::LaneAvailability;
use crate::workspace::Workspace;
use crate::workspace::main_area::tab_ops::PaneEntry;

impl Workspace {
    /// Switch to the nth lane (0-indexed) of the active project, sorted
    /// by `tab_order` to match the left-dock order. No-ops when `index`
    /// is out of range or no project is loaded.
    pub(in crate::workspace) fn activate_lane_by_index(
        &mut self,
        index: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(active_project_id) = self.active_project().map(|p| p.id) else {
            return;
        };
        let mut ids: Vec<(u32, LaneId)> = self
            .active_lanes()
            .iter()
            .map(|w| (w.tab_order, w.id))
            .collect();
        ids.sort_unstable_by_key(|&(order, _)| order);
        if let Some(&(_, id)) = ids.get(index) {
            self.activate_lane(
                LaneRef {
                    project: active_project_id,
                    lane: id,
                },
                window,
                cx,
            );
        }
    }

    /// Switch the visible lane. Every runtime lives in the single
    /// `runtimes` map; the active lane is the entry under `self.active`,
    /// so activating another just re-points that key — nothing is moved
    /// and parked lanes' PTY entities keep running. An unpopulated target
    /// seeds an empty runtime entry (no lazy pane spawn).
    pub(in crate::workspace) fn activate_lane(
        &mut self,
        target: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_lane_as(target, PaneEntry::Enter, window, cx);
    }

    pub(in crate::workspace) fn activate_lane_as(
        &mut self,
        target: LaneRef,
        entry: PaneEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active == target {
            self.close_page(cx);
            if entry == PaneEntry::Enter {
                self.reactivate_active_lane(window, cx);
            }
            return;
        }
        if self.lane_for(target).is_none() {
            return;
        }
        self.close_page(cx);
        // The orchestrator belongs to no worktree, so it does not follow the
        // user into the next one — a tab that trailed every switch would read
        // as content of whichever lane it landed in. The chip puts it back.
        self.close_orchestrator_tab_for_lane_change(window, cx);
        // Leaving the active lane abandons any pending amend (the prefill
        // belongs to the lane we're leaving). No-op when not amending.
        self.exit_amend_mode(window, cx);
        // Re-classify the incoming lane's root against the live
        // filesystem first: a lane whose directory vanished while inactive
        // flips to non-`Present` here, skipping the path-dependent tail.
        self.recompute_availability_for(target);
        // Active-lane change invalidates both the previous and incoming
        // visible file lists (selection moves with the active id).
        let previous = self.active;
        self.invalidate_visible_files_cache(previous);
        self.invalidate_visible_files_cache(target);
        // Clear keyboard cursor — it lived in the previous visible list.
        self.file_tree.files_selection = None;

        // Drop any in-flight drag hover so a stale half-fill overlay does
        // not linger on the newly-activated lane.
        self.main_area.pane_drop_hover = None;

        // Both runtimes live in the single `runtimes` map — nothing to swap;
        // a lane never activated gets an empty one.
        self.set_active(target);
        // The project layer is per project, so another project means another
        // effective config — what new panes spawn with and the dot reports.
        if previous.project != target.project {
            self.apply_store_config(cx);
        }
        // Seen now. Persisted by the `mutate_durable` each branch below ends on.
        if let Some(lane) = self.lane_for_mut(target)
            && std::mem::take(&mut lane.is_unread)
        {
            Self::refresh_dock_badge(cx);
        }

        // The bottom-dock draft swaps per input pane (via
        // `set_focused_pane`), not per lane — handled on the focus path
        // below when the incoming lane has a focused pane.

        // Update the last-active-lane hint so clicking the project header
        // snaps back to the lane the user just left.
        if let Some(project) = self.projects.get_mut(target.project) {
            project.last_active_lane_id = target.lane;
        }

        // An unavailable lane (root deleted / unreadable) still becomes
        // active, but path-spawning work is skipped: it would root a PTY
        // at the dead path and `refresh_git_status` would spam a toast.
        // The right-dock reconcile + persist below are filesystem-tolerant
        // (skills/mcp scan a missing dir as empty; commit button reads
        // in-memory cache), so they run — else the panels keep showing the
        // previous lane's data.
        if self
            .lane_for(target)
            .map(|l| l.availability != LaneAvailability::Present)
            .unwrap_or(false)
        {
            self.reconcile_right_dock_for_inaccessible_lane(window, cx);
            // Persist the active-ref change (`set_active(target)`) so a
            // quit right after selecting a missing lane keeps the
            // selection. The Present path persists via the tail below.
            self.mutate_durable(cx, |_, _| {});
            cx.notify();
            return;
        }

        // 3. No lazy seed. An empty lane (never opened, or emptied by the
        //    user) stays empty and renders the "No open tabs" empty-state;
        //    the user opens content from there. Only the first project at
        //    app launch seeds a shell (see `new_with_project_impl`).

        // 4. Refocus the active pane and request a resize — the
        //    lane may have been last seen at a different viewport.
        //    Route through `set_focused_pane` so the bottom-dock draft
        //    swaps to the newly-focused pane. An empty lane has no pane to
        //    focus (guarded below), so the draft simply stays put.
        if self
            .active_runtime()
            .panes
            .iter()
            .any(|p| p.id == self.active_runtime().focused_pane_id)
        {
            let focused = self.active_runtime().focused_pane_id;
            self.set_focused_pane(focused, window, cx);
            // Cannot be the orchestrator's: its tab was taken down above, and
            // `remove_orchestrator_tab` repairs `focused_pane_id` in every
            // runtime it touches.
            if entry == PaneEntry::Enter {
                self.focus_pane(focused, window, cx);
            }
        }
        // The incoming lane's runtime carries its own panes/split state;
        // recompute inactive-pane dim against the now-live focused pane.
        self.refresh_pane_dimming(cx);
        self.main_area.pending_resize = true;
        // The now-active lane's runtime may still hold File panes with
        // `Loading` content (a lane restored but never activated defers
        // its loads); fire them now that the lane is active.
        self.load_pending_file_panes(cx);
        self.mutate_durable(cx, |_, _| {});
        // 5. If the incoming lane's tree was modified while
        //    inactive, replay a single Bulk reload to catch up.
        self.replay_files_dirty(target, cx);
        // Project skill scope follows the active lane's path —
        // re-spawn so the panel switches to the new repo's skills.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.respawn_flow_watcher(cx);
        // Commit button reflects the active lane's staged count —
        // recompute now that `active` has flipped.
        self.sync_commit_buttons(cx);
        // Re-probe git status for the lane we just switched to so the
        // left-dock branch label catches up to any checkout done while
        // it was inactive — `refresh_git_status` reconciles
        // `Lane.kind.branch` from the live `git status` result.
        self.refresh_git_status(target, cx);

        cx.notify();
    }

    /// Same-lane click handler (the `self.active == target` fast path of
    /// [`Self::activate_lane`]). A healthy already-active lane is the
    /// common case and stays cheap: re-probe, see it's still `Present`
    /// with tabs, fall through to `notify`.
    ///
    /// The case that matters is self-healing: if the active lane was
    /// `Missing` / `AccessDenied` (root deleted, or Full Disk Access not
    /// yet granted) and the user has since recreated the directory or
    /// granted access, clicking the same row must re-probe and recover.
    /// `activate_lane`'s cross-lane `recompute_availability_for` never
    /// runs for a same-lane click, so without this re-probe the lane
    /// stays stuck non-`Present` until the user switches away and back.
    ///
    /// On recovery to `Present` the lane is re-probed and the right dock
    /// re-pointed, but no tab is seeded: an empty recovered lane lands on
    /// the "No open tabs" empty-state (the unified rule). Still non-`Present`
    /// → re-point the right dock (it may have nothing reflected yet) and
    /// stay put.
    fn reactivate_active_lane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active;
        let was_present = self
            .lane_for(active)
            .map(|l| l.availability == LaneAvailability::Present)
            .unwrap_or(false);
        // Healthy lane that already has tabs: nothing a same-lane click
        // should act on — keep the fast path cheap (never re-probe or
        // disturb live terminals). A `Present` lane with *no* tabs (user
        // closed them all, or it was never opened) deliberately falls
        // through so the re-probe below can recover availability; it still
        // renders the empty-state afterward.
        if was_present && !self.active_runtime().tabs.is_empty() {
            return;
        }
        self.recompute_availability_for(active);
        let now_present = self
            .lane_for(active)
            .map(|l| l.availability == LaneAvailability::Present)
            .unwrap_or(false);
        if now_present {
            // Recovered to Present. No tab is seeded — an empty lane renders
            // the empty-state; the user opens content from there. Focus only
            // if the lane already had a pane (guarded), then run the
            // path-dependent right-dock reconcile the empty-state had skipped.
            if self.has_focused_pane() {
                self.focus_pane(self.active_runtime().focused_pane_id, window, cx);
            }
            self.main_area.pending_resize = true;
            // The recovered availability is persisted runtime state.
            self.mutate_durable(cx, |_, _| {});
            // Deliberately omits `activate_lane`'s `load_pending_file_panes`
            // + `replay_files_dirty`: a lane reaching here was either Missing
            // all session (blank runtime — no Loading file panes, no dirty
            // backlog) or a Present lane whose tabs were all closed (likewise
            // nothing pending). Add them only if a future path can leave
            // stale file-view panes on a recovered lane.
            self.refresh_skills_watcher(cx);
            self.refresh_mcp_watcher(window, cx);
            self.sync_commit_buttons(cx);
            self.refresh_git_status(active, cx);
        } else {
            // Still inaccessible — re-point the right dock so it shows the
            // empty (inaccessible) lane rather than stale data.
            self.reconcile_right_dock_for_inaccessible_lane(window, cx);
        }
        cx.notify();
    }

    /// Re-point the right dock (Skills / Tools panels + commit button) at
    /// the now-active lane when that lane is inaccessible. Deliberately
    /// excludes `refresh_git_status` — it shells out `git status` against
    /// the dead path and would spam an error toast. The skills / mcp
    /// watchers scan a missing directory as empty and the commit button
    /// reads the in-memory cache (absent → disabled), so all three show
    /// an empty panel instead of the previous lane's data.
    fn reconcile_right_dock_for_inaccessible_lane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.respawn_flow_watcher(cx);
        self.sync_commit_buttons(cx);
    }
}
