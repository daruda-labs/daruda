//! The against-base axis: what a lane committed since the branch it left.
//!
//! Recomputed off the tracking refresh, because only a ref move — a commit,
//! a rebase, a fetch into the common dir — can change it. The diff is skipped
//! when neither tip moved, which is what most ref events amount to here.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::{Context, Window};

use std::path::PathBuf;

use crate::lane::git::base::{self, AgainstBase, BaseProblem, BaseTips, RangeFile};
use crate::lane::paths::LanePaths;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::DiffSource;
use crate::workspace::main_area::tab_ops::OpenIntent;

/// The pane a row of `found` opens: `file` at its absolute path, pinned to
/// the pair the list was read from. The one rule both the open and the row's
/// highlight read, so a row lights up exactly for the pane it would open.
pub(in crate::workspace) fn range_pane_for(
    found: &AgainstBase,
    file: &RangeFile,
    paths: &LanePaths<'_>,
) -> (PathBuf, DiffSource) {
    let source = DiffSource::Range {
        from: found.merge_base.clone(),
        to: found.tips.head.clone(),
        old_path: file.old_path.as_ref().map(|p| paths.from_git_status(p)),
        status: file.status,
    };
    (paths.from_git_status(&file.path), source)
}

/// What one background read found.
enum Read {
    /// Both tips are where the cached result left them.
    Unchanged,
    Fresh(Result<AgainstBase, BaseProblem>),
}

impl Workspace {
    /// The base a lane is compared against: the one it was created from,
    /// else the project's chosen base, else its detected default branch.
    fn base_name_for(&self, target: LaneRef) -> Option<String> {
        let lane = self.lane_for(target)?;
        let project = self.project_for(target.project)?;
        lane.base_ref
            .clone()
            .or_else(|| project.base_branch.clone())
            .or_else(|| project.default_branch.clone())
    }

    /// Re-read the against-base axis for `target`. Only the active lane is
    /// read: the view shows one lane at a time, and activation refreshes
    /// tracking, which lands here again.
    pub(in crate::workspace) fn refresh_against_base(
        &mut self,
        target: LaneRef,
        cx: &mut Context<Self>,
    ) {
        if target != self.active {
            return;
        }
        let Some(lane) = self.lane_for(target) else {
            return;
        };
        if !lane.is_git() {
            return;
        }
        let path = lane.path.clone();
        let base_name = self.base_name_for(target);
        let state = self.lane_scoped_mut(target);
        let branch = state.git.tracking.as_ref().and_then(|t| t.branch.clone());
        let cached: Option<BaseTips> = match state.git.against_base.as_deref() {
            Some(Ok(found)) => Some(found.tips.clone()),
            _ => None,
        };
        if !state.git.against_base_refresh.claim() {
            return;
        }

        let path_for_report = path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || {
                let tips = match base::base_tips(&path, base_name.as_deref(), branch.as_deref()) {
                    Ok(tips) => tips,
                    Err(problem) => return Read::Fresh(Err(problem)),
                };
                if cached.as_ref() == Some(&tips) {
                    return Read::Unchanged;
                }
                Read::Fresh(base::changes_since(&path, tips))
            },
            move |ws, read, cx| {
                let pending = ws
                    .lane_scoped
                    .get_mut(&target)
                    .is_some_and(|state| state.git.against_base_refresh.release());
                if let Read::Fresh(result) = read {
                    if let Err(BaseProblem::Git(message)) = &result {
                        let report = ErrorReport::new(
                            crate::surface::strings::error_git_against_base_failed(),
                        )
                        .severity(ErrorSeverity::Warning)
                        .message(message.clone())
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path_for_report))
                        .dedup("git.against_base")
                        .build();
                        ws.report_error(report, cx);
                    }
                    // `get_mut`, not `lane_scoped_mut`: a read that outlived
                    // its lane must not bring the lane's state back.
                    if let Some(state) = ws.lane_scoped.get_mut(&target) {
                        state.git.against_base = Some(std::sync::Arc::new(result));
                        cx.notify();
                    }
                }
                if pending {
                    ws.refresh_against_base(target, cx);
                }
            },
        )
        .detach();
    }

    /// The project's base may have just become known (default-branch
    /// detection lands after the lane is shown); re-read the active lane if
    /// it belongs to `project`. No ref moved, so nothing else would.
    pub(in crate::workspace) fn refresh_against_base_for_project(
        &mut self,
        project: daruda_store::project::ProjectId,
        cx: &mut Context<Self>,
    ) {
        if self.active.project == project {
            self.refresh_against_base(self.active, cx);
        }
    }

    /// Open one against-base file in the diff viewer, pinned to the commits
    /// its row was listed from.
    pub(in crate::workspace) fn open_against_base_file(
        &mut self,
        target: LaneRef,
        repo_rel: std::path::PathBuf,
        intent: OpenIntent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(lane) = self.lane_for(target) else {
            return;
        };
        let paths = lane.paths();
        let Some(Ok(found)) = self
            .lane_scoped
            .get(&target)
            .and_then(|state| state.git.against_base.as_deref())
        else {
            return;
        };
        let Some(file) = found.files.iter().find(|f| f.path == repo_rel) else {
            return;
        };
        let (abs, source) = range_pane_for(found, file, &paths);
        self.open_git_file_diff(target.lane, abs, source, intent, window, cx);
    }

    /// A click on an against-base row: the panel takes focus, as for the
    /// working-tree rows; a double click hands the file to the OS.
    pub(in crate::workspace) fn on_against_base_row_click(
        &mut self,
        target: LaneRef,
        repo_rel: std::path::PathBuf,
        click_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_changes_panel_focus.clone().focus(window, cx);
        if click_count >= 2 {
            if let Some(abs) = self
                .lane_for(target)
                .map(|lane| lane.paths().from_git_status(&repo_rel))
            {
                self.open_file_externally(target, abs, cx);
            }
            return;
        }
        self.open_against_base_file(target, repo_rel, OpenIntent::Preview, window, cx);
    }

    /// Fold or unfold the against-base section of `target`'s Git view.
    pub(in crate::workspace) fn toggle_against_base_collapse(
        &mut self,
        target: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.lane_scoped_mut(target).git;
        state.against_base_collapsed = !state.against_base_collapsed;
        cx.notify();
    }
}
