//! Lane create / remove: validation, the post-git finalize steps, lane-id
//! allocation, and the remove-lane modal openers.

use daruda_store::project::{LaneId, LaneRef, ProjectId};
use daruda_store::tasks::TaskAgentSurface;
use gpui::{Context, Window};

use super::{CreateWorktreePlan, RemoveWorktreePlan, git_repo_root_of};
use crate::lane::availability::LaneAvailability;
use crate::workspace::LaneRuntime;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::images::release_pane_images;
use crate::workspace::main_area::pane;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

impl Workspace {
    /// True when the target lane can be removed via `git worktree
    /// remove`. Main lanes (checkout at `repo_root`) and the
    /// default non-git stand-in are non-removable.
    pub(in crate::workspace) fn lane_removable(wt: &crate::lane::Lane) -> bool {
        match &wt.kind {
            daruda_store::project::LaneKind::Git { .. } => !wt.is_main,
            daruda_store::project::LaneKind::Default => false,
        }
    }

    /// Validate that `target` is removable and resolve its (repo_root,
    /// path) for the shell-out. Pure — no state mutation.
    pub(in crate::workspace) fn validate_remove_lane(
        &self,
        target: LaneRef,
    ) -> Result<RemoveWorktreePlan, String> {
        let wt = self
            .lane_for(target)
            .ok_or_else(crate::surface::strings::modal::remove_lane_err_not_found)?;
        if !Self::lane_removable(wt) {
            return Err(crate::surface::strings::modal::remove_lane_err_cannot_remove());
        }
        // A run's cancel token and its thread are only handed back when the
        // run ends on its own, so removing the lane would leave it working a
        // checkout `git worktree remove` just deleted with no way left to stop
        // it. Refusing matches what the shell-out does anyway: `git worktree
        // remove` fails on a dirty checkout, and a running flow makes one.
        if self.flows.runs.is_running(target) {
            return Err(crate::surface::strings::modal::remove_lane_err_flow_running());
        }
        // `runs` only knows this process. Another daruda — a debug build
        // beside a release one, which is why the lock root is deliberately
        // profile-independent — leaves no trace here but the lock. Git's own
        // dirty check is not the backstop: a run's artifacts sit under the
        // `.gitignore` the engine writes, so `worktree remove` cannot see
        // them, and the failure a modified file does cause is the one the
        // remove modal answers by offering `--force`.
        if let Some(holder) = self.lane_holder(&wt.path) {
            return Err(crate::surface::strings::modal::remove_lane_err_flow_elsewhere(holder.pid));
        }
        let repo_root = match &wt.kind {
            daruda_store::project::LaneKind::Git { repo_root, .. } => repo_root.clone(),
            _ => return Err(crate::surface::strings::modal::remove_lane_err_not_git()),
        };
        Ok(RemoveWorktreePlan {
            path: wt.path.clone(),
            repo_root,
            lock_dir: crate::workspace::flow_paths::lane_lock_dir(&self.lock_root, &wt.path),
        })
    }

