//! Lane lifecycle operations on `Workspace`: create / remove
//! validation + execution, modal openers, slot-id allocation, and the
//! lane activation path (re-points `active` into the single runtimes map).
//!
//! All git CLI calls go through `cx.background_executor` so the
//! UI thread never blocks; the post-git state mutations come back via
//! `cx.update` to touch GPUI entities and `Window`. Modal state lives
//! on `Workspace` (not in this file) because it has to outlive any
//! single render cycle.

use daruda_store::project::{LaneId, LaneRef, ProjectId};
use daruda_store::tasks::TaskAgentSurface;
use gpui::{Context, Window};

use super::LaneRuntime;
use super::ToggleLaneSwitcher;
use super::Workspace;
use super::command::lane_switcher::LaneCandidate;
use crate::lane::availability::LaneAvailability;
use crate::workspace::main_area::agent_chat_pane::agent_chat_ops::resolve_open_agent_id;
use crate::workspace::main_area::pane;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

/// Immutable plan produced by `CreateWorktreeModal::validate` — holds
/// the sanitized branch, derived new-path, and repo_root so the modal
/// can ship clones to a background executor without borrowing
/// `Workspace`.
#[derive(Debug, Clone)]
pub(in crate::workspace) struct CreateWorktreePlan {
    pub branch: String,
    pub new_path: std::path::PathBuf,
    pub repo_root: std::path::PathBuf,
    /// Ref the new branch should be based on (e.g. `main`,
    /// `origin/main`). `None` = whatever git's `lane add` defaults
    /// to (current HEAD when paired with `-b`).
    pub base_ref: Option<String>,
    /// Free-form description captured at creation time.
    /// Surfaced as the lane row sublabel in the left dock.
    pub description: Option<String>,
}

/// Counterpart to `CreateWorktreePlan` for the remove path.
#[derive(Debug)]
pub(in crate::workspace) struct RemoveWorktreePlan {
    pub path: std::path::PathBuf,
    pub repo_root: std::path::PathBuf,
}

impl Workspace {
    /// Switch to the nth lane of the active project by left-dock
    /// position (0-indexed). Lanes are sorted by `tab_order` so the
    /// position matches what the user sees in the left dock. No-ops
    /// when `index` is out of range or no project is loaded.
    pub(in crate::workspace) fn activate_lane_by_index(
        &mut self,
        index: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(active_project_id) = self.active_project().map(|p| p.id) else {
            return;
        };
        let mut ids: Vec<(u32, LaneId)> = self
            .active_lanes()
            .iter()
            .map(|w| (w.tab_order, w.id))
            .collect();
        ids.sort_unstable_by_key(|&(order, _)| order);
        if let Some(&(_, id)) = ids.get(index) {
            self.activate_lane(
                LaneRef {
                    project: active_project_id,
                    lane: id,
                },
                window,
                cx,
            );
        }
    }

    // ---- Lane switcher (Cmd+P) ----

    /// Toggle the fuzzy Lane switcher. On open, snapshot one candidate
    /// per lane across every project so the overlay render reads a
    /// frozen list (MVU: render never reaches into live project state).
    pub(in crate::workspace) fn on_toggle_lane_switcher(
        &mut self,
        _: &ToggleLaneSwitcher,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_switcher.is_open {
            self.lane_switcher.close();
        } else {
            let candidates = self.lane_switcher_candidates();
            self.lane_switcher.open(candidates);
        }
        cx.notify();
    }

    /// One [`LaneCandidate`] per lane across every project, labelled
    /// `"<project> / <lane>"`.
    fn lane_switcher_candidates(&self) -> Vec<LaneCandidate> {
        self.projects
            .iter()
            .flat_map(|project| {
                let project_id = project.id;
                let project_name = project.name.clone();
                project.lanes.iter().map(move |lane| LaneCandidate {
                    lane_ref: LaneRef {
                        project: project_id,
                        lane: lane.id,
                    },
                    label: format!("{} / {}", project_name, lane.display_name()),
                })
            })
            .collect()
    }

