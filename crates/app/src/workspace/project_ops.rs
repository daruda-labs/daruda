//! Project-level mutation ops on [`Workspace`] — add / close / move.
//!
//! Lane-scoped operations live in `lane_ops.rs`; this module
//! owns the project boundary itself (registering a new project root
//! with the workspace, removing one without tearing down the window).

use std::path::{Path, PathBuf};

use daruda_store::project::{LaneRef, ProjectId, ProjectUuid, WindowOpenPolicy};
use gpui::{AppContext as _, Context, Window};

use super::Workspace;

/// Scan the on-disk `projects/` pool for a `ProjectState` whose `root`
/// matches `root`. If found, return its UUID so a new workspace can
/// reference the same `ProjectState` file (policy B: shared intrinsic
/// project data, workspace-local overrides).
///
/// Returns `None` if no existing project file references this root.
/// Disk cost: one scan of `projects/` per call; small in practice
/// (single-digit MS for typical project counts). If profiling shows
/// this becoming a hotspot, cache the mapping in a `Workspace` field.
pub(in crate::workspace) fn find_existing_project_uuid_for_root(
    data_dir: &Path,
    root: &Path,
) -> Option<ProjectUuid> {
    let mut found = None;
    daruda_store::project::for_each_project_state_in(data_dir, |p| {
        if found.is_none() && p.root == root {
            found = Some(p.uuid);
        }
    });
    found
}

