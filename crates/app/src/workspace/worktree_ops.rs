//! Worktree lifecycle operations on `Workspace`: create / remove
//! validation + execution, modal openers, slot-id allocation, and the
//! tab-swap activation path.
//!
//! All git CLI calls go through `cx.background_executor` so the
//! UI thread never blocks; the post-git state mutations come back via
//! `cx.update` to touch GPUI entities and `Window`. Modal state lives
//! on `Workspace` (not in this file) because it has to outlive any
//! single render cycle.

use daruda_store::project::WorktreeId;
use gpui::{Context, Window};

use super::Workspace;
use super::WorktreeRuntime;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};
use crate::workspace::main_area::pane::{self, TabEntry};

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
    /// `origin/main`). `None` = whatever git's `worktree add` defaults
    /// to (current HEAD when paired with `-b`).
    pub base_ref: Option<String>,
    /// Free-form description captured at creation time.
    /// Surfaced as the worktree row sublabel in the left dock.
    pub description: Option<String>,
}

/// Counterpart to `CreateWorktreePlan` for the remove path.
#[derive(Debug)]
pub(in crate::workspace) struct RemoveWorktreePlan {
    pub path: std::path::PathBuf,
    pub repo_root: std::path::PathBuf,
}

impl Workspace {
    /// Switch to the nth worktree by left-dock position (0-indexed).
    /// Worktrees are sorted by `tab_order` so the position matches what
    /// the user sees in the left dock. No-ops when `index` is out of range.
    pub(in crate::workspace) fn activate_worktree_by_index(
        &mut self,
        index: usize,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let mut ids: Vec<(u32, WorktreeId)> =
            self.worktrees.iter().map(|w| (w.tab_order, w.id)).collect();
        ids.sort_unstable_by_key(|&(order, _)| order);
        if let Some(&(_, id)) = ids.get(index) {
            self.activate_worktree(id, window, cx);
        }
    }

    /// True when the target worktree can be removed via `git worktree
    /// remove`. Main worktrees (checkout at `repo_root`) and the
    /// default non-git stand-in are non-removable.
    pub(in crate::workspace) fn worktree_removable(wt: &crate::worktree::Worktree) -> bool {
        match &wt.kind {
            daruda_store::project::WorktreeKind::Git { repo_root, .. } => wt.path != *repo_root,
            daruda_store::project::WorktreeKind::Default => false,
        }
    }

    /// Validate that `id` is removable and resolve its (repo_root,
    /// path) for the shell-out. Pure — does not mutate state, so it's
    /// safe to call from tests or action handlers without side effects.
    pub(in crate::workspace) fn validate_remove_worktree(
        &self,
        id: daruda_store::project::WorktreeId,
    ) -> Result<RemoveWorktreePlan, String> {
        let wt = self
            .worktrees
            .iter()
            .find(|w| w.id == id)
            .ok_or_else(|| "Worktree not found.".to_string())?;
        if !Self::worktree_removable(wt) {
            return Err("This worktree cannot be removed.".to_string());
        }
        let repo_root = match &wt.kind {
            daruda_store::project::WorktreeKind::Git { repo_root, .. } => repo_root.clone(),
            _ => return Err("Not a git worktree.".to_string()),
        };
        let _ = id; // id returned via the caller's own variable
        Ok(RemoveWorktreePlan {
            path: wt.path.clone(),
            repo_root,
        })
    }

    /// Post-git cleanup on the UI thread: switch away if active, then
    /// drop the entry and its runtime. Invariant: the active worktree
    /// is always survivable because `validate_remove_worktree` refuses
    /// main/default kinds, so there's always at least one other entry.
    pub(in crate::workspace) fn finalize_remove_worktree(
        &mut self,
        id: daruda_store::project::WorktreeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Capture the worktree path before removal so the per-worktree
        // entries in the `SkillsState` / `McpState` Globals can be
        // pruned — otherwise the BTreeMap grows unbounded across the
        // session as worktrees come and go.
        let removed_path = self
            .worktrees
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.path.clone());