    /// Activate the focused lane and close the switcher. Mirrors
    /// `execute_palette_action`'s shape.
    pub(in crate::workspace) fn execute_lane_switcher_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.lane_switcher.focused_lane_ref();
        self.lane_switcher.close();
        cx.notify();
        if let Some(target) = target {
            self.activate_lane(target, window, cx);
        }
    }

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
    /// path) for the shell-out. Pure — does not mutate state, so it's
    /// safe to call from tests or action handlers without side effects.
    pub(in crate::workspace) fn validate_remove_lane(
        &self,
        target: LaneRef,
    ) -> Result<RemoveWorktreePlan, String> {
        let wt = self
            .lane_for(target)
            .ok_or_else(|| "Lane not found.".to_string())?;
        if !Self::lane_removable(wt) {
            return Err("This lane cannot be removed.".to_string());
        }
        let repo_root = match &wt.kind {
            daruda_store::project::LaneKind::Git { repo_root, .. } => repo_root.clone(),
            _ => return Err("Not a git worktree.".to_string()),
        };
        Ok(RemoveWorktreePlan {
            path: wt.path.clone(),
            repo_root,
        })
    }

    /// Post-git cleanup on the UI thread: switch away if active, then
    /// drop the entry and its runtime. Invariant: the active lane
    /// is always survivable because `validate_remove_lane` refuses
    /// main/default kinds, so there's always at least one other entry
    /// in the project.
    pub(in crate::workspace) fn finalize_remove_lane(
        &mut self,
        target: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Capture the lane path before removal so the per-lane
        // entries in the `SkillsState` / `McpState` Globals can be
        // pruned — otherwise the BTreeMap grows unbounded across the
        // session as lanes come and go.
        let removed_path = self.lane_for(target).map(|w| w.path.clone());

        // Pick the fallback `LaneRef` to activate when the removal
        // strips the currently active lane. Prefer a sibling
        // lane in the same project (Project membership stays put),
        // then fall back to *another* project's snap target so the
        // status bar / window title / pane runtime never wedge on a
        // dangling ref. Returning `None` here only happens in the
        // degenerate case where the workspace is about to be empty —
        // the section-header `[+]` is hidden for non-git workspaces so
        // in practice main lanes keep at least one per project.
        let fallback_target: Option<LaneRef> = if self.active == target {
            let same_project = self
                .projects
                .iter()
                .find(|p| p.id == target.project)
                .and_then(|p| {
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
        // Release the removed lane's panes from PTY tracking before
        // their runtime drops — otherwise the tracker keeps walking
        // the dead shell PIDs for the window's lifetime. Gated on
        // `target != self.active`: if no fallback was found above
        // (`fallback_target` was `None` — the degenerate about-to-be-
        // empty case) `target` is still the active lane, and dropping
        // its runtime would violate the "active runtime always present"
        // invariant. The window is closing in that case, so leave it.
        let removed_pane_ids: Vec<_> = self
            .main_area
            .runtimes
            .get(&target)
            .map(|runtime| runtime.panes.iter().map(|p| p.id).collect::<Vec<_>>())
            .unwrap_or_default();
        if target != self.active {
            self.release_pane_tracking(&removed_pane_ids, cx);
            self.main_area.runtimes.remove(&target);
        }
        // W-7 per-lane state must be cleared too — otherwise the
        // notify watcher keeps running, the cache holds stale paths,
        // and the gitignore matcher leaks. Dropping the entries also
        // drops the embedded `RecommendedWatcher`, which stops the
        // kernel-side watch.
        self.file_tree.file_trees.remove(&target);
        self.file_tree.file_watchers.remove(&target);
        self.file_tree.files_reload_queues.remove(&target);
        self.file_tree.files_visible_cache.remove(&target);
        self.file_tree.files_gitignore_index.remove(&target);
        self.git_status_in_flight.remove(&target);
        self.git_status_pending_repeat.remove(&target);
        self.git_status_cache.remove(&target);
        self.git_collapsed_dirs.remove(&target);
        self.git_changes_cursor.remove(&target);
        // Bottom-dock drafts are keyed per pane, so drop the entry for
        // every pane that lived in the removed lane; clear `input_owner`
        // if it pointed at one of them (the visible text then belongs to
        // no live pane).
        for pane_id in &removed_pane_ids {
            self.forget_pane_input_draft(*pane_id);
        }
        self.input_history.remove(&target);
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == target.project) {
            project.lanes.retain(|w| w.id != target.lane);
        }
        if let Some(path) = removed_path {
            use gpui::BorrowAppContext as _;
            cx.update_global::<crate::agent::skills::SkillsState, _>(|s, _| {
                s.forget_lane(&path);
            });
            cx.update_global::<crate::agent::mcp::McpState, _>(|s, _| {
                s.forget_lane(&path);
            });
            // Drop the per-lane slice from `SettingsStore` so its
            // BTreeMap doesn't grow unbounded across the session
            // (CLAUDE.md §"GPUI shared-state convention — Cleanup
            // rule").
            cx.update_global::<crate::settings_store::SettingsStore, _>(|s, _| {
                s.forget_lane(&path);
            });
        }
        // The watcher's pair list is fixed at spawn time, so removing
        // a lane leaves a dead `~/.claude/projects/<encoded>/`
        // entry under FSEvents subscription — re-spawn so the lookup
        // table matches the live lane set.
        self.refresh_jsonl_watcher(cx);
        // Project skill scope is anchored to the active lane's
        // root; restart the watcher so it tracks the new active path.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        // Empty closure: see group_ops.rs:83 for rationale. The preceding
        // refresh_* calls finish the mutation chain; the wrapper only needs
        // to schedule persist here.
        self.mutate_durable(cx, |_, _| {});
        // Force a render even when no fallback fired — the lane
        // list shrank by one and the dock would otherwise hold a row
        // for the now-removed lane until an unrelated render.
        // Same defensive notify shape `add_project` uses.
        cx.notify();
    }

    /// Resolve the active git repo_root from the active project's
    /// lane list. Returns `None` when the workspace isn't backed by
    /// a git repo. Used by callers (e.g. the `[+]` button) that need to
    /// construct a `CreateWorktreeModal` without traversing the
    /// lane list.
    pub(in crate::workspace) fn git_repo_root(&self) -> Option<std::path::PathBuf> {
        self.active_lanes().iter().find_map(|w| match &w.kind {
            daruda_store::project::LaneKind::Git { repo_root, .. } => Some(repo_root.clone()),
            _ => None,
        })
    }

    /// Post-git UI-thread work: spawn a pane at the new checkout,
    /// wrap it in a Tab / LaneRuntime, register the Lane
    /// entry, and activate the newcomer. Errors here leave the freshly
    /// created git worktree orphaned on disk (the user can clean up
    /// via `git worktree prune`) and bubble the message back so the
    /// modal shows it.
    pub(in crate::workspace) fn finalize_create_lane(
        &mut self,
        plan: CreateWorktreePlan,
        project_id: ProjectId,
        surface: TaskAgentSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, String> {
        if self.project_for(project_id).is_none() {
            return Err(crate::surface::strings::create_lane_err_no_active_project());
        }
        let CreateWorktreePlan {
            branch,
            new_path,
            repo_root,
            base_ref,
            description,
        } = plan;
        // The lane's initial pane mirrors the requested surface: a terminal
        // (default, incl. every manual lane creation) or an in-app Agent chat
        // session rooted at the new checkout. `create_agent_chat_pane` parks
        // Idle and connects lazily on first focus — the `activate_lane` below
        // focuses it, which starts the ACP session.
        let pane = match surface {
            TaskAgentSurface::Terminal => self
                .create_pane_with_cwd(Some(new_path.clone()), window, cx)
                .map_err(|e| e.to_string())?,
            TaskAgentSurface::AgentChat => {
                // No source agent-chat pane here (a new lane), so open under the
                // session-sticky default — matching `open_agent_chat_pane`.
                // `CreateWorktreePlan` carries no `remote_cwd` (a brand-new lane
                // only gets one via the sidebar's right-click, after creation),
                // so an agent whose command needs `{{cwd}}` parks in the
                // actionable "no remote path set" error here rather than
                // connecting.
                let agent_id = resolve_open_agent_id(&self.agents, self.last_agent_id.as_deref());
                self.create_new_agent_chat_pane(agent_id, Some(new_path.clone()), None, window, cx)
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
        // A freshly-added linked lane's toplevel is exactly
        // `new_path` — `git worktree add` writes the per-lane
        // `.git` pointer file there. No anchoring happens at this
        // point either, so `path` and `worktree_root` start equal.
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
        if let Some(project) = self.project_for_mut(project_id) {
            project.lanes.push(wt);
        }

        let runtime = LaneRuntime {
            tabs: vec![tab],
            panes: vec![pane],
            active_tab_index: 0,
            tab_history: Vec::new(),
            focused_pane_id: pane_id,
        };
        self.main_area.runtimes.insert(new_ref, runtime);
        self.activate_lane(new_ref, window, cx);
        // New cwd → new `~/.claude/projects/<encoded>/` to watch.
        self.refresh_jsonl_watcher(cx);
        // New lane root → new project-skills directory to watch.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        // Returning the spawned pane's id lets task-driven callers
        // (start_task) write into that PTY directly instead of
        // relying on `focused_pane_id`, which races with whatever
        // focus state activate_lane leaves behind on edge cases.
        Ok(pane_id)
    }

    /// Monotonic lane id allocator scoped to `project_id`.
    /// Walks both the lane list and every registered runtime key
    /// so a phantom key from a crash mid-remove never collides with a
    /// fresh id.
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

    /// Switch the visible lane. Every lane's runtime lives in the single
    /// `runtimes` map; the active lane is the entry under `self.active`,
    /// so activating a different one just re-points that key — no fields
    /// are moved and every parked lane's PTY entities keep running. If the
    /// target has never been populated (e.g. `bootstrap_from_project`
    /// loaded its metadata from `git worktree list` but no tabs were
    /// serialized), a fresh pane is spawned at the lane's path so the user
    /// never lands on an empty viewport.
    pub(in crate::workspace) fn activate_lane(
        &mut self,
        target: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active == target {
            self.reactivate_active_lane(window, cx);
            return;
        }
        if self.lane_for(target).is_none() {
            return;
        }
        // Leaving the active lane abandons any pending amend — the prefilled
        // message belongs to the lane we're leaving. No-op when not amending.
        self.exit_amend_mode(window, cx);
        // Re-classify the incoming lane's root against the live
        // filesystem before any path-dependent work runs below. A lane
        // whose directory vanished while inactive must flip to
        // non-`Present` here so the lazy-seed (PTY spawn) and the
        // path-dependent tail are skipped.
        self.recompute_availability_for(target);
        // Trigger #5 — active lane change. Both the previous and the
        // incoming visible lists become stale (selection moves with the
        // active id).
        let previous = self.active;
        self.invalidate_visible_files_cache(previous);
        self.invalidate_visible_files_cache(target);
        // Clear keyboard cursor — it lived in the previous lane's
        // visible list.
        self.file_tree.files_selection = None;

        // 1. Re-point the active key. Both lanes' runtimes already live in
        //    the single `runtimes` map, so there is nothing to freeze or
        //    swap — the previous lane's runtime stays under its key and the
        //    target's stays under its. A lane loaded from `git worktree
        //    list` but never activated has no entry yet; `entry().or_default`
        //    seeds an empty one so `active_runtime()` resolves immediately.
        self.main_area.runtimes.entry(target).or_default();
        // Drop any in-flight drag hover so a stale half-fill overlay does not
        // linger on the newly-activated lane. The notify paths below cover it.
        self.main_area.pane_drop_hover = None;

        self.active = target;

        // The bottom-dock draft is swapped per input pane by
        // `set_focused_pane`, not per lane: a lane switch changes the
        // focused pane, so the swap runs on the focus path below when the
        // incoming lane has a focused pane. An empty lane has none, so the
        // draft stays put.

        // Update the project's last-active-lane hint so clicking
        // the project header in the left dock snaps to the same
        // lane the user just left.
        if let Some(project) = self.projects.iter_mut().find(|p| p.id == target.project) {
            project.last_active_lane_id = target.lane;
        }
        // File viewer panes live in their own lane's runtime entry, so
        // each lane retains its own open files across activations.

        // An unavailable lane (root deleted / unreadable) becomes the
        // active lane — `active` now points at its (possibly empty)
        // runtime entry — but the path-spawning work is skipped:
        // lazy-seed would root a PTY at the dead path, and
        // `refresh_git_status` would shell out `git status` against it
        // and spam an error toast. The right-dock reconcile + persist
        // below are filesystem-tolerant (skills/mcp scan a missing dir as
        // empty; the commit button reads in-memory cache) so they still
        // run — otherwise the panels would keep showing the *previous*
        // lane's data after the switch.
        if self
            .lane_for(target)
            .map(|l| l.availability != LaneAvailability::Present)
            .unwrap_or(false)
        {
            self.reconcile_right_dock_for_inaccessible_lane(window, cx);
            // Persist the active-ref change (`self.active = target`) so a
            // quit right after selecting a missing lane keeps the
            // selection. The Present path persists via the tail below.
            self.mutate_durable(cx, |_, _| {});
            cx.notify();
            return;
        }

        // 3. No lazy seed. An empty lane (never opened, or emptied by the
        //    user) stays empty and renders the "No open tabs" empty-state;
        //    the user opens content from there. Only the first project at
        //    app launch seeds a shell (see `new_with_project_impl`).

        // 4. Refocus the active pane and request a resize — the
        //    lane may have been last seen at a different viewport.
        //    Route through `set_focused_pane` so the bottom-dock draft
        //    swaps to the newly-focused pane. An empty lane has no pane to
        //    focus (guarded below), so the draft simply stays put.
        if self
            .active_runtime()
            .panes
            .iter()
            .any(|p| p.id == self.active_runtime().focused_pane_id)
        {
            let focused = self.active_runtime().focused_pane_id;
            self.set_focused_pane(focused, window, cx);
            self.focus_pane(focused, window, cx);
        }
        // The incoming lane's runtime carries its own panes/split state;
        // recompute inactive-pane dim against the now-live focused pane.
        self.refresh_pane_dimming(cx);
        self.main_area.pending_resize = true;
        // The now-active lane's runtime may still hold File panes with
        // `Loading` content (a lane restored but never activated defers
        // its loads); fire them now that the lane is active.
        self.load_pending_file_panes(cx);
        self.mutate_durable(cx, |_, _| {});
        // 5. If the incoming lane's tree was modified while
        //    inactive, replay a single Bulk reload to catch up.
        self.replay_files_dirty(target, cx);
        // Project skill scope follows the active lane's path —
        // re-spawn so the panel switches to the new repo's skills.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        // Commit button reflects the active lane's staged count —
        // recompute now that `active` has flipped.
        self.sync_commit_buttons(cx);
        // Re-probe git status for the lane we just switched to so the
        // left-dock branch label catches up to any checkout done while
        // it was inactive — `refresh_git_status` reconciles
        // `Lane.kind.branch` from the live `git status` result.
        self.refresh_git_status(target, cx);

        cx.notify();
    }

    /// Same-lane click handler (the `self.active == target` fast path of
    /// [`Self::activate_lane`]). A healthy already-active lane is the
    /// common case and stays cheap: re-probe, see it's still `Present`
    /// with tabs, fall through to `notify`.
    ///
    /// The case that matters is self-healing: if the active lane was
    /// `Missing` / `AccessDenied` (root deleted, or Full Disk Access not
    /// yet granted) and the user has since recreated the directory or
    /// granted access, clicking the same row must re-probe and recover.
    /// `activate_lane`'s cross-lane `recompute_availability_for` never
    /// runs for a same-lane click, so without this re-probe the lane
    /// stays stuck non-`Present` until the user switches away and back.
    ///
    /// On recovery to `Present` the lane is re-probed and the right dock
    /// re-pointed, but no tab is seeded: an empty recovered lane lands on
    /// the "No open tabs" empty-state (the unified rule). Still non-`Present`
    /// → re-point the right dock (it may have nothing reflected yet) and
    /// stay put.
    fn reactivate_active_lane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active;
        let was_present = self
            .lane_for(active)
            .map(|l| l.availability == LaneAvailability::Present)
            .unwrap_or(false);
        // Healthy lane that already has tabs: nothing a same-lane click
        // should act on — keep the fast path cheap (never re-probe or
        // disturb live terminals). A `Present` lane with *no* tabs (user
        // closed them all, or it was never opened) deliberately falls
        // through so the re-probe below can recover availability; it still
        // renders the empty-state afterward.
        if was_present && !self.active_runtime().tabs.is_empty() {
            return;
        }
        self.recompute_availability_for(active);
        let now_present = self
            .lane_for(active)
            .map(|l| l.availability == LaneAvailability::Present)
            .unwrap_or(false);
        if now_present {
            // Recovered to Present. No tab is seeded — an empty lane renders
            // the empty-state; the user opens content from there. Focus only
            // if the lane already had a pane (guarded), then run the
            // path-dependent right-dock reconcile the empty-state had skipped.
            if self.has_focused_pane() {
                self.focus_pane(self.active_runtime().focused_pane_id, window, cx);
            }
            self.main_area.pending_resize = true;
            // The recovered availability is persisted runtime state.
            self.mutate_durable(cx, |_, _| {});
            // Deliberately omits `activate_lane`'s `load_pending_file_panes`
            // + `replay_files_dirty`: a lane reaching here was either Missing
            // all session (blank runtime — no Loading file panes, no dirty
            // backlog) or a Present lane whose tabs were all closed (likewise
            // nothing pending). Add them only if a future path can leave
            // stale file-view panes on a recovered lane.
            self.refresh_skills_watcher(cx);
            self.refresh_mcp_watcher(window, cx);
            self.sync_commit_buttons(cx);
            self.refresh_git_status(active, cx);
        } else {
            // Still inaccessible — re-point the right dock so it shows the
            // empty (inaccessible) lane rather than stale data.
            self.reconcile_right_dock_for_inaccessible_lane(window, cx);
        }
        cx.notify();
    }

    /// Re-point the right dock (Skills / Tools panels + commit button) at
    /// the now-active lane when that lane is inaccessible. Deliberately
    /// excludes `refresh_git_status` — it shells out `git status` against
    /// the dead path and would spam an error toast. The skills / mcp
    /// watchers scan a missing directory as empty and the commit button
    /// reads the in-memory cache (absent → disabled), so all three show
    /// an empty panel instead of the previous lane's data.
    fn reconcile_right_dock_for_inaccessible_lane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.sync_commit_buttons(cx);
    }

    /// Move `from` immediately before `to` in the lanes list of
    /// the project they share, renumbering `tab_order` for all
    /// lanes afterwards. Lane DnD is intentionally scoped to a
    /// single project — cross-project drops are rejected as a no-op so
    /// a lane never migrates between projects through the dock.
    /// Also no-ops when `from == to`, when the project is missing, or
    /// when either id is not present in that project.
    pub(in crate::workspace) fn reorder_lane(
        &mut self,
        from: LaneRef,
        to: LaneRef,
        cx: &mut Context<Self>,
    ) {
        if from == to {
            return;
        }
        if from.project != to.project {
            return;
        }
        let Some(project) = self.projects.iter_mut().find(|p| p.id == from.project) else {
            return;
        };
        let from_idx = project.lanes.iter().position(|w| w.id == from.lane);
        let to_idx = project.lanes.iter().position(|w| w.id == to.lane);
        let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else {
            return;
        };
        // Insert at `to`'s ORIGINAL index. Direction-aware via the
        // implicit shift caused by `remove`: when `from_idx < to_idx`
        // (downward drag) the removal shifts `to` down one slot, so
        // inserting at `to_idx` lands AFTER the now-shifted `to`;
        // when `from_idx > to_idx` (upward drag) `to` keeps its
        // index, so inserting at `to_idx` lands BEFORE `to`. This
        // matches the standard list-DnD expectation that "drop X
        // onto Y's slot" makes X take Y's row regardless of drag
        // direction.
        let item = project.lanes.remove(from_idx);
        project.lanes.insert(to_idx, item);
        for (i, w) in project.lanes.iter_mut().enumerate() {
            w.tab_order = i as u32;
        }
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// Apply `f` to the lane named by the full [`LaneRef`], then persist +
    /// notify iff a lane was actually found. Resolves the project by
    /// `target.project` — never the *active* project — because lane ids
    /// restart per project (every project has a lane id 0), so a bare
    /// `LaneId` against the active project would silently mutate the wrong
    /// project's like-id'd lane (a left-dock edit on a background project
    /// bleeding into the foreground one).
    fn mutate_lane<F>(&mut self, target: LaneRef, f: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&mut crate::lane::Lane),
    {
        let mutated = self
            .project_for_mut(target.project)
            .and_then(|p| p.lanes.iter_mut().find(|w| w.id == target.lane))
            .map(f)
            .is_some();
        if mutated {
            self.mutate_durable(cx, |_, _| {});
            cx.notify();
        } else {
            // The target lane/project vanished between the affordance being
            // built (context-menu open) and the edit landing — e.g. the lane
            // was removed while the edit modal was open. Log rather than
            // silently drop the user's typed value. No toast: it is a rare
            // race and one per stale submit would be noisier than useful.
            let report = daruda_store::observability::error_report::ErrorReport::new(
                "Lane edit dropped: target no longer exists",
            )
            .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
            .with_context("project", target.project.to_string())
            .with_context("lane", target.lane.to_string())
            .at(file!(), line!())
            .dedup("lane.mutate.stale_target")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
    }

    /// Update the free-form description for the lane named by `target`.
    /// `None` clears it, reverting the left dock sublabel to the lane path.
    pub(in crate::workspace) fn set_lane_description(
        &mut self,
        target: LaneRef,
        description: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_description(description), cx);
    }

    /// Update the remote path the lane named by `target` connects
    /// its session to. `None` reverts the lane to the local
    /// filesystem. Only affects panes created *after* this call —
    /// `resolve_new_pane_cwd` resolves the remote cwd once, at
    /// pane-creation time, and never re-runs for panes already open.
    pub(in crate::workspace) fn set_lane_remote_cwd(
        &mut self,
        target: LaneRef,
        remote_cwd: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_remote_cwd(remote_cwd), cx);
    }

    /// Update the user-visible display name for the lane named by `target`.
    /// `None` clears it and reverts to the branch / path fallback.
    pub(in crate::workspace) fn set_lane_name(
        &mut self,
        target: LaneRef,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.mutate_lane(target, |wt| wt.set_name(name), cx);
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
        let target_id = wt.id;
        let label = gpui::SharedString::from(wt.display_name());
        let path = gpui::SharedString::from(wt.path.to_string_lossy().into_owned());
        let plan = match self.validate_remove_lane(target) {
            Ok(p) => p,
            Err(msg) => {
                let report = daruda_store::observability::error_report::ErrorReport::new(
                    "Cannot remove worktree",
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
            crate::surface::strings::remove_lane_modal_title(),
            None,
            move |window, cx| {
                super::left_dock::projects::remove_modal::RemoveWorktreeModal::new(
                    ws_for_modal.clone(),
                    target_id,
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

    /// Resolve the base ref for a new lane against the active project
    /// (lanes are created into `active_project()`, matching
    /// `finalize_create_lane`). Single resolution point — both
    /// `add_lane` consumers route through this so the recorded base
    /// always matches what git actually branched from.
    pub(in crate::workspace) fn resolve_lane_base_ref(
        &self,
        requested: Option<String>,
    ) -> Option<String> {
        let p = self.active_project();
        resolved_lane_base_ref(
            requested,
            p.and_then(|p| p.base_branch.as_deref()),
            p.and_then(|p| p.default_branch.as_deref()),
        )
    }
}

/// Effective base ref for a new lane: an explicit user choice wins;
/// otherwise fall back to the project's base branch, then its
/// detected default branch; `None` lets git use the current HEAD.
fn resolved_lane_base_ref(
    requested: Option<String>,
    base_branch: Option<&str>,
    default_branch: Option<&str>,
) -> Option<String> {
    requested.or_else(|| base_branch.or(default_branch).map(str::to_owned))
}

/// Minimal branch-name guard — blocks the obvious foot-guns that git
/// itself would reject (spaces, path-reserved characters, `..`,
/// leading/trailing slash, control chars). Git's full rule set is
/// richer; this is a preflight filter so the modal can surface a
/// helpful error before shelling out.
pub(in crate::workspace) fn sanitize_branch_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // git-check-ref-format rules we preflight here: no `..`, no
    // leading/trailing `/`, no leading `.` (rule 6), no shell-hostile
    // or protocol-reserved characters, no control chars.
    if trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
    {
        return None;
    }
    let bad = [' ', ':', '~', '^', '?', '*', '[', '\\'];
    if trimmed.chars().any(|c| bad.contains(&c) || c.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::resolved_lane_base_ref;

    #[test]
    fn explicit_request_overrides_project_branches() {
        let out = resolved_lane_base_ref(
            Some("origin/feat".to_string()),
            Some("develop"),
            Some("main"),
        );
        assert_eq!(out.as_deref(), Some("origin/feat"));
    }

    #[test]
    fn falls_back_to_base_branch_when_unspecified() {
        let out = resolved_lane_base_ref(None, Some("develop"), Some("main"));
        assert_eq!(out.as_deref(), Some("develop"));
    }

    #[test]
    fn falls_back_to_default_branch_when_no_base() {
        let out = resolved_lane_base_ref(None, None, Some("main"));
        assert_eq!(out.as_deref(), Some("main"));
    }

    #[test]
    fn returns_none_when_nothing_known() {
        let out = resolved_lane_base_ref(None, None, None);
        assert_eq!(out, None);
    }
}
