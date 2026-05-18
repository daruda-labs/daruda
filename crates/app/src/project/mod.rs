//! Runtime [`Project`] — the workspace-visible counterpart to
//! [`daruda_store::project::SerializedProject`].
//!
//! A project owns a root directory, a non-empty list of runtime
//! [`crate::worktree::Worktree`]s, a "last active worktree" hint for
//! the left dock, and visual metadata (color, tab order, group).
//!
//! GPUI-free: lives alongside [`crate::worktree`] in dependency order
//! `workspace/ → project/ → worktree/`. Workspace assembles a
//! `Vec<Project>` at construction time; persistence reads/writes the
//! serialized mirror in [`daruda_store::project::SerializedProject`].

use std::path::PathBuf;

use daruda_store::project::{
    GroupId, ProjectId, SerializedProject, WorktreeId, WorktreeRef, derive_name_from_path,
};
use gpui::BackgroundExecutor;

use crate::worktree::Worktree;

/// Runtime project entry. Contains the runtime worktrees plus the
/// metadata needed to render the left-dock tree (color, tab order,
/// group membership). One `Project` per opened repository root.
#[derive(Debug)]
pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    pub worktrees: Vec<Worktree>,
    /// Worktree the user last clicked inside this project. Snap target
    /// when the project header is clicked in the left dock without a
    /// specific worktree pick. Always a member of `worktrees`.
    pub last_active_worktree_id: WorktreeId,
    /// `None` = ungrouped (rendered at top level alongside groups in
    /// the same `tab_order` pool).
    pub group_id: Option<GroupId>,
    /// Optional hex color for the project header in the left dock.
    pub color: Option<String>,
    /// Sort order within `Workspace::projects`. Renumbered to `0..N`
    /// by DnD operations so the integer is dense.
    pub tab_order: u32,
}

impl Project {
    /// Build a runtime project from a freshly opened directory. Walks
    /// the filesystem to discover git worktrees (or falls back to a
    /// single `Default` worktree for non-git paths) via
    /// [`Worktree::bootstrap_from_project`]. The default `id` is `0` —
    /// `Workspace::add_project` overwrites it with the monotonic id.
    pub fn bootstrap(id: ProjectId, root: PathBuf) -> Self {
        let name = derive_name_from_path(&root);
        let worktrees = Worktree::bootstrap_from_project(&root);
        let last_active_worktree_id = worktrees.first().map(|w| w.id).unwrap_or(0);
        Self {
            id,
            root,
            name,
            worktrees,
            last_active_worktree_id,
            group_id: None,
            color: None,
            tab_order: 0,
        }
    }

    /// Hydrate a runtime project from its persisted shape. Worktrees
    /// inflate via [`Worktree::from_serialized`]; tabs/panes are
    /// rebuilt separately by the workspace restore path.
    pub fn from_serialized(s: &SerializedProject) -> Self {
        let worktrees = s.worktrees.iter().map(Worktree::from_serialized).collect();
        Self {
            id: s.id,
            root: s.root.clone(),
            name: s.name.clone(),
            worktrees,
            last_active_worktree_id: s.last_active_worktree_id,
            group_id: s.group_id,
            color: s.color.clone(),
            tab_order: s.tab_order,
        }
    }

    /// Borrow a worktree by id.
    pub fn worktree(&self, id: WorktreeId) -> Option<&Worktree> {
        self.worktrees.iter().find(|w| w.id == id)
    }

    /// Mutably borrow a worktree by id.
    pub fn worktree_mut(&mut self, id: WorktreeId) -> Option<&mut Worktree> {
        self.worktrees.iter_mut().find(|w| w.id == id)
    }

    /// Worktree the project should focus when the user clicks the
    /// project header. Falls back to the first worktree when the saved
    /// hint no longer exists (deleted between sessions).
    pub fn snap_target(&self) -> Option<WorktreeRef> {
        let id = if self
            .worktrees
            .iter()
            .any(|w| w.id == self.last_active_worktree_id)
        {
            self.last_active_worktree_id
        } else {
            self.worktrees.first()?.id
        };
        Some(WorktreeRef {
            project: self.id,
            worktree: id,
        })
    }

    /// First worktree's `WorktreeRef` — used when the workspace
    /// activates a project that has never been focused before.
    pub fn first_worktree_ref(&self) -> Option<WorktreeRef> {
        self.worktrees.first().map(|w| WorktreeRef {
            project: self.id,
            worktree: w.id,
        })
    }

    /// Bootstrap N projects in parallel on `executor`. Each root spawns
    /// its own blocking task (the underlying `bootstrap_from_project`
    /// shells out to `git worktree list`), so the wall-time of restoring
    /// a multi-project workspace converges on `max(times)` instead of
    /// `sum(times)`.
    ///
    /// `ids` is paired with `roots` so each spawned `Project` lands on
    /// its persisted [`ProjectId`] rather than the default `0`. Caller
    /// must ensure `ids.len() == roots.len()`; mismatched lengths panic
    /// to prevent silent re-id of the wrong project.
    pub async fn bootstrap_many(
        ids: Vec<ProjectId>,
        roots: Vec<PathBuf>,
        executor: &BackgroundExecutor,
    ) -> Vec<Project> {
        assert_eq!(
            ids.len(),
            roots.len(),
            "bootstrap_many: ids and roots length mismatch"
        );
        if roots.is_empty() {
            return Vec::new();
        }
        let tasks: Vec<_> = ids
            .into_iter()
            .zip(roots.into_iter())
            .map(|(id, root)| executor.spawn(async move { Project::bootstrap(id, root) }))
            .collect();
        let mut out = Vec::with_capacity(tasks.len());
        for t in tasks {
            out.push(t.await);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_non_git_yields_default_worktree() {
        let dir = std::env::temp_dir().join("daruda_project_bootstrap_non_git");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = Project::bootstrap(0, dir.clone());
        assert_eq!(p.worktrees.len(), 1);
        assert_eq!(p.last_active_worktree_id, p.worktrees[0].id);
        assert!(p.group_id.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snap_target_falls_back_to_first_when_hint_stale() {
        let dir = std::env::temp_dir().join("daruda_project_snap_stale");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::bootstrap(7, dir.clone());
        p.last_active_worktree_id = 999; // not in p.worktrees
        let snap = p.snap_target().unwrap();
        assert_eq!(snap.project, 7);
        assert_eq!(snap.worktree, p.worktrees[0].id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui::test]
    async fn bootstrap_many_assigns_each_id(cx: &mut gpui::TestAppContext) {
        let dir_a = std::env::temp_dir().join("daruda_bootstrap_many_a");
        let dir_b = std::env::temp_dir().join("daruda_bootstrap_many_b");
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let executor = cx.executor();
        let projects =
            Project::bootstrap_many(vec![3, 5], vec![dir_a.clone(), dir_b.clone()], &executor)
                .await;
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].id, 3);
        assert_eq!(projects[0].root, dir_a);
        assert_eq!(projects[1].id, 5);
        assert_eq!(projects[1].root, dir_b);
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[gpui::test]
    async fn bootstrap_many_empty_returns_empty(cx: &mut gpui::TestAppContext) {
        let executor = cx.executor();
        let projects = Project::bootstrap_many(Vec::new(), Vec::new(), &executor).await;
        assert!(projects.is_empty());
    }
}
