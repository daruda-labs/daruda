//! Lane metadata edits: drag reorder within a project and the description /
//! session-host / name setters.

use daruda_store::project::{LaneRef, LaneSessionHost};
use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Move `from` immediately before `to` in the lanes list of
    /// the project they share, renumbering `tab_order` for all
    /// lanes afterwards. Lane DnD is intentionally scoped to a
    /// single project — cross-project drops are rejected as a no-op so
    /// a lane never migrates between projects through the dock.
    /// Also no-ops when `from == to`, when the project is missing, or
    /// when either id is not present in that project.
    pub(in crate::workspace) fn reorder_lane(
        &mut self,
        from: LaneRef,
        to: LaneRef,
        cx: &mut Context<Self>,
    ) {
        if from == to {
            return;
        }
        if from.project != to.project {
            return;
        }
        let Some(project) = self.projects.get_mut(from.project) else {
            return;
        };
        let from_idx = project.lanes.iter().position(|w| w.id == from.lane);
        let to_idx = project.lanes.iter().position(|w| w.id == to.lane);
        let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else {
            return;
        };
        // Insert at `to`'s ORIGINAL index. Direction-aware via the
        // implicit shift caused by `remove`: when `from_idx < to_idx`
        // (downward drag) the removal shifts `to` down one slot, so
        // inserting at `to_idx` lands AFTER the now-shifted `to`;
        // when `from_idx > to_idx` (upward drag) `to` keeps its
        // index, so inserting at `to_idx` lands BEFORE `to`. This
        // matches the standard list-DnD expectation that "drop X
        // onto Y's slot" makes X take Y's row regardless of drag
        // direction.
        let item = project.lanes.remove(from_idx);
        project.lanes.insert(to_idx, item);
        for (i, w) in project.lanes.iter_mut().enumerate() {
            w.tab_order = i as u32;
        }
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// Apply `f` to the lane named by the full [`LaneRef`], then persist +
    /// notify iff a lane was actually found. Resolves the project by
    /// `target.project` — never the *active* project — because lane ids
    /// restart per project (every project has a lane id 0), so a bare
    /// `LaneId` against the active project would silently mutate the wrong
    /// project's like-id'd lane (a left-dock edit on a background project
    /// bleeding into the foreground one).
    fn mutate_lane<F>(&mut self, target: LaneRef, f: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut crate::lane::Lane),
    {
        let mutated = self
            .project_for_mut(target.project)
            .and_then(|p| p.lanes.iter_mut().find(|w| w.id == target.lane))
            .map(f)
            .is_some();
        if mutated {
            self.mutate_durable(cx, |_, _| {});
            cx.notify();
        } else {
            // The target lane/project vanished between the affordance being
            // built (context-menu open) and the edit landing — e.g. the lane
            // was removed while the edit modal was open. Log rather than
            // silently drop the user's typed value. No toast: it is a rare
            // race and one per stale submit would be noisier than useful.
            let report = daruda_store::observability::error_report::ErrorReport::new(
                "Lane edit dropped: target no longer exists",
            )
            .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
            .with_context("project", target.project.to_string())
            .with_context("lane", target.lane.to_string())
            .at(file!(), line!())
            .dedup("lane.mutate.stale_target")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
    }

    /// Update the free-form description for the lane named by `target`.
    /// `None` clears it, reverting the left dock sublabel to the lane path.
    pub(in crate::workspace) fn set_lane_description(
        &mut self,
        target: LaneRef,
        description: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_description(description), cx);
    }

    /// Answer where the lane named by `target` attaches its session —
    /// always `Some`, since opening the Session Host modal and saving *is*
    /// answering the question (picking Local is the explicit "run it here"
    /// that retires a legacy `remote_cwd`/agent pair, not a way to go back
    /// to unanswered — see `Lane::set_session_host`'s doc). Every pane's
    /// next *connect* (fresh or already open) re-resolves from the lane
    /// fresh via `resolve_session_command` — only an already-connected
    /// session keeps running on the host it connected with.
    pub(in crate::workspace) fn set_lane_session_host(
        &mut self,
        target: LaneRef,
        host: LaneSessionHost,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_session_host(Some(host)), cx);
    }

    /// Update the user-visible display name for the lane named by `target`.
    /// `None` clears it and reverts to the branch / path fallback.
    pub(in crate::workspace) fn set_lane_name(
        &mut self,
        target: LaneRef,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_name(name), cx);
    }
}
