//! A file pane's git status letter, projected from the lane's cached
//! status — the one derivation the open path and every git refresh share.

use std::path::PathBuf;

use daruda_store::project::LaneRef;
use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::left_dock::file_tree_ops::repo_status_index;
use crate::workspace::main_area::file_view_pane::DiffSource;

/// Absolutise a file pane's path against its lane root. Every live opener
/// passes an absolute path; only old session state can still carry a
/// lane-relative one.
pub(super) fn abs_pane_path(lane_root: &std::path::Path, path: &std::path::Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        lane_root.join(path)
    }
}

impl Workspace {
    /// `absolute path → git status char` for `target`. A file pane's path is
    /// absolute whichever view opened it, and git's paths resolve against the
    /// working-tree root — so one conversion covers both, including a change
    /// outside a subdirectory lane that the Git Changes view still lists.
    pub(super) fn status_index_by_abs(
        &self,
        target: LaneRef,
    ) -> std::collections::HashMap<PathBuf, char> {
        let Some(lane) = self.lane_for(target) else {
            return std::collections::HashMap::new();
        };
        let paths = lane.paths();
        repo_status_index(self.lane_git_worktree(target))
            .into_iter()
            .map(|(rel, status)| (paths.from_git_status(&rel), status))
            .collect()
    }

    /// The git status char for `path` in `target` — `None` when the file has
    /// no pending change, the lane's status hasn't been fetched yet, or
    /// `source` is a range (which carries its own; see `PaneFileView::status`).
    ///
    /// The single derivation of a pane's `live_status`: `open_pane_file_view`
    /// stamps it on open (including onto a reused tab) and
    /// [`Self::sync_file_pane_statuses`] re-stamps every open pane on each git
    /// refresh. Openers deliberately do **not** pass a status in — it is a
    /// projection of the lane's cached status, so a caller-supplied copy could only
    /// ever be the same value or a staler one. Four of the eight call sites
    /// had no way to know it and passed `None`, which since the toolbar's mode
    /// strip started gating the Changes segment on `is_some()` meant a changed
    /// file opened from the agent chat, a skill, or a task offered no diff.
    pub(super) fn git_status_for_path(
        &self,
        target: LaneRef,
        path: &std::path::Path,
        source: &DiffSource,
    ) -> Option<char> {
        if !source.is_live() {
            return None;
        }
        let lane_root = self.lane_for(target).map(|w| w.path.clone())?;
        self.status_index_by_abs(target)
            .get(&abs_pane_path(&lane_root, path))
            .copied()
    }

    /// Re-derive every open file pane's `file_status` for `target` from the
    /// lane's freshly-fetched `git status`.
    ///
    /// `file_status` answers "does this file have a pending change?" — the
    /// viewer toolbar draws its badge from it and offers the Changes segment
    /// only when it is `Some` — but it is written once, at open time, and is
    /// deliberately not persisted. Without this pass a pane restored from disk
    /// never offers Changes, and a pane left open across an edit or a commit
    /// keeps whatever the opening click happened to see. The open path stamps
    /// the same value from the same index — see [`Self::git_status_for_path`].
    pub(super) fn sync_file_pane_statuses(&mut self, target: LaneRef, cx: &mut Context<Self>) {
        let Some(lane_root) = self.lane_for(target).map(|w| w.path.clone()) else {
            return;
        };
        let by_abs = self.status_index_by_abs(target);

        let Some(runtime) = self.main_area.runtimes.get_mut(&target) else {
            return;
        };
        let mut changed = false;
        for pane in runtime.panes.iter_mut() {
            let Some(fv) = pane.file_view_mut() else {
                continue;
            };
            // A range pane reads its own letter; it has no live one.
            let next = if fv.source.is_live() {
                by_abs.get(&abs_pane_path(&lane_root, &fv.path)).copied()
            } else {
                None
            };
            if fv.live_status != next {
                fv.live_status = next;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }
}