        if self.active_worktree_id == id
            && let Some(fallback) = self.worktrees.iter().find(|w| w.id != id).map(|w| w.id)
        {
            self.activate_worktree(fallback, window, cx);
        }
        self.main_area.inactive_worktree_runtimes.remove(&id);
        // W-7 per-worktree state must be cleared too — otherwise the
        // notify watcher keeps running, the cache holds stale paths,
        // and the gitignore matcher leaks. Dropping the entries also
        // drops the embedded `RecommendedWatcher`, which stops the
        // kernel-side watch.
        self.file_tree.file_trees.remove(&id);
        self.file_tree.file_watchers.remove(&id);
        self.file_tree.files_reload_queues.remove(&id);
        self.file_tree.files_visible_cache.remove(&id);
        self.file_tree.files_gitignore_index.remove(&id);
        self.git_status_in_flight.remove(&id);
        self.git_status_pending_repeat.remove(&id);
        self.worktrees.retain(|w| w.id != id);
        if let Some(path) = removed_path {
            use gpui::BorrowAppContext as _;
            cx.update_global::<crate::agent::skills::SkillsState, _>(|s, _| {
                s.forget_worktree(&path);
            });
            cx.update_global::<crate::agent::mcp::McpState, _>(|s, _| {
                s.forget_worktree(&path);
            });
            // Drop the per-worktree slice from `SettingsStore` so its
            // BTreeMap doesn't grow unbounded across the session
            // (CLAUDE.md §"GPUI shared-state convention — Cleanup
            // rule").
            cx.update_global::<crate::settings_store::SettingsStore, _>(|s, _| {
                s.forget_worktree(&path);
            });
        }
        // The watcher's pair list is fixed at spawn time, so removing
        // a worktree leaves a dead `~/.claude/projects/<encoded>/`
        // entry under FSEvents subscription — re-spawn so the lookup
        // table matches the live worktree set.
        self.refresh_jsonl_watcher(cx);
        // Project skill scope is anchored to the active worktree's
        // root; restart the watcher so it tracks the new active path.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        self.mark_dirty_and_save(cx);
    }

    /// Resolve the active git repo_root from the worktree list. Returns
    /// `None` when the workspace isn't backed by a git repo. Used by
    /// callers (e.g. the `[+]` button) that need to construct a
    /// `CreateWorktreeModal` without traversing the worktree list.
    pub(in crate::workspace) fn git_repo_root(&self) -> Option<std::path::PathBuf> {
        self.worktrees.iter().find_map(|w| match &w.kind {
            daruda_store::project::WorktreeKind::Git { repo_root, .. } => Some(repo_root.clone()),
            _ => None,
        })
    }

    /// Post-git UI-thread work: spawn a pane at the new checkout,
    /// wrap it in a Tab / WorktreeRuntime, register the Worktree
    /// entry, and activate the newcomer. Errors here leave the freshly
    /// created git worktree orphaned on disk (the user can clean up
    /// via `git worktree prune`) and bubble the message back so the
    /// modal shows it.
    pub(in crate::workspace) fn finalize_create_worktree(
        &mut self,
        plan: CreateWorktreePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneId, String> {
        let CreateWorktreePlan {
            branch,
            new_path,
            repo_root,
            base_ref,
            description,
        } = plan;
        let pane = self
            .create_pane_with_cwd(Some(new_path.clone()), window, cx)
            .map_err(|e| e.to_string())?;
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        let tab = pane::TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        };

        let new_id = self.allocate_worktree_id();
        let order = self.worktrees.len() as u32;
        // A freshly-added linked worktree's toplevel is exactly
        // `new_path` — `git worktree add` writes the per-worktree
        // `.git` pointer file there. No anchoring happens at this
        // point either, so `path` and `worktree_root` start equal.
        let worktree_root = new_path.clone();
        let mut wt = crate::worktree::Worktree::git(
            new_id,
            new_path,
            Some(branch),
            repo_root,
            worktree_root,
            order,
        );
        wt.base_ref = base_ref;
        wt.description = description;
        self.worktrees.push(wt);

        let runtime = WorktreeRuntime {
            tabs: vec![tab],
            panes: vec![pane],
            active_tab_index: 0,
            tab_history: Vec::new(),
            focused_pane_id: pane_id,
        };
        self.main_area.inactive_worktree_runtimes.insert(new_id, runtime);
        self.activate_worktree(new_id, window, cx);
        // New cwd → new `~/.claude/projects/<encoded>/` to watch.
        self.refresh_jsonl_watcher(cx);
        // New worktree root → new project-skills directory to watch.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        // Returning the spawned pane's id lets task-driven callers
        // (start_task) write into that PTY directly instead of
        // relying on `focused_pane_id`, which races with whatever
        // focus state activate_worktree leaves behind on edge cases.
        Ok(pane_id)
    }

    /// Monotonic worktree id allocator. Walks both the worktree list
    /// and the stashed inactive runtimes so a phantom key from a
    /// crash mid-remove never collides with a fresh id.
    fn allocate_worktree_id(&self) -> daruda_store::project::WorktreeId {
        let max_list = self.worktrees.iter().map(|w| w.id).max();
        let max_map = self.main_area.inactive_worktree_runtimes.keys().copied().max();
        match (max_list, max_map) {
            (Some(a), Some(b)) => a.max(b) + 1,
            (Some(a), None) => a + 1,
            (None, Some(b)) => b + 1,
            (None, None) => 0,
        }
    }

    /// Switch the visible worktree. The Workspace's `tabs` / `panes`
    /// / focus fields represent the **active** worktree's runtime;
    /// activating a different one swaps those fields with the target
    /// worktree's stored runtime. If the target has never been
    /// populated (e.g. `bootstrap_from_project` loaded its metadata
    /// from `git worktree list` but no tabs were serialized), a fresh
    /// pane is spawned at the worktree's path so the user never lands
    /// on an empty viewport. PTY entities survive the swap because
    /// `_stdout_task` moves with the Pane rather than being cloned.
    pub(in crate::workspace) fn activate_worktree(
        &mut self,
        id: daruda_store::project::WorktreeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_worktree_id == id {
            // Same-worktree click is a no-op — the worktree is already
            // active and its tabs (including any file viewer panes)
            // stay in place.
            return;
        }
        if !self.worktrees.iter().any(|w| w.id == id) {
            return;
        }
        // Trigger #5 — active worktree change. Both the previous and the
        // incoming visible lists become stale (selection moves with the
        // active id).
        let previous = self.active_worktree_id;
        self.invalidate_visible_files_cache(previous);
        self.invalidate_visible_files_cache(id);
        // Clear keyboard cursor — it lived in the previous worktree's
        // visible list.
        self.file_tree.files_selection = None;

        // 1. Freeze the currently active runtime into the inactive map.
        let current = self.active_worktree_id;
        let frozen = WorktreeRuntime {
            tabs: std::mem::take(&mut self.main_area.tabs),
            panes: std::mem::take(&mut self.main_area.panes),
            active_tab_index: std::mem::take(&mut self.main_area.active_tab_index),
            tab_history: std::mem::take(&mut self.main_area.tab_history),
            focused_pane_id: std::mem::take(&mut self.main_area.focused_pane_id),
        };
        self.main_area.inactive_worktree_runtimes.insert(current, frozen);

        // 2. Pull the target worktree's runtime into the live fields.
        let next = self
            .main_area.inactive_worktree_runtimes
            .remove(&id)
            .unwrap_or_default();
        self.main_area.tabs = next.tabs;
        self.main_area.panes = next.panes;
        self.main_area.active_tab_index = next.active_tab_index;
        self.main_area.tab_history = next.tab_history;
        self.main_area.focused_pane_id = next.focused_pane_id;
        self.active_worktree_id = id;
        // File viewer panes travel with their worktree's tab list via
        // the `WorktreeRuntime` swap above, so each worktree retains
        // its own open files across activations.

        // 3. Lazy seed: a worktree loaded from `git worktree list` on
        //    startup has metadata but no runtime tabs. First-time
        //    activation spawns one pane rooted at the worktree path so
        //    the user lands in the right shell immediately. On PTY
        //    failure the error surfaces in the status bar and the
        //    viewport stays empty — still better than a silent black
        //    pane.
        if self.main_area.tabs.is_empty() {
            let cwd = self
                .worktrees
                .iter()
                .find(|w| w.id == id)
                .map(|w| w.path.clone());
            match self.create_pane_with_cwd(cwd, window, cx) {
                Ok(pane) => {
                    let pane_id = pane.id;
                    self.main_area.panes.push(pane);
                    let tab_id = self.alloc_id();
                    self.main_area.tabs.push(TabEntry {
                        id: tab_id,
                        layout: PaneLayout::Pane(pane_id),
                        last_focused_pane: pane_id,
                        user_label: None,
                    });
                    self.main_area.active_tab_index = 0;
                    self.main_area.focused_pane_id = pane_id;
                    self.bump_activity(pane_id);
                }
                Err(e) => {
                    self.report_pane_error("activate worktree", e, cx);
                }
            }
        }

        // 4. Refocus the active pane and request a resize — the
        //    worktree may have been last seen at a different viewport.
        if self.main_area.panes.iter().any(|p| p.id == self.main_area.focused_pane_id) {
            self.focus_pane(self.main_area.focused_pane_id, window, cx);
        }
        self.main_area.pending_resize = true;
        // Any File panes that arrived in the live `panes` vec via the
        // runtime swap above may still have `Loading` content (if they
        // came in from a restored-but-never-active runtime); fire
        // their loads now that the worktree is active.
        self.load_pending_file_panes(cx);
        self.mark_dirty_and_save(cx);
        // 5. If the incoming worktree's tree was modified while
        //    inactive, replay a single Bulk reload to catch up.
        self.replay_files_dirty(id, cx);
        // Project skill scope follows the active worktree's path —
        // re-spawn so the panel switches to the new repo's skills.
        self.refresh_skills_watcher(cx);
        self.refresh_mcp_watcher(window, cx);
        // Commit button reflects the active worktree's staged count —
        // recompute now that `active_worktree_id` has flipped.
        self.sync_commit_buttons(cx);
        cx.notify();
    }

    /// Move `from_id` immediately before `to_id` in the left dock list,
    /// renumbering `tab_order` for all worktrees afterwards. No-ops when
    /// either id is absent or both ids are the same.
    pub(in crate::workspace) fn reorder_worktree(
        &mut self,
        from_id: WorktreeId,
        to_id: WorktreeId,
        cx: &mut Context<Self>,
    ) {
        if from_id == to_id {
            return;
        }
        let from_idx = self.worktrees.iter().position(|w| w.id == from_id);
        let to_idx = self.worktrees.iter().position(|w| w.id == to_id);
        let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) else {
            return;
        };
        let item = self.worktrees.remove(from_idx);
        let insert_at = if from_idx < to_idx {
            to_idx - 1
        } else {
            to_idx
        };
        self.worktrees.insert(insert_at, item);
        for (i, w) in self.worktrees.iter_mut().enumerate() {
            w.tab_order = i as u32;
        }
        self.mark_dirty_and_save(cx);
        cx.notify();
    }

    /// Update the free-form description for `id`. `None` clears it,
    /// reverting the left dock sublabel to the worktree path.
    pub(in crate::workspace) fn set_worktree_description(
        &mut self,
        id: WorktreeId,
        description: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(wt) = self.worktrees.iter_mut().find(|w| w.id == id) {
            wt.set_description(description);
            self.mark_dirty_and_save(cx);
            cx.notify();
        }
    }

    /// Update the user-visible display name for `id`. `None` clears it
    /// and reverts to the branch / path fallback.
    pub(in crate::workspace) fn set_worktree_name(
        &mut self,
        id: WorktreeId,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(wt) = self.worktrees.iter_mut().find(|w| w.id == id) {
            wt.set_name(name);
            self.mark_dirty_and_save(cx);
            cx.notify();
        }
    }
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