    /// Post-git cleanup on the UI thread: switch away if active, then
    /// drop the entry and its runtime. Invariant: the active lane is
    /// always survivable because `validate_remove_lane` refuses
    /// main/default kinds, so at least one other entry remains.
    pub(in crate::workspace) fn finalize_remove_lane(
        &mut self,
        target: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Capture the path before removal so the per-lane `GlobalSkills` /
        // `McpState` Global entries can be pruned (else they grow
        // unbounded across the session).
        let removed_path = self.lane_for(target).map(|w| w.path.clone());

        // Fallback `LaneRef` to activate when removal strips the active
        // lane: prefer a sibling in the same project, else another
        // project's snap target so nothing wedges on a dangling ref.
        // `None` only in the degenerate about-to-be-empty case (the `[+]`
        // is hidden for non-git workspaces, so main lanes keep ≥1 each).
        let fallback_target: Option<LaneRef> = if self.active == target {
            let same_project = self.projects.get(target.project).and_then(|p| {
                p.lanes
                    .iter()
                    .find(|w| w.id != target.lane)
                    .map(|w| LaneRef {
                        project: target.project,
                        lane: w.id,
                    })
            });
            let cross_project = || {
                self.projects
                    .iter()
                    .find(|p| p.id != target.project)
                    .and_then(|p| p.snap_target())
            };
            same_project.or_else(cross_project)
        } else {
            None
        };
        if let Some(fallback) = fallback_target {
            self.activate_lane(fallback, window, cx);
        }
        // Release the removed lane's panes from PTY tracking before their
        // runtime drops — else the tracker keeps walking dead shell PIDs.
        // Gated on `target != self.active`: if no fallback was found
        // (degenerate about-to-be-empty case), dropping the active
        // runtime would break the "active runtime always present"
        // invariant. The window is closing then, so leave it.
        let removed_pane_ids: Vec<_> = self
            .main_area
            .runtimes
            .get(&target)
            .map(|runtime| runtime.panes.iter().map(|p| p.id).collect::<Vec<_>>())
            .unwrap_or_default();
        // Never the orchestrator's: its pane can be in the runtime being torn
        // down, but it belongs to a session that outlives any worktree —
        // `hide_orchestrator_tab` is what stashes its draft.
        let forgotten: Vec<_> = removed_pane_ids
            .iter()
            .copied()
            .filter(|id| !self.is_orchestrator_pane(*id))
            .collect();
        if target != self.active {
            self.forget_panes(&forgotten, cx);
            if let Some(runtime) = self.main_area.runtimes.get_mut(&target) {
                release_pane_images(&mut runtime.panes, &removed_pane_ids, window, cx);
            }
            self.main_area.runtimes.remove(&target);
        }
        // Clear per-lane state too — else the notify watcher keeps
        // running, the cache holds stale paths, and the gitignore matcher
        // leaks. Dropping the entries also drops the embedded
        // `RecommendedWatcher`, stopping the kernel-side watch.
        self.lane_scoped.remove(&target);
        if let Some(project) = self.projects.get_mut(target.project) {
            project.lanes.retain(|w| w.id != target.lane);
        }
        self.page_details_follow_projects(window, cx);
        if let Some(path) = removed_path {
            use gpui::BorrowAppContext as _;
            cx.update_global::<crate::agent::skills::global::GlobalSkills, _>(|s, _| {
                s.forget_lane(&path);
            });
            cx.update_global::<crate::agent::mcp::McpState, _>(|s, _| {
                s.forget_lane(&path);
            });
            // Drop the per-lane slice from `SettingsStore` so its
            // BTreeMap doesn't grow unbounded (CLAUDE.md §"GPUI
            // shared-state convention — Cleanup rule").
            cx.update_global::<crate::settings_store::SettingsStore, _>(|s, _| {
                s.forget_lane(&path);
            });
        }
        // Watcher pair list is fixed at spawn, so removing a lane leaves
        // a dead JSONL subscription — re-spawn to match the live lane set.
        self.refresh_jsonl_watcher(cx);
        // Skill scope follows the active lane's root; restart the watcher.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.respawn_flow_watcher(cx);
        // Empty closure: see group_ops.rs:83. Only schedules persist.
        self.mutate_durable(cx, |_, _| {});
        // Force a render even when no fallback fired — the lane list
        // shrank and the dock would otherwise keep the removed row.
        cx.notify();
    }

    /// Resolve the active git repo_root from the active project's lane
    /// list. `None` when the workspace isn't git-backed. Lets callers
    /// (e.g. the `[+]` button) build a `CreateWorktreeModal` without
    /// traversing the lane list.
    pub(in crate::workspace) fn git_repo_root(&self) -> Option<std::path::PathBuf> {
        git_repo_root_of(self.active_lanes())
    }

    /// [`Self::git_repo_root`] for a named project rather than the active one.
    pub(in crate::workspace) fn git_repo_root_for_project(
        &self,
        project: ProjectId,
    ) -> Option<std::path::PathBuf> {
        git_repo_root_of(&self.project_for(project)?.lanes)
    }

    /// Post-git UI-thread work: spawn a pane at the new checkout, wrap it
    /// in a Tab / LaneRuntime, register the Lane, and activate it. Errors
    /// leave the created worktree orphaned on disk (recover via `git
    /// worktree prune`) and bubble the message back to the modal.
    /// `requested_agent` names the agent the initial chat pane opens under.
    /// `None` keeps the session-sticky default; an id the catalog does not
    /// hold falls back to the catalog default, not to the sticky one — the
    /// rule [`Workspace::open_agent_id`] already applies, not a second one.
    /// Only the control surface passes a value — the create form and the task
    /// workflow both leave the choice to the session.
    pub(in crate::workspace) fn finalize_create_lane(
        &mut self,
        plan: CreateWorktreePlan,
        project_id: ProjectId,
        surface: TaskAgentSurface,
        requested_agent: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, String> {
        if self.project_for(project_id).is_none() {
            return Err(crate::surface::strings::create_lane::err_no_active_project());
        }
        let CreateWorktreePlan {
            branch,
            new_path,
            repo_root,
            base_ref,
            description,
            session_host,
        } = plan;
        // The initial pane mirrors the requested surface: a terminal
        // (default) or an in-app Agent chat rooted at the new checkout.
        // The agent-chat pane parks Idle and connects lazily on first
        // focus — the `activate_lane` below focuses it, starting the ACP
        // session.
        let pane = match surface {
            TaskAgentSurface::Terminal => {
                let account = self.default_account_selection_for_new_pane(None);
                let prepared = self.resolve_account(account, pane::AccountDomain::Any);
                self.create_pane_with_cwd(
                    Some(new_path.clone()),
                    account,
                    prepared.as_ref(),
                    window,
                    cx,
                )
                .map_err(|e| e.to_string())?
            }
            TaskAgentSurface::AgentChat => {
                // No source pane (new lane), so open under the
                // session-sticky default. A brand-new lane has neither a
                // legacy `remote_cwd` nor a `session_host` yet, so an agent
                // whose command needs the legacy `{{cwd}}` token parks in
                // the "no remote path set" error rather than connecting;
                // every other launch shape resolves to `Local`.
                let agent_id = self.open_agent_id(requested_agent);
                self.create_new_agent_chat_pane(
                    agent_id,
                    Some(new_path.clone()),
                    None,
                    None,
                    window,
                    cx,
                )
            }
        };
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        let tab = pane::TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        };

        let new_id = self.allocate_lane_id(project_id);
        let new_ref = LaneRef {
            project: project_id,
            lane: new_id,
        };
        let order = self
            .project_for(project_id)
            .map(|p| p.lanes.len())
            .unwrap_or(0) as u32;
        // A freshly-added linked lane's toplevel is exactly `new_path`
        // (where `git worktree add` writes the `.git` pointer). No
        // anchoring yet, so `path` and `worktree_root` start equal.
        let worktree_root = new_path.clone();
        let mut wt = crate::lane::Lane::git(
            new_id,
            new_path,
            Some(branch),
            repo_root,
            worktree_root,
            order,
        );
        wt.base_ref = base_ref;
        wt.description = description;
        if let Some(host) = session_host {
            wt.set_session_host(Some(host));
        }
        if let Some(project) = self.project_for_mut(project_id) {
            project.lanes.push(wt);
        }

        let runtime = LaneRuntime {
            preview_tab_id: None,
            tabs: vec![tab],
            panes: vec![pane],
            active_tab_index: 0,
            tab_history: Vec::new(),
            focused_pane_id: pane_id,
        };
        self.main_area.runtimes.insert(new_ref, runtime);
        self.activate_lane(new_ref, window, cx);
        self.page_details_follow_projects(window, cx);
        // New cwd → new JSONL dir to watch.
        self.refresh_jsonl_watcher(cx);
        // New lane root → new project-skills directory to watch.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.respawn_flow_watcher(cx);
        // Return the spawned pane's id so task-driven callers (start_task)
        // write into that PTY directly rather than relying on
        // `focused_pane_id`, which races with activate_lane's focus state.
        Ok(pane_id)
    }

