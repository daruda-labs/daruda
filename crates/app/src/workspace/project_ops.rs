//! Project-level mutation ops on [`Workspace`] — add / close / move.
//!
//! Lane-scoped operations live in `lane_ops.rs`; this module
//! owns the project boundary itself (registering a new project root
//! with the workspace, removing one without tearing down the window).

use std::path::{Path, PathBuf};

use daruda_store::project::{LaneKind, LaneRef, ProjectId, ProjectUuid, WindowOpenPolicy};
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
    /// Clear all main-area runtime state and reset the active ref.
    /// Called from `close_active_project` when no project (or no usable
    /// lane) remains.
    fn reset_to_empty_workspace(&mut self, cx: &mut Context<Self>) {
        self.main_area.tabs.clear();
        self.main_area.panes.clear();
        self.main_area.active_tab_index = 0;
        self.main_area.tab_history.clear();
        self.main_area.focused_pane_id = 0;
        self.active = LaneRef::default();
        self.mutate_durable(cx, |_, _| {});
    }

    /// Add a freshly-opened project to this workspace and activate its
    /// first lane.
    ///
    /// Mints a new [`ProjectId`] from the monotonic `next_project_id`
    /// counter, walks the filesystem at `root` to discover git
    /// lanes (or falls back to one default), and pushes the result
    /// onto `self.projects`. Then routes through `activate_lane`
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
        let target = project.first_lane_ref();
        self.projects.push(project);
        // Activate the new lane. When `self.projects` was empty
        // before this call there is no prior runtime to freeze, but
        // `activate_lane` is still the right path: it lazy-seeds
        // a pane at the new lane's path so the user lands on a
        // live shell immediately.
        if let Some(t) = target {
            // First project case: `self.active` is the default
            // (project=0, lane=0). `activate_lane` skips when
            // `self.active == target`, but with monotonic ids the new
            // project's id is always > 0 the first time so this fires.
            // Manually set `self.active` to a sentinel that differs
            // from `target` so the swap path runs even when the
            // workspace previously had no live runtime to freeze.
            if self.projects.len() == 1 {
                self.active = LaneRef::default();
            }
            self.activate_lane(t, window, cx);
        }
        // Empty closure: see group_ops.rs:83 for rationale. `activate_lane`
        // consumes `&mut Window`, so the persist trigger has to land after
        // those borrows release.
        self.mutate_durable(cx, |_, _| {});
        // Detect the new project's default branch in the background.
        // `Project::new_with_uuid` sets `default_branch: None` to avoid
        // blocking the UI thread; this spawn fills it in and persists.
        if let Some(root) = self
            .projects
            .iter()
            .find(|p| p.id == new_id && p.lanes.iter().any(|l| l.is_git()))
            .map(|p| p.root.clone())
        {
            crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
                cx,
                move || crate::lane::git::default_branch(&root),
                move |ws, detected, cx| {
                    if let Some(branch) = detected
                        && let Some(p) = ws.projects.iter_mut().find(|p| p.id == new_id)
                    {
                        p.default_branch = Some(branch);
                        ws.mutate_durable(cx, |_, _| {});
                    }
                },
            )
            .detach();
        }
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
    /// `inactive_lane_runtimes` so memory does not leak.
    pub(crate) fn close_active_project(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(project_id) = self.active_project().map(|p| p.id) else {
            return false;
        };
        // Release every pane the closing project owns — the live ones
        // (the active lane belongs to this project) and those in its
        // frozen runtimes — before they drop below. Skipping this
        // leaves the tracker polling dead shell PIDs for the window's
        // lifetime and stale claude bindings behind.
        let owned_pane_ids: Vec<_> = self
            .main_area
            .panes
            .iter()
            .map(|p| p.id)
            .chain(
                self.main_area
                    .inactive_lane_runtimes
                    .iter()
                    .filter(|(key, _)| key.project == project_id)
                    .flat_map(|(_, runtime)| runtime.panes.iter().map(|p| p.id)),
            )
            .collect();
        self.release_pane_tracking(&owned_pane_ids, cx);
        // Forget every inactive runtime that belonged to the removed
        // project — the WorktreeRefs become dangling once the project
        // is gone.
        self.main_area
            .inactive_lane_runtimes
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
        // the caller to close the window. Persist the empty list (mirrors
        // the no-survivor-lane branch below) so a force-quit between this
        // point and the natural shutdown save can't resurrect the closed
        // project from a stale on-disk snapshot.
        if self.projects.is_empty() {
            self.reset_to_empty_workspace(cx);
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
            self.reset_to_empty_workspace(cx);
            return false;
        };
        // Reset live runtime; the removed project's panes are gone for
        // good and their TabEntry ids hold no PaneIds we can reuse.
        // `self.active` is intentionally left pointing at the deleted
        // project's lane ref — its project_id is guaranteed
        // distinct from `next_target.project` (we just removed it from
        // `self.projects`), so `activate_lane`'s same-target guard
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
        self.activate_lane(next_target, window, cx);
        // `activate_lane`'s freeze step wrote a dangling empty
        // runtime under the deleted project's lane ref. Drop it so
        // `inactive_lane_runtimes` stays clean.
        self.main_area
            .inactive_lane_runtimes
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
            async_cx.update(|app_cx| {
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
                                .inactive_lane_runtimes
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

    /// Collect `(id, root)` for every project that owns at least one
    /// git lane. Non-git projects are skipped so the reconcile pass
    /// never spawns background git work for a directory git can't
    /// answer for.
    fn git_project_roots(&self) -> Vec<(ProjectId, PathBuf)> {
        git_project_roots(&self.projects)
    }

    /// Re-detect each git project's `default_branch` from git on
    /// restore and update the runtime project when it drifted. This
    /// backfills legacy state files (where `default_branch` is `None`)
    /// and absorbs external changes (e.g. the repo's `origin/HEAD`
    /// moved while daruda was closed).
    ///
    /// Scope is deliberately narrow — only `default_branch` is
    /// refreshed. No lanes are added or removed; main-lane recovery
    /// belongs to the repo base node, not here.
    pub(in crate::workspace) fn reconcile_project_default_branches(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        for (project_id, root) in self.git_project_roots() {
            crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
                cx,
                move || crate::lane::git::default_branch(&root),
                move |ws, detected, cx| {
                    let Some(p) = ws.projects.iter_mut().find(|p| p.id == project_id) else {
                        return;
                    };
                    let Some(branch) =
                        reconciled_default_branch(p.default_branch.as_deref(), detected)
                    else {
                        return;
                    };
                    p.default_branch = Some(branch);
                    // Persist the refreshed value and re-stage the
                    // left-dock snapshot (rule 10: targeted notify).
                    ws.mutate_durable(cx, |_, _| {});
                },
            )
            .detach();
        }
    }

    /// Upgrade construction-placeholder lane lists to git-discovered
    /// ones off the UI thread. Window creation seeds each fresh
    /// project with a single `Default` placeholder lane
    /// ([`crate::project::Project::bootstrap_placeholder`]) so it never
    /// blocks on git CLI; this pass re-runs the discovery on the
    /// background executor and swaps the result in. It also heals a
    /// state file persisted during the placeholder window (e.g. the
    /// app died before discovery returned): restore re-enters here and
    /// the same probe upgrades the persisted placeholder.
    ///
    /// Genuinely non-git projects share the single-`Default` shape;
    /// for them discovery returns the same shape and the upgrade is a
    /// no-op.
    pub(in crate::workspace) fn reconcile_bootstrapped_lanes(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<(ProjectId, PathBuf)> = self
            .projects
            .iter()
            .filter(|p| has_placeholder_lanes(p))
            .map(|p| (p.id, p.root.clone()))
            .collect();
        for (project_id, root) in targets {
            crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
                cx,
                move || crate::lane::Lane::bootstrap_from_project(&root),
                move |ws, lanes, cx| ws.apply_discovered_lanes(project_id, lanes, cx),
            )
            .detach();
        }
    }

    /// Foreground half of [`Self::reconcile_bootstrapped_lanes`] —
    /// swap `lanes` into the project and repair everything addressed
    /// by the placeholder ref. Split out (and workspace-visible) so
    /// tests can drive the swap synchronously.
    pub(in crate::workspace) fn apply_discovered_lanes(
        &mut self,
        project_id: ProjectId,
        lanes: Vec<crate::lane::Lane>,
        cx: &mut Context<Self>,
    ) {
        // Discovery returned the same single-Default shape — the root
        // genuinely isn't a git repo and the placeholder is already
        // correct.
        if lanes.len() == 1 && !lanes[0].kind.is_git() {
            return;
        }
        let Some(p) = self.projects.iter_mut().find(|p| p.id == project_id) else {
            return; // project closed while discovery ran
        };
        // Swap only while the placeholder is still in place — any real
        // lane list that appeared meanwhile wins.
        if !has_placeholder_lanes(p) {
            return;
        }
        let old_ref = LaneRef {
            project: project_id,
            lane: p.lanes[0].id,
        };
        let new_active_id = lanes.first().map(|l| l.id).unwrap_or(0);
        p.lanes = lanes;
        p.last_active_lane_id = new_active_id;
        let new_ref = LaneRef {
            project: project_id,
            lane: new_active_id,
        };
        if old_ref != new_ref {
            // Discovery assigns ids before sorting the project-root
            // lane first, so the active lane's id may differ from the
            // placeholder's `0` — re-key everything addressed by the
            // placeholder ref.
            if self.active == old_ref {
                self.active = new_ref;
            } else if let Some(rt) = self.main_area.inactive_lane_runtimes.remove(&old_ref) {
                self.main_area.inactive_lane_runtimes.insert(new_ref, rt);
            }
            self.invalidate_visible_files_cache(old_ref);
            self.invalidate_visible_files_cache(new_ref);
        }
        // The placeholder carried no git lane, so the startup
        // default-branch pass skipped this project — run it now that
        // git lanes exist.
        self.reconcile_project_default_branches(cx);
        self.mutate_durable(cx, |_, _| {});
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
                                let keep = ws.update(cx_w, |ws, cx| {
                                    // Bail if the target vanished between
                                    // open and confirm — never delete a
                                    // different, still-active project.
                                    if !ws.activate_target_project(project_id, window, cx) {
                                        return true;
                                    }
                                    ws.close_active_project(window, cx)
                                });
                                if !keep {
                                    window.remove_window();
                                    crate::windows::ensure_welcome_if_last(cx_w);
                                }
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

/// True when `p` still carries only the construction placeholder — a
/// single `Default` lane ([`crate::project::Project::bootstrap_placeholder`]'s
/// shape). Genuinely non-git projects share this shape; callers treat
/// them identically (probe → same shape back → no-op).
fn has_placeholder_lanes(p: &crate::project::Project) -> bool {
    p.lanes.len() == 1 && matches!(p.lanes[0].kind, LaneKind::Default)
}

/// `(id, root)` for every project that owns at least one git lane.
/// Free function so the git/non-git filtering is unit-testable
/// without a full `Workspace`.
fn git_project_roots(projects: &[crate::project::Project]) -> Vec<(ProjectId, PathBuf)> {
    projects
        .iter()
        .filter(|p| p.lanes.iter().any(|l| l.is_git()))
        .map(|p| (p.id, p.root.clone()))
        .collect()
}

/// Decide the value to store for a project's `default_branch` given
/// the currently-held value and what git just detected. Returns the
/// new value to store, or `None` when no update is needed (so the
/// caller can skip the mutation and the notify).
///
/// - `detected == None` → keep current, no change.
/// - detected matches current → no change.
/// - otherwise (backfill from `None`, or drift) → store the detected
///   value.
fn reconciled_default_branch(current: Option<&str>, detected: Option<String>) -> Option<String> {
    match detected {
        Some(branch) if current != Some(branch.as_str()) => Some(branch),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use daruda_store::project::{ProjectId, ProjectUuid};

    use super::{git_project_roots, reconciled_default_branch};
    use crate::lane::Lane;
    use crate::project::Project;

    /// Build a runtime project with a single lane of the requested
    /// git-ness, without touching the filesystem.
    fn project_with_lane(id: ProjectId, root: &str, git: bool) -> Project {
        let path = PathBuf::from(root);
        let lane = if git {
            Lane::git(
                0,
                path.clone(),
                Some("main".to_string()),
                path.clone(),
                path.clone(),
                0,
            )
        } else {
            Lane::default_for_project(0, path.clone())
        };
        Project {
            id,
            uuid: ProjectUuid::new(),
            root: path,
            name: String::new(),
            default_branch: None,
            base_branch: None,
            lanes: vec![lane],
            last_active_lane_id: 0,
            group_id: None,
            color: None,
            tab_order: 0,
            is_collapsed: false,
            availability: crate::lane::availability::LaneAvailability::Present,
        }
    }

    #[test]
    fn git_project_roots_keeps_only_git_projects() {
        let projects = vec![
            project_with_lane(0, "/tmp/daruda_git_a", true),
            project_with_lane(1, "/tmp/daruda_nongit_b", false),
            project_with_lane(2, "/tmp/daruda_git_c", true),
        ];
        let roots = git_project_roots(&projects);
        assert_eq!(
            roots,
            vec![
                (0, PathBuf::from("/tmp/daruda_git_a")),
                (2, PathBuf::from("/tmp/daruda_git_c")),
            ],
            "only projects with a git lane are returned, paired with their id"
        );
    }

    #[test]
    fn reconcile_backfills_none() {
        assert_eq!(
            reconciled_default_branch(None, Some("main".to_string())),
            Some("main".to_string())
        );
    }

    #[test]
    fn reconcile_updates_on_drift() {
        assert_eq!(
            reconciled_default_branch(Some("main"), Some("develop".to_string())),
            Some("develop".to_string())
        );
    }

    #[test]
    fn reconcile_no_change_when_equal() {
        assert_eq!(
            reconciled_default_branch(Some("main"), Some("main".to_string())),
            None
        );
    }

    #[test]
    fn reconcile_keeps_current_when_detection_fails() {
        assert_eq!(reconciled_default_branch(Some("main"), None), None);
        assert_eq!(reconciled_default_branch(None, None), None);
    }

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
