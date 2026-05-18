//! `git status` refresh, merge finalization, repo-root lookup, and
//! commit-footer button-state sync.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::WorktreeId;
use gpui::Context;

use crate::workspace::Workspace;

impl Workspace {
    /// Repo root of worktree `id`, or `None` when it isn't git-backed.
    pub(in crate::workspace) fn git_repo_root_for(&self, id: WorktreeId) -> Option<PathBuf> {
        self.worktrees
            .iter()
            .find(|w| w.id == id)
            .and_then(|w| match &w.kind {
                daruda_store::project::WorktreeKind::Git { repo_root, .. } => {
                    Some(repo_root.clone())
                }
                daruda_store::project::WorktreeKind::Default => None,
            })
    }

    /// Kick off a background `git status` for `worktree_id` and update
    /// `git_status_cache` when done. No-op for non-git worktrees.
    ///
    /// Concurrency guard: at most one in-flight task per worktree. A
    /// second call while one is running sets `git_status_pending_repeat`,
    /// which the in-flight task drains by re-invoking itself once
    /// before returning. This collapses watcher-event bursts (a `cargo
    /// build` can emit 30+ debounced events per second) into at most
    /// two `git status` invocations: the running one + one repeat that
    /// captures everything that landed during the run.
    pub(in crate::workspace) fn refresh_git_status(
        &mut self,
        worktree_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.worktrees.iter().find(|w| w.id == worktree_id) else {
            return;
        };
        let path = wt.path.clone();
        if !wt.is_git() {
            return;
        }

        if !self.git_status_in_flight.insert(worktree_id) {
            // Already running — request a re-fire on completion.
            self.git_status_pending_repeat.insert(worktree_id);
            return;
        }

        let path_for_report = path.clone();
        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || crate::worktree::git::git_status(&path),
            move |ws, result, cx| {
                ws.git_status_in_flight.remove(&worktree_id);
                match result {
                    Ok(data) => {
                        ws.git_status_cache.insert(worktree_id, data);
                        // Trigger #6 — git status refresh updates badges.
                        ws.invalidate_visible_files_cache(worktree_id);
                        // Commit button reflects the active worktree's
                        // staged count — recompute when that worktree's
                        // cache changes.
                        if worktree_id == ws.active_worktree_id {
                            ws.sync_commit_buttons(cx);
                        }
                    }
                    Err(e) => {
                        // `git status` failure disables the entire Git Changes
                        // panel — staged/unstaged lists, commit footer, file
                        // badges all go stale. That meets the CLAUDE.md
                        // "core function broke" bar for Error severity.
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
                // Drain the repeat slot — re-fire once to capture
                // events that landed while the previous run was busy.
                if ws.git_status_pending_repeat.remove(&worktree_id) {
                    ws.refresh_git_status(worktree_id, cx);
                }
            },
        )
        .detach();
    }

    /// Refresh git status for the merge target worktree after a successful merge.
    pub(in crate::workspace) fn finalize_merge(
        &mut self,
        target_wt_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        self.refresh_git_status(target_wt_id, cx);
    }

    /// Recompute and push the disabled state of the commit footer's
    /// split-button.
    ///
    /// The `commit` split-button is disabled when an op is in flight or
    /// no files are staged. Committing with nothing staged would just
    /// fail at the git CLI, so the UI surfaces the "no-op" state
    /// up-front. The Amend item inside the dropdown shares the same
    /// disabled state because `DropdownButton` ties the caret to the
    /// primary button's enablement.
    pub(in crate::workspace) fn sync_commit_buttons(&mut self, cx: &mut Context<Self>) {
        let staged_count = self
            .git_status_cache
            .get(&self.active_worktree_id)
            .map(|s| s.staged.len())
            .unwrap_or(0);
        let in_flight = self.git_op_in_flight;
        let commit_disabled = in_flight || staged_count == 0;
        self.git_commit_input.update(cx, |panel, cx| {
            panel.set_action_disabled("commit", commit_disabled, cx);
        });
    }
}
