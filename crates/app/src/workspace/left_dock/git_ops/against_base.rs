//! The against-base axis: what a lane committed since the branch it left.
//!
//! Recomputed off the tracking refresh, because only a ref move — a commit,
//! a rebase, a fetch into the common dir — can change it. The diff is skipped
//! when neither tip moved, which is what most ref events amount to here.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::lane::git::base::{self, AgainstBase, BaseProblem, BaseTips};
use crate::workspace::Workspace;

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
        let cached: Option<BaseTips> = match &state.git.against_base {
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
                    ws.lane_scoped_mut(target).git.against_base = Some(result);
                    cx.notify();
                }
                if pending {
                    ws.refresh_against_base(target, cx);
                }
            },
        )
        .detach();
    }
}
