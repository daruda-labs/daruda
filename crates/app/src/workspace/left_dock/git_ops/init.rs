//! `git init` for non-git worktrees, with post-init re-probe.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{LaneId, LaneRef};
use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Run `git init` in a non-git worktree, then re-probe so the
    /// lane's `kind` flips from `Default` to `Git` and the Git
    /// Changes view starts surfacing changes immediately. No-op for
    /// lanes that are already git-backed.
    pub(in crate::workspace) fn init_git_repo(&mut self, lane_id: LaneId, cx: &mut Context<Self>) {
        if self.git_op_in_flight {
            return;
        }
        let target = LaneRef {
            project: self.active.project,
            lane: lane_id,
        };
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        if wt.is_git() {
            return;
        }
        let path = wt.path.clone();
        self.git_op_in_flight = true;
        cx.notify();
        let path_for_report = path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || -> Result<Option<_>, crate::lane::git::GitError> {
                crate::lane::git::git_init(&path)?;
                Ok(crate::lane::git::probe_repo(&path))
            },
            move |ws, result, cx| {
                ws.git_op_in_flight = false;
                // `git_op_in_flight` drives the Git view's disabled/loading
                // state; the `Ok(None)` / `Err` arms below have no other
                // workspace notify, so render the workspace here and let the
                // left-dock staging diff invalidate the `.cached()` dock.
                cx.notify();
                match result {
                    Ok(Some(probe)) => {
                        // Pick the matching lane entry's branch from
                        // `git worktree list` so the header label flips
                        // from "detached" to the actual branch name (git
                        // init defaults to `main`, but a user-configured
                        // `init.defaultBranch` may differ — read what git
                        // actually decided rather than guessing).
                        if let Some(wt) = ws
                            .active_project_mut()
                            .and_then(|p| p.lanes.iter_mut().find(|w| w.id == lane_id))
                        {
                            let probed_entry = probe
                                .lanes
                                .iter()
                                .find(|p| p.path == wt.path)
                                .or_else(|| probe.lanes.first());
                            let probed_branch = probed_entry.and_then(|p| p.branch.clone());
                            // Freshly init'd repo: the toplevel from the
                            // probe entry IS this lane's root. Fall
                            // back to `wt.path` if for some reason the
                            // entry list is empty.
                            let worktree_root = probed_entry
                                .map(|p| p.path.clone())
                                .unwrap_or_else(|| wt.path.clone());
                            wt.set_kind(daruda_store::project::LaneKind::Git {
                                repo_root: probe.repo_root,
                                branch: probed_branch,
                                worktree_root,
                            });
                        }
                        ws.refresh_git_status(target, cx);
                    }
                    Ok(None) => {
                        // `git init` succeeded — the repo is on disk and
                        // usable; only the follow-up probe that flips
                        // `LaneKind::Default → Git` failed. The user
                        // can re-open the project and the next probe
                        // will pick it up. Warning, not Error.
                        let report = ErrorReport::new("git init succeeded but probe failed")
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.init.probe")
                            .build();
                        ws.report_error(report, cx);
                    }
                    Err(e) => {
                        let report = ErrorReport::new("git init failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.init")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
            },
        )
        .detach();
    }
}
