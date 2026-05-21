//! Runtime [`Project`] — the workspace-visible counterpart to the
//! persisted [`daruda_store::project::ProjectState`] plus the
//! per-workspace [`daruda_store::project::ProjectOverride`].
//!
//! A project owns a root directory, a non-empty list of runtime
//! [`crate::lane::Lane`]s, a "last active lane" hint for
//! the left dock, and visual metadata (color, tab order, group).
//!
//! GPUI-free: lives alongside [`crate::lane`] in dependency order
//! `workspace/ → project/ → lane/`. Workspace assembles a
//! `Vec<Project>` at construction time; persistence reads/writes via
//! [`crate::workspace::Workspace::snapshot_for_disk`] and
//! [`crate::workspace::Workspace::restore_from_disk`].

use std::path::PathBuf;

use daruda_store::project::{
    GroupId, LaneId, LaneRef, ProjectId, ProjectOverride, ProjectState, ProjectUuid,
    derive_name_from_path,
};
use gpui::BackgroundExecutor;

use crate::lane::Lane;

/// Runtime project entry. Contains the runtime lanes plus the
/// metadata needed to render the left-dock tree (color, tab order,
/// group membership). One `Project` per opened repository root.
#[derive(Debug)]
pub struct Project {
    pub id: ProjectId,
    /// Stable cross-session identifier — matches the UUID stored on
    /// disk at `projects/<uuid>.json`. Independent of the runtime
    /// `id` (which is per-workspace and not stable across sessions).
    pub uuid: ProjectUuid,
    pub root: PathBuf,
    pub name: String,
    pub lanes: Vec<Lane>,
    /// Lane the user last clicked inside this project. Snap target
    /// when the project header is clicked in the left dock without a
    /// specific lane pick. Always a member of `lanes`.
    pub last_active_lane_id: LaneId,
    /// `None` = ungrouped (rendered at top level alongside groups in
    /// the same `tab_order` pool).
    pub group_id: Option<GroupId>,
    /// Optional hex color for the project header in the left dock.
    pub color: Option<String>,
    /// Sort order within `Workspace::projects`. Renumbered to `0..N`
    /// by DnD operations so the integer is dense.
    pub tab_order: u32,
    /// True when the user has collapsed the project header in the
    /// left dock. The lane list is hidden when collapsed; click on
    /// the project chevron toggles. Persisted across sessions.
    pub is_collapsed: bool,
}

impl Project {
    /// Build a runtime project from a freshly opened directory. Walks
    /// the filesystem to discover git worktrees (or falls back to a
    /// single `Default` lane for non-git paths) via
    /// [`Lane::bootstrap_from_project`]. The default `id` is `0` —
    /// `Workspace::add_project` overwrites it with the monotonic id.
    pub fn bootstrap(id: ProjectId, root: PathBuf) -> Self {
        let name = derive_name_from_path(&root);
        let lanes = Lane::bootstrap_from_project(&root);
        let last_active_lane_id = lanes.first().map(|w| w.id).unwrap_or(0);
        Self {
            id,
            uuid: ProjectUuid::new(),
            root,
            name,
            lanes,
            last_active_lane_id,
            group_id: None,
            color: None,
            tab_order: 0,
            is_collapsed: false,
        }
    }

    /// Build a runtime project with a caller-supplied UUID. Used by
    /// code paths that need to attach this runtime entry to an existing
    /// on-disk [`daruda_store::project::ProjectState`] (policy B: the
    /// same root may appear in multiple workspaces, but each shares a
    /// single ProjectState identified by its UUID). Lane discovery
    /// is identical to [`Project::bootstrap`].
    pub fn new_with_uuid(id: ProjectId, uuid: ProjectUuid, root: PathBuf) -> Self {
        let name = derive_name_from_path(&root);
        let lanes = Lane::bootstrap_from_project(&root);
        let last_active_lane_id = lanes.first().map(|w| w.id).unwrap_or(0);
        Self {
            id,
            uuid,
            root,
            name,
            lanes,
            last_active_lane_id,
            group_id: None,
            color: None,
            tab_order: 0,
            is_collapsed: false,
        }
    }

    /// Hydrate a runtime project from the UUID-keyed on-disk shape —
    /// a [`ProjectState`] (intrinsic per-project fields) plus the
    /// per-workspace [`ProjectOverride`] (cosmetic decoration). Used
    /// by [`crate::workspace::Workspace::restore_from_disk`].
    pub fn from_disk(id: ProjectId, ps: &ProjectState, ov: &ProjectOverride) -> Self {
        let lanes = ps.lanes.iter().map(Lane::from_serialized).collect();
        Self {
            id,
            uuid: ps.uuid,
            root: ps.root.clone(),
            name: ps.name.clone().unwrap_or_default(),
            lanes,
            last_active_lane_id: ps.last_active_lane_id,
            group_id: ov.group_id,
            color: ov.color.clone(),
            // `ProjectOverride::tab_order` is persisted as `usize` for
            // schema-level forward-compat (BTree keys, JSON numbers);
            // the runtime field is `u32` because every reordering op
            // works in `0..N` range. Cast is safe: `tab_order` is
            // bounded by project count.
            tab_order: ov.tab_order as u32,
            is_collapsed: ov.is_collapsed,
        }
    }

    /// Borrow a lane by id.
    pub fn lane(&self, id: LaneId) -> Option<&Lane> {
        self.lanes.iter().find(|w| w.id == id)
    }

    /// Mutably borrow a lane by id.
    pub fn worktree_mut(&mut self, id: LaneId) -> Option<&mut Lane> {
        self.lanes.iter_mut().find(|w| w.id == id)
    }

    /// Lane the project should focus when the user clicks the
    /// project header. Falls back to the first lane when the saved
    /// hint no longer exists (deleted between sessions).
    pub fn snap_target(&self) -> Option<LaneRef> {
        let id = if self.lanes.iter().any(|w| w.id == self.last_active_lane_id) {
            self.last_active_lane_id
        } else {
            self.lanes.first()?.id
        };
        Some(LaneRef {
            project: self.id,
            lane: id,
        })
    }

    /// First lane's `LaneRef` — used when the workspace
    /// activates a project that has never been focused before.
    pub fn first_worktree_ref(&self) -> Option<LaneRef> {
        self.lanes.first().map(|w| LaneRef {
            project: self.id,
            lane: w.id,
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
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(p.last_active_lane_id, p.lanes[0].id);
        assert!(p.group_id.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snap_target_falls_back_to_first_when_hint_stale() {
        let dir = std::env::temp_dir().join("daruda_project_snap_stale");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Project::bootstrap(7, dir.clone());
        p.last_active_lane_id = 999; // not in p.lanes
        let snap = p.snap_target().unwrap();
        assert_eq!(snap.project, 7);
        assert_eq!(snap.lane, p.lanes[0].id);
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
