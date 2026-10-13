//! Project removal: closing the active project, deleting it on disk, and
//! the delete-project chooser that routes to either.

use std::path::PathBuf;

use daruda_store::project::ProjectId;
use gpui::{AppContext as _, Context, Window};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::images::release_pane_images;

impl Workspace {
    /// Remove the active project and route the workspace to the next one.
    ///
    /// The window always survives. When the removal leaves nothing to show
    /// — no projects left, or every surviving project is lane-less — the
    /// workspace resets to the empty state and `render` paints the Landing
    /// view; callers have nothing to handle.
    ///
    /// Every lane's runtime for the removed project also drops out of
    /// `runtimes` so memory does not leak.
    pub(crate) fn close_active_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project_id) = self.active_project().map(|p| p.id) else {
            return;
        };
        // Release every pane the closing project owns — across all of its
        // lanes' runtimes (including the active lane, which belongs to
        // this project) — before they drop below. Skipping this leaves
        // the tracker polling dead shell PIDs for the window's lifetime
        // and stale claude bindings behind. The runtimes themselves are
        // dropped *after* `activate_lane` re-points `self.active` away
        // from this project (a single `retain` at the tail), so the
        // "active runtime always present" invariant is never broken
        // mid-teardown.
        let owned_pane_ids: Vec<_> = self
            .main_area
            .runtimes
            .iter()
            .filter(|(key, _)| key.project == project_id)
            .flat_map(|(_, runtime)| runtime.panes.iter().map(|p| p.id))
            .filter(|id| !self.is_orchestrator_pane(*id))
            .collect();
        self.forget_panes(&owned_pane_ids, cx);
        // Same reason, for the GPU images a Markdown preview holds: the
        // sprite atlas never evicts on its own, so a pane that drops without
        // this keeps its textures for the process's lifetime.
        for (_, runtime) in self
            .main_area
            .runtimes
            .iter_mut()
            .filter(|(key, _)| key.project == project_id)
        {
            release_pane_images(&mut runtime.panes, &owned_pane_ids, window, cx);
        }
        // Drop per-lane caches for the closing project so they do
        // not leak across project deletes.
        self.lane_scoped.retain(|key, _| key.project != project_id);

        self.projects.retain(|p| p.id != project_id);

        // No more projects — clear the active runtime fields and leave the
        // window standing on Landing. Persist the empty list (mirrors the
        // no-survivor-lane branch below) so a force-quit between this point
        // and the natural shutdown save can't resurrect the closed project
        // from a stale on-disk snapshot.
        if self.projects.is_empty() {
            self.reset_to_empty_workspace(window, cx);
            return;
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
            // the window lands on Landing rather than a blank viewport.
            self.reset_to_empty_workspace(window, cx);
            return;
        };
        // `self.active` is intentionally left pointing at the deleted
        // project's lane ref through this call — its project_id is
        // guaranteed distinct from `next_target.project` (we just removed
        // it from `self.projects`), so `activate_lane`'s same-target guard
        // doesn't fire. Resetting to `LaneRef::default()` here would
        // collide with the natural (project=0, lane=0) target of the
        // surviving first project and false-trigger the guard, leaving
        // main_area empty (regression covered by
        // `close_active_project_keeps_window_when_other_remain`). The
        // deleted project's runtime stays in `runtimes` until the retain
        // below so `active_runtime()` keeps resolving during the switch.
        self.activate_lane(next_target, window, cx);
        // Now that `self.active` points at the surviving lane, forget every
        // runtime that belonged to the removed project (the formerly-active
        // one included) — their `LaneRef`s are dangling once the project
        // is gone. The active runtime survives because `next_target` is in
        // a different project.
        self.main_area
            .runtimes
            .retain(|key, _| key.project != project_id);
        // After the switch, so a detail falling back lands on the survivor.
        self.page_details_follow_projects(window, cx);
        self.mutate_durable(cx, |_, _| {});
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
        let removals: Vec<(PathBuf, PathBuf, Option<PathBuf>)> = project
            .lanes
            .iter()
            .filter_map(|wt| {
                if let daruda_store::project::LaneKind::Git {
                    repo_root,
                    worktree_root,
                    ..
                } = &wt.kind
                {
                    Some((
                        repo_root.clone(),
                        worktree_root.clone(),
                        crate::workspace::flow_paths::lane_lock_dir(&self.lock_root, worktree_root),
                    ))
                } else {
                    None
                }
            })
            .collect();
        let window_handle = window.window_handle();

        cx.spawn(async move |_this, async_cx| {
            let executor = async_cx.background_executor().clone();
            let mut errors: Vec<(PathBuf, String)> = Vec::new();
            for (repo, wt_root, lock_dir) in removals {
                let repo_clone = repo.clone();
                let wt_clone = wt_root.clone();
                let result = executor
                    .spawn(async move {
                        crate::lane::git::remove_lane(&repo_clone, &wt_clone, false).inspect(
                            |()| crate::workspace::flow_paths::forget_lane_lock(lock_dir),
                        )
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
            async_cx.update(|app_cx| {
                // SILENT-OK: workspace may drop during background disk cleanup
                let _ = app_cx.update_window(window_handle, |_, window, cx_w| {
                    let Some(ws) = _this.upgrade() else {
                        return;
                    };
                    ws.update(cx_w, |ws, cx| {
                        for (path, message) in &errors {
                            let report = daruda_store::observability::error_report::ErrorReport::new(
                                crate::surface::strings::error::lane_disk_cleanup_failed(),
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
                            ws.page_details_follow_projects(window, cx);
                            // Active is in a different project, so dropping
                            // every runtime keyed to `project_id` leaves the
                            // active runtime intact.
                            ws.main_area
                                .runtimes
                                .retain(|key, _| key.project != project_id);
                            // Reaching zero here needs `self.active` to have
                            // been dangling already, but if it happens the
                            // workspace must land in the same normalized
                            // empty state every other path establishes —
                            // `render` reads `active_runtime()` to paint
                            // Landing.
                            if ws.projects.is_empty() {
                                ws.reset_to_empty_workspace(window, cx);
                            } else {
                                ws.mutate_durable(cx, |_, _| {});
                            }
                            return;
                        }
                        ws.close_active_project(window, cx);
                    });
                });
            });
        })
        .detach();
    }

    /// Open the delete-project chooser for the active project, wiring
    /// its submit branch to `close_active_project` (keep on disk) or
    /// `delete_active_project_on_disk` (remove from disk). Single entry
    /// point shared by the left-dock project context menu and the
    /// main-area inaccessible empty-state, so both reuse the same
    /// deferred-close dance (the dialog tears the modal entity down on
    /// submit, so the workspace mutation must run after the current
    /// event cycle drains — see `app_cx.defer` below).
    pub(in crate::workspace) fn open_delete_active_project_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.active_project().map(|p| p.id) else {
            return;
        };
        self.open_delete_project_modal(project_id, window, cx);
    }

    /// Open the delete-project chooser for `project_id` without forcing
    /// the workspace focus onto it. Used by the left-dock context menu's
    /// Remove action on a non-removable (main / default) lane whose root
    /// is inaccessible: the lane stands in for the whole project, but
    /// snapping focus to a dead lane just to target the modal would
    /// strand the user there if they cancel.
    ///
    /// The deletion machinery (`close_active_project` /
    /// `delete_active_project_on_disk`) targets `self.active`, so the
    /// submit branches activate the target project's snap lane *first* —
    /// but only once the user has confirmed. Cancel / Esc dismisses
    /// without touching focus. Shares the same deferred-close dance as
    /// [`Self::open_delete_active_project_modal`].
    pub(in crate::workspace) fn open_delete_project_modal(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_name) = self.project_for(project_id).map(|p| p.name.clone()) else {
            return;
        };
        let window_handle = window.window_handle();
        let ws_for_submit = cx.weak_entity();
        crate::workspace::delete_project_modal::open_delete_project_modal(
            project_name,
            move |choice, _window, app_cx| {
                use crate::workspace::delete_project_modal::DeleteProjectChoice;
                let ws_weak = ws_for_submit.clone();
                app_cx.defer(move |app_cx| {
                    let Some(ws) = ws_weak.upgrade() else {
                        return;
                    };
                    crate::windows::try_update_workspace_window(
                        window_handle,
                        app_cx,
                        "project.delete_by_id",
                        move |window, cx_w| match choice {
                            DeleteProjectChoice::KeepOnDisk => {
                                ws.update(cx_w, |ws, cx| {
                                    // Bail if the target vanished between
                                    // open and confirm — never delete a
                                    // different, still-active project.
                                    if !ws.activate_target_project(project_id, window, cx) {
                                        return;
                                    }
                                    // Closing the last one empties the
                                    // workspace; the window stays on Landing.
                                    ws.close_active_project(window, cx);
                                });
                            }
                            DeleteProjectChoice::DeleteOnDisk => {
                                ws.update(cx_w, |ws, cx| {
                                    if !ws.activate_target_project(project_id, window, cx) {
                                        return;
                                    }
                                    ws.delete_active_project_on_disk(window, cx);
                                });
                            }
                        },
                    );
                });
            },
            window,
            cx,
        );
    }

    /// Snap the active focus onto `project_id`'s snap-target lane so the
    /// active-project-keyed delete path operates on it. Returns `true`
    /// when `self.active` now points at `project_id` (already-active or
    /// just-snapped), `false` when the project vanished from the workspace
    /// between modal-open and confirm — in which case the caller must NOT
    /// run the delete, or it would operate on the wrong (still-active)
    /// project.
    fn activate_target_project(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active.project == project_id {
            return true;
        }
        let Some(target) = self.project_for(project_id).and_then(|p| p.snap_target()) else {
            return false;
        };
        self.activate_lane(target, window, cx);
        true
    }
}
