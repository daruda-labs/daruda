//! Index mutations — stage / unstage / discard.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{LaneId, LaneRef};
use gpui::{Context, Window};

use crate::path_ext::PathExt;
use crate::surface::strings as app_strings;
use crate::ui::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;
use crate::workspace::left_dock::git_ops::lock::GitLock;

impl Workspace {
    /// Stage a single file from the working tree into the index.
    ///
    /// Runs from the lane's git toplevel so (a) linked lanes stage into their
    /// own per-lane index rather than the shared `repo_root`, and (b) porcelain
    /// paths (which are toplevel-relative) resolve correctly even for an
    /// anchored main lane whose `wt.path` is a subdirectory.
    pub(in crate::workspace) fn stage_file(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_add(&wt_top, &path),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(app_strings::error::git_add_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("lane", redact_home(&wt_for_report))
                        .with_context("path", redact_home(&path_for_report))
                        .dedup("git.stage")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Remove a file from the index (unstage), keeping working-tree changes.
    ///
    /// Runs from the lane's git toplevel — see [`Self::stage_file`] for
    /// why `wt.path` and the shared `repo_root` are both unsuitable.
    pub(in crate::workspace) fn unstage_file(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_restore_staged(&wt_top, &path),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(app_strings::error::git_restore_staged_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("lane", redact_home(&wt_for_report))
                        .with_context("path", redact_home(&path_for_report))
                        .dedup("git.unstage")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Stage every path in `paths` in one git invocation. Used by the
    /// per-directory "stage all in this dir" checkbox.
    pub(in crate::workspace) fn stage_paths(
        &mut self,
        lane_id: LaneId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_add_paths(&wt_top, &paths),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(app_strings::error::git_add_paths_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("lane", redact_home(&wt_for_report))
                        .with_context("count", paths_count.to_string())
                        .dedup("git.stage_paths")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Unstage every path in `paths` in one git invocation. Companion to
    /// [`Self::stage_paths`] for the per-dir "unstage all" toggle.
    pub(in crate::workspace) fn unstage_paths(
        &mut self,
        lane_id: LaneId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_restore_staged_paths(&wt_top, &paths),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report =
                        ErrorReport::new(app_strings::error::git_restore_staged_paths_failed())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("lane", redact_home(&wt_for_report))
                            .with_context("count", paths_count.to_string())
                            .dedup("git.unstage_paths")
                            .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Stage all unstaged and untracked files (`git add --all`).
    pub(in crate::workspace) fn stage_all(&mut self, lane_id: LaneId, cx: &mut Context<Self>) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let path_for_report = wt_top.clone();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_add_all(&wt_top),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(app_strings::error::git_add_all_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path_for_report))
                        .dedup("git.stage_all")
                        .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Unstage all files (`git restore --staged .`).
    pub(in crate::workspace) fn unstage_all(&mut self, lane_id: LaneId, cx: &mut Context<Self>) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let Some(wt_top) = wt.git_worktree_root().map(std::path::Path::to_path_buf) else {
            return;
        };
        let path_for_report = wt_top.clone();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::git_restore_all_staged(&wt_top),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report =
                        ErrorReport::new(app_strings::error::git_restore_staged_all_failed())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.unstage_all")
                            .build();
                    ws.report_error(report, cx);
                }
                cx.notify();
            },
        );
    }

    /// Confirm, then put `path` back to HEAD — staged and unstaged changes
    /// alike. The body says whether the file is restored or deleted: both
    /// are irreversible.
    pub(in crate::workspace) fn on_discard_file(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(window, cx);
        if self.git_lock_held(GitLock::Index) {
            return;
        }
        if !self
            .active_project()
            .is_some_and(|p| p.lanes.iter().any(|w| w.id == lane_id))
        {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let filename = path.file_name_lossy();
        let body = match self.discard_kind(target, &path) {
            DiscardKind::Untracked => app_strings::git::confirm_discard_untracked_body(&filename),
            DiscardKind::Added => app_strings::git::confirm_discard_added_body(&filename),
            DiscardKind::Tracked => app_strings::git::confirm_discard_tracked_body(&filename),
        };

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git::confirm_discard_title(),
            body,
            app_strings::git::confirm_discard_ok(),
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let pinned = vec![path.clone()];
                    ws.update(app_cx, |ws, cx| ws.discard_changes(lane_id, pinned, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Confirm, then put every changed file in the lane back to HEAD and
    /// delete its untracked files. Ignored files are left alone. The files
    /// counted are the ones discarded: a file that appears while the dialog
    /// is up is not among them.
    pub(in crate::workspace) fn on_discard_all(
        &mut self,
        lane_id: LaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_lock_held(GitLock::Index) {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(status) = self.lane_git_worktree(target) else {
            return;
        };
        let mut pinned: Vec<PathBuf> = Vec::new();
        let mut untracked = 0;
        for entry in status.staged.iter().chain(&status.unstaged) {
            if pinned.contains(&entry.path) {
                continue;
            }
            pinned.push(entry.path.clone());
            if entry.x == '?' {
                untracked += 1;
            }
        }
        if pinned.is_empty() {
            return;
        }
        let tracked = pinned.len() - untracked;
        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git::confirm_discard_all_title(),
            app_strings::git::confirm_discard_all_body(tracked, untracked),
            app_strings::git::confirm_discard_ok(),
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let pinned = pinned.clone();
                    ws.update(app_cx, |ws, cx| ws.discard_changes(lane_id, pinned, cx));
                }
            },
            window,
            cx,
        );
    }

    /// Put `pinned` back to HEAD. The status is read afresh off the UI thread,
    /// so a path an agent committed or changed meanwhile is judged by what it
    /// is now. The caller has already asked.
    pub(in crate::workspace) fn discard_changes(
        &mut self,
        lane_id: LaneId,
        pinned: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt_top) = self
            .lane_for(target)
            .and_then(|wt| wt.git_worktree_root())
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        if pinned.is_empty() {
            return;
        }
        let path_for_report = wt_top.clone();
        self.spawn_locked_git_work(
            GitLock::Index,
            target,
            cx,
            move || crate::lane::git::discard::discard(&wt_top, &pinned),
            move |ws, result, cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(app_strings::error::git_restore_failed())
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path_for_report))
                        .dedup("git.discard")
                        .build();
                    ws.report_error(report, cx);
                    cx.notify();
                }
            },
        );
    }

    /// What discarding `path` does, read off the panel's status — for the
    /// confirm's wording only; the discard itself re-reads it. A merge
    /// conflict sits in the unstaged set, so a staged addition is a real one.
    fn discard_kind(&self, target: LaneRef, path: &std::path::Path) -> DiscardKind {
        let Some(status) = self.lane_git_worktree(target) else {
            return DiscardKind::Tracked;
        };
        if status
            .staged
            .iter()
            .any(|e| e.path == path && matches!(e.x, 'A' | 'C'))
        {
            return DiscardKind::Added;
        }
        if status.unstaged.iter().any(|e| e.path == path && e.x == '?') {
            return DiscardKind::Untracked;
        }
        DiscardKind::Tracked
    }
}

/// How a discard confirm describes the file it acts on.
enum DiscardKind {
    /// Goes back to the committed version.
    Tracked,
    /// Added since the last commit, so it is deleted.
    Added,
    /// Never tracked, so it is deleted.
    Untracked,
}
