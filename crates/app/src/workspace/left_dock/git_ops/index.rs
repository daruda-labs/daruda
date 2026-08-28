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
        if self.git_stage_in_flight {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_add(&wt_top, &path),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new(app_strings::error_git_add_failed())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("lane", redact_home(&wt_for_report))
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.stage")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
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
        if self.git_stage_in_flight {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        let wt_for_report = wt_top.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_restore_staged(&wt_top, &path),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report =
                            ErrorReport::new(app_strings::error_git_restore_staged_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("lane", redact_home(&wt_for_report))
                                .with_context("path", redact_home(&path_for_report))
                                .dedup("git.unstage")
                                .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Stage every path in `paths` in one git invocation. Used by the
    /// per-directory "stage all in this dir" checkbox.
    pub(in crate::workspace) fn stage_paths(
        &mut self,
        lane_id: LaneId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight || paths.is_empty() {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_add_paths(&wt_top, &paths),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new(app_strings::error_git_add_paths_failed())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("lane", redact_home(&wt_for_report))
                            .with_context("count", paths_count.to_string())
                            .dedup("git.stage_paths")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Unstage every path in `paths` in one git invocation. Companion to
    /// [`Self::stage_paths`] for the per-dir "unstage all" toggle.
    pub(in crate::workspace) fn unstage_paths(
        &mut self,
        lane_id: LaneId,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight || paths.is_empty() {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let wt_for_report = wt_top.clone();
        let paths_count = paths.len();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_restore_staged_paths(&wt_top, &paths),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report =
                            ErrorReport::new(app_strings::error_git_restore_staged_paths_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("lane", redact_home(&wt_for_report))
                                .with_context("count", paths_count.to_string())
                                .dedup("git.unstage_paths")
                                .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Stage all unstaged and untracked files (`git add --all`).
    pub(in crate::workspace) fn stage_all(&mut self, lane_id: LaneId, cx: &mut Context<Self>) {
        if self.git_stage_in_flight {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_top.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_add_all(&wt_top),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new(app_strings::error_git_add_all_failed())
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.stage_all")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Unstage all files (`git restore --staged .`).
    pub(in crate::workspace) fn unstage_all(&mut self, lane_id: LaneId, cx: &mut Context<Self>) {
        if self.git_stage_in_flight {
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
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_top.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_restore_all_staged(&wt_top),
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let report =
                            ErrorReport::new(app_strings::error_git_restore_staged_all_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("path", redact_home(&path_for_report))
                                .dedup("git.unstage_all")
                                .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
            },
        )
        .detach();
    }

    /// Open a confirm dialog before discarding working-tree changes for a
    /// file. The actual git operation runs in [`Self::do_discard_file`] only
    /// after the user confirms. Both untracked deletes (`git clean -f`)
    /// and tracked restores (`git restore`) are irreversible, so the
    /// confirm body spells out which one the user is about to do.
    pub(in crate::workspace) fn on_discard_file(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        is_untracked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_context_menu(window, cx);
        if self.git_stage_in_flight {
            return;
        }
        if !self
            .active_project()
            .is_some_and(|p| p.lanes.iter().any(|w| w.id == lane_id))
        {
            return;
        }
        let filename = path.file_name_lossy();
        let body = app_strings::git_confirm_discard_body(&filename, is_untracked);

        let weak = cx.weak_entity();
        open_confirm_dialog(
            app_strings::git_confirm_discard_title(),
            body,
            app_strings::git_confirm_discard_ok(),
            ButtonVariant::Danger,
            move |_, _window, app_cx| {
                if let Some(ws) = weak.upgrade() {
                    let path = path.clone();
                    ws.update(app_cx, |ws, cx| {
                        ws.do_discard_file(lane_id, path, is_untracked, cx)
                    });
                }
            },
            window,
            cx,
        );
    }

    /// Discard working-tree changes for a file. For untracked files, deletes
    /// the file (`git clean -f`); for tracked files, restores the last committed
    /// state (`git restore`). Caller must have obtained user confirmation via
    /// [`Self::on_discard_file`].
    fn do_discard_file(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        is_untracked: bool,
        cx: &mut Context<Self>,
    ) {
        if self.git_stage_in_flight {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let wt_path = wt.path.clone();
        let repo_root = self.git_repo_root_for(target);
        // `path` is a repo-root-relative pathspec (from git status output).
        // `git restore`/`git clean` must run from the lane directory with a
        // lane-relative path — use LanePaths for the two-step conversion.
        let paths = crate::lane::paths::LanePaths {
            wt_path: &wt_path,
            repo_root: repo_root.as_deref(),
        };
        let abs = paths.from_git_status(&path);
        let wt_rel_path = paths.to_wt_relative(&abs).unwrap_or(path);
        self.git_stage_in_flight = true;
        cx.notify();
        let path_for_report = wt_path.clone();
        let rel_for_report = wt_rel_path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                if is_untracked {
                    crate::lane::git::git_clean_untracked(&wt_path, &wt_rel_path)
                } else {
                    crate::lane::git::git_discard_working(&wt_path, &wt_rel_path)
                }
            },
            move |ws, result, cx| {
                ws.git_stage_in_flight = false;
                match result {
                    Ok(()) => {
                        ws.refresh_git_status(target, cx);
                    }
                    Err(e) => {
                        let title = if is_untracked {
                            "git clean -f failed"
                        } else {
                            "git restore failed"
                        };
                        let report = ErrorReport::new(title)
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .with_context("file", redact_home(&rel_for_report))
                            .dedup("git.discard")
                            .build();
                        ws.report_error(report, cx);
                        cx.notify();
                    }
                }
            },
        )
        .detach();
    }
}
