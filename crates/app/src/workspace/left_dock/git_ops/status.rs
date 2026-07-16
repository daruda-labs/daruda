//! `git status` refresh, merge finalization, repo-root lookup, and
//! commit-footer button-state sync.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Repo root of the targeted lane, or `None` when it isn't
    /// git-backed.
    pub(in crate::workspace) fn git_repo_root_for(&self, target: LaneRef) -> Option<PathBuf> {
        self.lane_for(target).and_then(|w| match &w.kind {
            daruda_store::project::LaneKind::Git { repo_root, .. } => Some(repo_root.clone()),
            daruda_store::project::LaneKind::Default => None,
        })
    }

    /// Kick off a background `git status` for `target` and update
    /// `git_status_cache` when done. No-op for non-git worktrees.
    ///
    /// Concurrency guard: at most one in-flight task per lane. A second call
    /// while one runs sets `git_status_pending_repeat`, which the in-flight
    /// task drains by re-invoking itself once before returning. This collapses
    /// watcher-event bursts into at most two invocations (the running one + a
    /// repeat capturing everything that landed during the run).
    pub(in crate::workspace) fn refresh_git_status(
        &mut self,
        target: LaneRef,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let path = wt.path.clone();
        if !wt.is_git() {
            return;
        }

        if !self.git_status_in_flight.insert(target) {
            // Already running — request a re-fire on completion.
            self.git_status_pending_repeat.insert(target);
            return;
        }

        let path_for_report = path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::lane::git::git_status(&path),
            move |ws, result, cx| {
                ws.git_status_in_flight.remove(&target);
                match result {
                    Ok(data) => {
                        // Propagate an external branch switch into the lane's
                        // recorded branch. The left-dock label reads
                        // `Lane.kind.branch` instead of re-probing on render,
                        // so this refresh is the only path keeping it current.
                        ws.reconcile_lane_branch(target, data.branch.as_deref(), cx);
                        ws.git_status_cache.insert(target, data);
                        // Refreshed status updates the file badges.
                        ws.invalidate_visible_files_cache(target);
                        // Commit button reflects the active lane's staged count.
                        if target == ws.active {
                            ws.sync_commit_buttons(cx);
                        }
                    }
                    Err(e) => {
                        // A status failure staleness-freezes the whole Git
                        // Changes panel, meeting the CLAUDE.md "core function
                        // broke" bar for Error severity.
                        let report = ErrorReport::new("git status failed")
                            .severity(ErrorSeverity::Error)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path_for_report))
                            .dedup("git.status")
                            .build();
                        ws.report_error(report, cx);
                    }
                }
                cx.notify();
                // Drain the repeat slot — re-fire once for events that
                // landed while the previous run was busy.
                if ws.git_status_pending_repeat.remove(&target) {
                    ws.refresh_git_status(target, cx);
                }
            },
        )
        .detach();
    }

    /// Propagate the live `git status` branch into the lane's recorded
    /// `kind.branch` so an external `git checkout` doesn't leave the label
    /// stale. Rewrites and persists only when the branch actually drifted, so
    /// watcher refresh bursts don't schedule a save each time. Routes through
    /// [`crate::lane::Lane::set_kind`] so the derived `is_main` mirror can't
    /// drift; `repo_root` / `worktree_root` are preserved.
    pub(in crate::workspace) fn reconcile_lane_branch(
        &mut self,
        target: LaneRef,
        live_branch: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let Some(lane) = self.lane_for(target) else {
            return;
        };
        let daruda_store::project::LaneKind::Git {
            branch,
            repo_root,
            worktree_root,
        } = &lane.kind
        else {
            return;
        };
        if branch.as_deref() == live_branch {
            return;
        }
        let new_kind = daruda_store::project::LaneKind::Git {
            branch: live_branch.map(str::to_owned),
            repo_root: repo_root.clone(),
            worktree_root: worktree_root.clone(),
        };
        self.mutate_durable(cx, |ws, _| {
            if let Some(lane) = ws.lane_for_mut(target) {
                lane.set_kind(new_kind);
            }
        });
    }

    /// Refresh git status for the merge target lane after a successful merge.
    pub(in crate::workspace) fn finalize_merge(&mut self, target: LaneRef, cx: &mut Context<Self>) {
        self.refresh_git_status(target, cx);
    }

    /// Recompute the disabled state of the commit footer's split-button:
    /// disabled when an op is in flight or nothing is staged (surfacing the
    /// no-op up-front instead of failing at the git CLI). The dropdown Amend
    /// item shares this state, since `DropdownButton` ties the caret to the
    /// primary button.
    pub(in crate::workspace) fn sync_commit_buttons(&mut self, cx: &mut Context<Self>) {
        let staged_count = self
            .git_status_cache
            .get(&self.active)
            .map(|s| s.staged.len())
            .unwrap_or(0);
        let in_flight = self.git_op_in_flight;
        // A normal commit needs staged changes; an amend can be message-only
        // (no staged changes), so amend mode only blocks while in flight.
        let commit_disabled = in_flight || (!self.is_amend_mode() && staged_count == 0);
        self.git_commit_input.update(cx, |panel, cx| {
            panel.set_action_disabled("commit", commit_disabled, cx);
        });
    }
}