    /// Monotonic lane id allocator scoped to `project_id`. Walks both the
    /// lane list and every runtime key so a phantom key from a crash
    /// mid-remove never collides with a fresh id.
    fn allocate_lane_id(&self, project_id: ProjectId) -> LaneId {
        let max_list = self
            .project_for(project_id)
            .iter()
            .flat_map(|p| p.lanes.iter())
            .map(|w| w.id)
            .max();
        let max_map = self
            .main_area
            .runtimes
            .keys()
            .filter(|r| r.project == project_id)
            .map(|r| r.lane)
            .max();
        max_list
            .into_iter()
            .chain(max_map)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0)
    }

    /// Open the Remove-lane confirmation modal for `target`. Single
    /// entry point shared by the left-dock row `×` button and the
    /// main-area inaccessible empty-state, so both spawn the identical
    /// validated modal instead of copying the build sequence. No-op
    /// when the lane is gone or non-removable (main / default kinds —
    /// those route through the project-delete flow instead).
    pub(in crate::workspace) fn open_remove_lane_modal(
        &mut self,
        target: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        if !Self::lane_removable(wt) {
            return;
        }
        let label = gpui::SharedString::from(wt.display_name());
        let path = gpui::SharedString::from(wt.path.to_string_lossy().into_owned());
        let plan = match self.validate_remove_lane(target) {
            Ok(p) => p,
            Err(msg) => {
                let report = daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::error::lane_remove_failed(),
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .message(msg)
                .at(file!(), line!())
                .dedup("lane.open_remove_modal.validate")
                .build();
                self.report_error(report, cx);
                return;
            }
        };
        // Pull the branch name so the modal can offer "Also delete
        // branch X" — None for default / detached lanes (modal hides
        // the checkbox).
        let branch = self.lane_for(target).and_then(|w| match &w.kind {
            daruda_store::project::LaneKind::Git {
                branch: Some(b), ..
            } => Some(b.clone()),
            _ => None,
        });
        let ws_for_modal = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            crate::surface::strings::modal::remove_lane_title(),
            None,
            move |window, cx| {
                crate::workspace::left_dock::projects::remove_modal::RemoveWorktreeModal::new(
                    ws_for_modal.clone(),
                    target,
                    label,
                    path,
                    plan,
                    window,
                    cx,
                )
                .with_branch(branch)
            },
            window,
            cx,
        );
    }

    /// Remove the currently active lane/project when its root directory
    /// is inaccessible (missing / access-denied). Routes by kind:
    /// removable git worktrees open the Remove-lane modal; a main or
    /// default lane stands in for the whole project, so it opens the
    /// delete-project chooser instead (there is no `git worktree
    /// remove` for the main checkout). No-op when the active lane is
    /// `Present` or absent — the affordance is only offered for the
    /// inaccessible empty-state.
    pub(in crate::workspace) fn request_remove_inaccessible_active(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.active_lane() else {
            return;
        };
        if active.availability == LaneAvailability::Present {
            return;
        }
        let target = self.active;
        if Self::lane_removable(active) {
            self.open_remove_lane_modal(target, window, cx);
        } else {
            self.open_delete_active_project_modal(window, cx);
        }
    }
}