impl Workspace {
    /// Add a freshly-opened project to this workspace and activate its
    /// first lane.
    ///
    /// Mints a new [`ProjectId`] from the monotonic `next_project_id`
    /// counter, walks the filesystem at `root` to discover git
    /// lanes (or falls back to one default), and pushes the result
    /// onto `self.projects`. Then routes through `activate_worktree`
    /// to swap the live `MainAreaContext` over to the new lane —
    /// the previous active runtime is preserved in the inactive map.
    ///
    /// **Pre-condition (caller):** the same-workspace duplicate-root
    /// check has already run. Policy B explicitly allows the same root
    /// across multiple windows (each shares the on-disk `ProjectState`
    /// via a reused UUID — see `find_existing_project_uuid_for_root`),
    /// so the cross-window registry guard no longer applies. Same-window
    /// dedup is still the caller's responsibility: invoking this with a
    /// root already in `self.projects` will register a second runtime
    /// project under a new id and the UI then renders the same folder
    /// twice.
    pub(crate) fn add_project(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<LaneRef> {
        let new_id: ProjectId = self.next_project_id;
        self.next_project_id = self.next_project_id.checked_add(1)?;
        let tab_order = self.projects.len() as u32;
        // Policy B: when this root already has a `ProjectState` on disk
        // (from another workspace), reuse its UUID so the new runtime
        // project points at the canonical shared file. Lane-list
        // mutations from either workspace flow through the same
        // `<data_dir>/projects/<uuid>.json`.
        //
        // `ProjectUuid::default()` is the nil sentinel — we need a
        // freshly-minted v4, hence the explicit closure.
        #[allow(clippy::unwrap_or_default)]
        let uuid = find_existing_project_uuid_for_root(&self.data_dir, &root)
            .unwrap_or_else(ProjectUuid::new);
        let mut project = crate::project::Project::new_with_uuid(new_id, uuid, root);
        project.tab_order = tab_order;
        let target = project.first_worktree_ref();
        self.projects.push(project);
        // Activate the new lane. When `self.projects` was empty
        // before this call there is no prior runtime to freeze, but
        // `activate_worktree` is still the right path: it lazy-seeds
        // a pane at the new lane's path so the user lands on a
        // live shell immediately.
        if let Some(t) = target {
            // First project case: `self.active` is the default
            // (project=0, lane=0). `activate_worktree` skips when
            // `self.active == target`, but with monotonic ids the new
            // project's id is always > 0 the first time so this fires.
            // Manually set `self.active` to a sentinel that differs
            // from `target` so the swap path runs even when the
            // workspace previously had no live runtime to freeze.
            if self.projects.len() == 1 {
                self.active = LaneRef::default();
            }
            self.activate_worktree(t, window, cx);
        }
        // Empty closure: see group_ops.rs:83 for rationale. `activate_worktree`
        // consumes `&mut Window`, so the persist trigger has to land after
        // those borrows release.
        self.mutate_durable(cx, |_, _| {});
        target
    }

    /// Replace the workspace-level `window_open_policy` and persist.
    /// Called by the Open Project chooser modal when the user ticks
    /// "Don't ask again" after picking AddHere / NewWindow.
    pub(crate) fn set_window_open_policy(
        &mut self,
        policy: WindowOpenPolicy,
        cx: &mut Context<Self>,
    ) {
        if self.window_open_policy == policy {
            return;
        }
        self.window_open_policy = policy;
        self.mutate_durable(cx, |_, _| {});
    }

    /// Toggle the collapsed flag on a project. The project's lane
    /// list is hidden under its header when collapsed; clicking the
    /// project chevron in the left dock drives this. Persists through
    /// `mark_dirty_and_save`.
    pub(in crate::workspace) fn toggle_project_collapse(
        &mut self,
        project_id: ProjectId,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.iter_mut().find(|p| p.id == project_id) else {
            return;
        };
        project.is_collapsed = !project.is_collapsed;
        self.mutate_durable(cx, |_, _| {});
    }

    /// Current workspace-level "Open Project" policy. Read by the
    /// global `OpenFolder` handler to decide between adding to the
    /// current window vs. opening a fresh one.
    pub(crate) fn window_open_policy(&self) -> WindowOpenPolicy {
        self.window_open_policy
    }

    /// Open the project's root in a fresh daruda window without
    /// touching the current workspace. Used by the left-dock Project
    /// context menu's "Open in New Window" entry (§5.1).
    ///
    /// Policy B explicitly allows the same root in multiple windows
    /// (the two workspaces share the on-disk `ProjectState` via UUID
    /// reuse — see `find_existing_project_uuid_for_root`), so this
    /// path always spawns a fresh window without a cross-window dedup
    /// check.
    pub(in crate::workspace) fn open_project_in_new_window(
        &self,
        project_id: ProjectId,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.iter().find(|p| p.id == project_id) else {
            return;
        };
        let root = project.root.clone();
        let config = crate::settings_store::SettingsStore::global(cx).user_arc();
        let store_project = daruda_store::project::Project::from_path(&root);
        // Policy B: the new workspace will reuse this project's
        // existing UUID when it scans `<data_dir>/projects/` on first
        // `add_project` (or recreate the file fresh if scrub'd). No
        // cross-window dedup applies here — by construction the user
        // explicitly asked for a second window pointing at the same
        // root.
        let opts = crate::windows::build_window_options(&config);
        crate::windows::open_project_with_mode(
            config,
            None,
            Some(store_project),
            opts,
            crate::windows::OpenMode::NewWindow,
            None,
            cx,
        );
    }

    /// Rename the currently active project. No-op when the workspace
    /// has no projects (Welcome state) or when the new name equals the
    /// current one. Returns `true` when the rename mutated state.
    pub(in crate::workspace) fn rename_active_project(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(project) = self.active_project_mut() else {
            return false;
        };
        if project.name == name {
            return false;
        }
        project.name = name;
        self.mutate_durable(cx, |_, _| {});
        true
    }

    /// Remove the active project and route the workspace to the next
    /// project (or signal "close this window" when none remain).
    ///
    /// Returns `true` when a usable lane remains after the removal — the
    /// caller should keep the window open. Returns `false` when the
    /// removal leaves the workspace with nothing to show (no projects
    /// left, or every surviving project is lane-less); the caller closes
    /// the window, which routes to the Welcome screen.
    ///
    /// Inactive lanes from the removed project also drop out of
    /// `inactive_worktree_runtimes` so memory does not leak.
    pub(crate) fn close_active_project(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(project_id) = self.active_project().map(|p| p.id) else {
            return false;
        };
        // Forget every inactive runtime that belonged to the removed
        // project — the WorktreeRefs become dangling once the project
        // is gone.
        self.main_area
            .inactive_worktree_runtimes
            .retain(|key, _| key.project != project_id);
        // Drop per-lane caches for the closing project so they do
        // not leak across project deletes.
        self.git_status_cache
            .retain(|key, _| key.project != project_id);
        self.git_status_in_flight
            .retain(|key| key.project != project_id);
        self.git_status_pending_repeat
            .retain(|key| key.project != project_id);
        self.git_collapsed_dirs
            .retain(|key, _| key.project != project_id);
        self.git_changes_cursor
            .retain(|key, _| key.project != project_id);
        // Five FileTreeContext caches keyed by LaneRef — drop every
        // entry belonging to the removed project. The notify watchers
        // stop when their entries drop; the gitignore matchers and
        // visible-row caches are pure data, free to discard.
        self.file_tree
            .file_trees
            .retain(|key, _| key.project != project_id);
        self.file_tree
            .files_visible_cache
            .retain(|key, _| key.project != project_id);
        self.file_tree
            .file_watchers
            .retain(|key, _| key.project != project_id);
        self.file_tree
            .files_reload_queues
            .retain(|key, _| key.project != project_id);
        self.file_tree
            .files_gitignore_index
            .retain(|key, _| key.project != project_id);

        self.projects.retain(|p| p.id != project_id);

        // No more projects — clear the active runtime fields and tell
        // the caller to close the window.
        if self.projects.is_empty() {
            self.main_area.tabs.clear();
            self.main_area.panes.clear();
            self.main_area.active_tab_index = 0;
            self.main_area.tab_history.clear();
            self.main_area.focused_pane_id = 0;
            self.active = LaneRef::default();
            return false;
        }

        // Pick a fallback project (first remaining with a usable
        // lane). Iterating finds a valid snap_target even when
        // the natural first project's lane list is somehow empty.
        let Some(next_target) = self.projects.iter().find_map(|p| p.snap_target()) else {
            // No surviving project has a usable lane. Every Project is
            // constructed with at least one lane (bootstrap, and restore
            // re-discovers an empty list), so this is runtime corruption
            // that left nothing to display. Treat it as an empty
            // workspace — same outcome as the no-projects case above — so
            // the caller closes the window and the user lands on Welcome
            // rather than a blank viewport.
            self.main_area.tabs.clear();
            self.main_area.panes.clear();
            self.main_area.active_tab_index = 0;
            self.main_area.tab_history.clear();
            self.main_area.focused_pane_id = 0;
            self.active = LaneRef::default();
            self.mutate_durable(cx, |_, _| {});
            return false;
        };
        // Reset live runtime; the removed project's panes are gone for
        // good and their TabEntry ids hold no PaneIds we can reuse.
        // `self.active` is intentionally left pointing at the deleted
        // project's lane ref — its project_id is guaranteed
        // distinct from `next_target.project` (we just removed it from
        // `self.projects`), so `activate_worktree`'s same-target guard
        // doesn't fire. Resetting to `LaneRef::default()` here would
        // collide with the natural (project=0, lane=0) target of
        // the surviving first project and false-trigger the guard,
        // leaving main_area empty (regression covered by
        // `close_active_project_keeps_window_when_other_remain`).
        self.main_area.tabs.clear();
        self.main_area.panes.clear();
        self.main_area.active_tab_index = 0;
        self.main_area.tab_history.clear();
        self.main_area.focused_pane_id = 0;
        self.activate_worktree(next_target, window, cx);
        // `activate_worktree`'s freeze step wrote a dangling empty
        // runtime under the deleted project's lane ref. Drop it so
        // `inactive_worktree_runtimes` stays clean.
        self.main_area
            .inactive_worktree_runtimes
            .retain(|key, _| key.project != project_id);
        self.mutate_durable(cx, |_, _| {});
        true
    }

    /// Disk-cleanup variant of [`Self::close_active_project`]. Runs
    /// `git worktree remove` for every linked lane of the active
    /// project on the background executor, then strips the project
    /// directory via `fs::remove_dir_all` for any default-kind / left
    /// over directories. UI bookkeeping (`close_active_project` +
    /// window removal when it was the last project) happens after the
    /// disk work finishes.
    ///
    /// Errors are toast-reported; the project still gets unregistered
    /// even when one or more lanes fail to remove on disk — the
    /// dock entry going stale is the worse failure mode.
    pub(crate) fn delete_active_project_on_disk(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.active_project() else {
            return;
        };
        let project_id = project.id;
        let project_path = project.root.clone();
        // Snapshot the (repo_root, worktree_root) pairs for the git
        // lanes we'll remove. Default-kind lanes are skipped —
        // they're not git-managed, so `fs::remove_dir_all` on
        // `project_path` is sufficient.
        let removals: Vec<(PathBuf, PathBuf)> = project
            .lanes
            .iter()
            .filter_map(|wt| {
                if let daruda_store::project::LaneKind::Git {
                    repo_root,
                    worktree_root,
                    ..
                } = &wt.kind
                {
                    Some((repo_root.clone(), worktree_root.clone()))
                } else {
                    None
                }
            })
            .collect();
        let window_handle = window.window_handle();

        cx.spawn(async move |_this, async_cx| {
            let executor = async_cx.background_executor().clone();
            let mut errors: Vec<(PathBuf, String)> = Vec::new();
            for (repo, wt_root) in removals {
                let repo_clone = repo.clone();
                let wt_clone = wt_root.clone();
                let result = executor
                    .spawn(async move {
                        crate::lane::git::remove_lane(&repo_clone, &wt_clone, false)
                    })
                    .await;
                if let Err(e) = result {
                    errors.push((wt_root, e.to_string()));
                }
            }
            // Default-kind project directory or any leftover (e.g. the
            // primary lane under a subdirectory anchor) gets a
            // best-effort recursive delete. Already-gone is fine.
            let path_for_rm = project_path.clone();
            let rm_err: std::io::Result<()> = executor
                .spawn(async move {
                    if path_for_rm.is_dir() {
                        std::fs::remove_dir_all(&path_for_rm)
                    } else {
                        Ok(())
                    }
                })
                .await;
            if let Err(e) = rm_err {
                errors.push((project_path.clone(), e.to_string()));
            }

            // SILENT-OK: workspace may drop during background disk cleanup
            let _ = async_cx.update(|app_cx| {
                // SILENT-OK: workspace may drop during background disk cleanup
                let _ = app_cx.update_window(window_handle, |_, window, cx_w| {
                    let Some(ws) = _this.upgrade() else {
                        return;
                    };
                    let close_window = ws.update(cx_w, |ws, cx| {
                        for (path, message) in &errors {
                            let report = daruda_store::observability::error_report::ErrorReport::new(
                                "Lane disk cleanup failed",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context(
                                "path",
                                daruda_store::observability::system_info::redact_home(path),
                            )
                            .with_context("error", message.clone())
                            .dedup("project.delete.disk")
                            .build();
                            ws.report_error(report, cx);
                        }
                        // Active may have shifted while disk work ran;
                        // if our project is no longer active, drop its
                        // registered state directly without going
                        // through the activate path.
                        if ws.active.project != project_id {
                            ws.projects.retain(|p| p.id != project_id);
                            ws.main_area
                                .inactive_worktree_runtimes
                                .retain(|key, _| key.project != project_id);
                            ws.mutate_durable(cx, |_, _| {});
                            return ws.projects.is_empty();
                        }
                        let keep = ws.close_active_project(window, cx);
                        !keep
                    });
                    if close_window {
                        window.remove_window();
                        crate::windows::ensure_welcome_if_last(cx_w);
                    }
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    // Behaviour for `add_project` / `toggle_project_collapse` /
    // `rename_active_project` / `close_active_project` /
    // `open_project_in_new_window` is exercised end-to-end through the
    // workspace fixtures in `workspace::tests::projects` and
    // `workspace::tests::dnd` (group / move-to-group / activation), which
    // share the same `gpui::TestAppContext` plumbing required to drive
    // these `&mut Window` / `&mut Context<Self>` methods. A local stub
    // here would either duplicate that scaffolding or skip the actual
    // GPUI surface — the parent suites are the better seam.
}
