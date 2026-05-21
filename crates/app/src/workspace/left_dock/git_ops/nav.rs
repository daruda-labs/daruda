//! Git Changes dock keyboard cursor + directory collapse.

use std::path::PathBuf;

use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window};

use crate::workspace::Workspace;

impl Workspace {
    /// Build the keyboard-navigable file order for the Git Changes view
    /// in the active lane. Defers to the left dock's
    /// `ordered_visible_paths` helper so any future change to the render
    /// order (sticky conflicts, custom sort) automatically applies to
    /// `↑↓` nav.
    fn git_changes_visible_paths(&self) -> Vec<PathBuf> {
        let Some(s) = self.git_status_cache.get(&self.active) else {
            return Vec::new();
        };
        let Some(wt) = self.active_lane() else {
            return Vec::new();
        };
        let collapsed = self
            .git_collapsed_dirs
            .get(&self.active)
            .cloned()
            .unwrap_or_default();
        crate::workspace::left_dock::git_changes::ordered_visible_paths(s, &collapsed, &wt.paths())
    }

    /// Move the Git Changes keyboard cursor to a specific path. Used by
    /// row clicks so subsequent arrow-key nav resumes from the clicked
    /// row rather than wherever the cursor was last left.
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
        self.git_changes_cursor.insert(target, path);
        cx.notify();
    }

    /// Move the Git Changes keyboard cursor to the next or previous row.
    /// `delta = +1` walks down, `delta = -1` walks up. Out-of-list
    /// cursors snap to the first/last visible row; an empty list is a
    /// no-op.
    pub(in crate::workspace) fn move_git_changes_cursor(
        &mut self,
        delta: isize,
        cx: &mut Context<Self>,
    ) {
        let visible = self.git_changes_visible_paths();
        if visible.is_empty() {
            return;
        }
        let active_ref = self.active;
        let current_idx = self
            .git_changes_cursor
            .get(&active_ref)
            .and_then(|p| visible.iter().position(|v| v == p));
        let new_idx: usize = match (current_idx, delta) {
            (None, d) if d >= 0 => 0,
            (None, _) => visible.len() - 1,
            (Some(i), d) => {
                let len = visible.len() as isize;
                ((i as isize + d).rem_euclid(len)) as usize
            }
        };
        self.git_changes_cursor
            .insert(active_ref, visible[new_idx].clone());
        cx.notify();
    }

    /// Toggle the staged/unstaged state of the file under the keyboard
    /// cursor (Space). No-op when the cursor is unset or the file has
    /// vanished from `git status`.
    pub(in crate::workspace) fn toggle_git_changes_cursor_stage(&mut self, cx: &mut Context<Self>) {
        let active_ref = self.active;
        let active_id = self.active.lane;
        let Some(cursor) = self.git_changes_cursor.get(&active_ref).cloned() else {
            return;
        };
        let Some(s) = self.git_status_cache.get(&active_ref) else {
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
    pub(in crate::workspace) fn activate_git_changes_cursor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_ref = self.active;
        let active_id = self.active.lane;
        let Some(cursor) = self.git_changes_cursor.get(&active_ref).cloned() else {
            return;
        };
        let Some(s) = self.git_status_cache.get(&active_ref) else {
            return;
        };
        let staged_entry = s.staged.iter().find(|e| e.path == cursor);
        let unstaged_entry = s.unstaged.iter().find(|e| e.path == cursor);
        let (is_staged, status_char) = match (staged_entry, unstaged_entry) {
            (Some(se), _) => (true, se.x),
            (None, Some(ue)) => (false, ue.y),
            (None, None) => return,
        };

        // The diff viewer wants the absolute path (it routes through
        // `open_pane_file_view` which loads from the filesystem); resolve
        // the repo-root-relative cursor via LanePaths.
        let Some(wt) = self.active_lane() else {
            return;
        };
        let abs = wt.paths().from_git_status(&cursor);
        self.open_git_file_diff(active_id, abs, is_staged, Some(status_char), window, cx);
    }

    /// Toggle the collapse state of a directory group in the Git Changes
    /// view. State is per-lane and in-memory only — Git Changes is
    /// task-driven (open it, deal with the diff, close it), so persisting
    /// collapse state across app restarts would mostly preserve stale
    /// "I last collapsed this dir three weeks ago" noise.
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
        let set = self.git_collapsed_dirs.entry(target).or_default();
        if !set.remove(&dir) {
            set.insert(dir);
        }
        cx.notify();
    }
}
