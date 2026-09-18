//! Git Changes dock keyboard cursor + directory collapse.

use std::path::PathBuf;

use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window};

use crate::workspace::Workspace;
use crate::workspace::lane_scoped::GitCursor;
use crate::workspace::main_area::tab_ops::OpenIntent;

impl Workspace {
    /// Wired to `ToggleGitChangesFocus` — the only keyboard way into this
    /// panel, and the way back out.
    pub(in crate::workspace) fn toggle_git_changes_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.git_changes_panel_focus.clone();
        self.toggle_left_dock_panel_focus(
            daruda_store::project::LeftDockView::GitChanges,
            panel,
            window,
            cx,
        );
    }

    /// Keyboard-navigable file order for the active lane's Git Changes view,
    /// each entry carrying the list row it is drawn at. Defers to
    /// `visible_file_rows` so render-order changes apply to `↑↓` nav too.
    fn git_changes_visible_rows(&self) -> Vec<(usize, PathBuf)> {
        let Some(s) = self.lane_git_worktree(self.active) else {
            return Vec::new();
        };
        let Some(wt) = self.active_lane() else {
            return Vec::new();
        };
        let collapsed = self
            .lane_scoped
            .get(&self.active)
            .map(|state| state.git.collapsed_dirs.clone())
            .unwrap_or_default();
        crate::workspace::left_dock::git_changes::visible_file_rows(s, &collapsed, &wt.paths())
    }

    /// Move the Git Changes keyboard cursor to a specific path. Used by
    /// row clicks so subsequent arrow-key nav resumes from the clicked
    /// row rather than wherever the cursor was last left. The position is
    /// recorded alongside so a later disappearance has somewhere to fall back
    /// to — see [`GitCursor`].
    pub(in crate::workspace) fn set_git_changes_cursor(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        // Keep the previous position when the path is not on screen (its group
        // is collapsed, a refresh is mid-flight): falling back to 0 would send
        // the next arrow key to the top of the list, which is the very thing
        // `GitCursor::index` exists to prevent.
        let index = self
            .git_changes_visible_rows()
            .iter()
            .position(|(_, p)| *p == path)
            .or_else(|| {
                self.lane_scoped
                    .get(&target)
                    .and_then(|state| state.git.cursor.as_ref())
                    .map(|c| c.index)
            })
            .unwrap_or(0);
        self.lane_scoped_mut(target).git.cursor = Some(GitCursor { path, index });
        cx.notify();
    }

    /// A click on a Git Changes file row. Owns the whole transition so the
    /// row's closure stays a one-line dispatch and the behaviour is reachable
    /// by a test without a synthetic mouse event. Double click hands the file
    /// to the external editor; a single click moves the cursor and previews.
    pub(in crate::workspace) fn on_git_changes_row_click(
        &mut self,
        lane_id: LaneId,
        repo_path: PathBuf,
        staged: bool,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A click landed in this panel, so this panel takes keyboard focus —
        // stated rather than left to GPUI's `track_focus` mousedown, and the
        // same rule the Files rows follow.
        self.git_changes_panel_focus.clone().focus(window, cx);
        // Resolved here rather than taken as a second argument: the row's
        // path is repo-root-relative, and a linked lane's root is not the
        // repo root, so only `LanePaths` can absolutise it correctly.
        let Some(abs) = self
            .active_lane()
            .map(|wt| wt.paths().from_git_status(&repo_path))
        else {
            return;
        };
        if click_count >= 2 {
            self.open_file_externally(lane_id, abs, cx);
            return;
        }
        self.set_git_changes_cursor(lane_id, repo_path, cx);
        self.open_git_file_diff(lane_id, abs, staged, OpenIntent::Preview, window, cx);
    }

    /// Move the Git Changes keyboard cursor to the next or previous row.
    /// `delta = +1` walks down, `delta = -1` walks up; the ends wrap. An empty
    /// list is a no-op.
    ///
    /// When the cursor's file has left the list since the last keypress — a
    /// discard, an agent reverting it, a collapsed group — this re-anchors at
    /// the position it held rather than moving, so one key press lands the
    /// user back where they were and the next one carries on.
    pub(in crate::workspace) fn move_git_changes_cursor(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible = self.git_changes_visible_rows();
        if visible.is_empty() {
            return;
        }
        let active_ref = self.active;
        let cursor = self
            .lane_scoped
            .get(&active_ref)
            .and_then(|state| state.git.cursor.as_ref());
        let current_idx = cursor.and_then(|c| visible.iter().position(|(_, path)| *path == c.path));
        let new_idx = match (current_idx, cursor) {
            (Some(i), _) => super::super::wrap_step(Some(i), delta, visible.len()),
            // Anchor gone, but we know where it was: land there.
            (None, Some(c)) => c.index.min(visible.len() - 1),
            (None, None) => super::super::wrap_step(None, delta, visible.len()),
        };
        let (row, path) = visible[new_idx].clone();
        self.lane_scoped_mut(active_ref).git.cursor = Some(GitCursor {
            path,
            index: new_idx,
        });
        // Keep the cursor on screen — skimming past the viewport otherwise
        // previews a row the user cannot see.
        self.git_changes_scroll_handle
            .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
        let panel = self.git_changes_panel_focus.clone();
        self.arm_left_dock_preview(panel, window, cx, |ws, window, cx| {
            ws.open_git_changes_cursor(OpenIntent::Preview, window, cx)
        });
        cx.notify();
    }

    /// Toggle the staged/unstaged state of the file under the keyboard
    /// cursor (Space). No-op when the cursor is unset or the file has
    /// vanished from `git status`.
    pub(in crate::workspace) fn toggle_git_changes_cursor_stage(&mut self, cx: &mut Context<Self>) {
        let active_ref = self.active;
        let active_id = self.active.lane;
        let Some(cursor) = self
            .lane_scoped
            .get(&active_ref)
            .and_then(|state| state.git.cursor.as_ref().map(|c| c.path.clone()))
        else {
            return;
        };
        let Some(s) = self.lane_git_worktree(active_ref) else {
            return;
        };
        let is_staged = s.staged.iter().any(|e| e.path == cursor);
        if is_staged {
            self.unstage_file(active_id, cursor, cx);
        } else {
            self.stage_file(active_id, cursor, cx);
        }
    }

    /// Open the diff viewer for the file under the keyboard cursor (Enter).
    /// Drops any pending preview: the user asked for this row now, and letting
    /// the timer fire afterwards would re-open the same file a second time.
    pub(in crate::workspace) fn activate_git_changes_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.left_dock_preview = None;
        let (path, staged) = self.git_changes_cursor_target();
        if self.step_into_open_file_view(path, staged, window, cx) {
            return;
        }
        self.open_git_changes_cursor(OpenIntent::Commit, window, cx);
    }

    /// Absolute path the keyboard cursor points at and whether that row is
    /// staged — the pair that identifies an open file pane.
    fn git_changes_cursor_target(&self) -> (Option<PathBuf>, bool) {
        let Some(cursor) = self
            .lane_scoped
            .get(&self.active)
            .and_then(|state| state.git.cursor.as_ref().map(|c| c.path.clone()))
        else {
            return (None, false);
        };
        let staged = self
            .lane_git_worktree(self.active)
            .is_some_and(|s| s.staged.iter().any(|e| e.path == cursor));
        let abs = self
            .active_lane()
            .map(|wt| wt.paths().from_git_status(&cursor));
        (abs, staged)
    }

    /// Open the file under the keyboard cursor in the pane-area viewer,
    /// keeping the panel focused. Shared by Enter and the arrow-key preview —
    /// the two differ only in what triggers them.
    fn open_git_changes_cursor(
        &mut self,
        intent: OpenIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_ref = self.active;
        let active_id = self.active.lane;
        let Some(cursor) = self
            .lane_scoped
            .get(&active_ref)
            .and_then(|state| state.git.cursor.as_ref().map(|c| c.path.clone()))
        else {
            return;
        };
        let Some(s) = self.lane_git_worktree(active_ref) else {
            return;
        };
        let staged_entry = s.staged.iter().find(|e| e.path == cursor);
        let unstaged_entry = s.unstaged.iter().find(|e| e.path == cursor);
        let is_staged = match (staged_entry, unstaged_entry) {
            (Some(_), _) => true,
            (None, Some(_)) => false,
            (None, None) => return,
        };

        // The diff viewer loads from the filesystem, so resolve the
        // repo-root-relative cursor to an absolute path via LanePaths.
        let Some(wt) = self.active_lane() else {
            return;
        };
        let abs = wt.paths().from_git_status(&cursor);
        self.open_git_file_diff(active_id, abs, is_staged, intent, window, cx);
    }

    /// Toggle the collapse state of a directory group in the Git Changes
    /// view. Per-lane and in-memory only — not persisted across restarts,
    /// since the view is task-driven and stale collapse state is just noise.
    pub(in crate::workspace) fn toggle_git_dir_collapse(
        &mut self,
        lane_id: LaneId,
        dir: String,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let set = &mut self.lane_scoped_mut(target).git.collapsed_dirs;
        if !set.remove(&dir) {
            set.insert(dir);
        }
        cx.notify();
    }
}
