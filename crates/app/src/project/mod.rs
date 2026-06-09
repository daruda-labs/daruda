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
    /// Repo's actual default branch (e.g. "main"). Detected at project
    /// open (origin/HEAD → local main/master → current HEAD). `None`
    /// for non-git projects or when detection fails. Display + reconcile
    /// anchor.
    pub default_branch: Option<String>,
    /// User-overridable base ref new lanes branch from. `None` falls
    /// back to `default_branch`, then current HEAD.
    pub base_branch: Option<String>,
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
        Self::new_with_uuid(id, ProjectUuid::new(), root)
    }

    /// Build a runtime project with a caller-supplied UUID. Used by
    /// code paths that need to attach this runtime entry to an existing
    /// on-disk [`daruda_store::project::ProjectState`] (policy B: the
    /// same root may appear in multiple workspaces, but each shares a
    /// single ProjectState identified by its UUID). Lane discovery
    /// is identical to [`Project::bootstrap`].
    ///
    /// The field defaults here also serve as [`Project::bootstrap`]'s
    /// defaults — `bootstrap` delegates to this constructor. If a new
    /// field needs different initialization for the fresh-directory case,
    /// override it explicitly in `bootstrap` rather than relying on this
    /// constructor's default.
    pub fn new_with_uuid(id: ProjectId, uuid: ProjectUuid, root: PathBuf) -> Self {
        let name = derive_name_from_path(&root);
        let lanes = Lane::bootstrap_from_project(&root);
        let last_active_lane_id = lanes.first().map(|w| w.id).unwrap_or(0);
        Self {
            id,
            uuid,
            root,
            name,
            default_branch: None,
            base_branch: None,
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
    ///
    /// A project must always carry at least one lane. When the persisted
    /// list is empty — a corrupt or interrupted save — the lanes are
    /// re-discovered from the filesystem so the project self-heals
    /// rather than restoring with zero lanes (which would leave the
    /// left dock with no row to activate).
    pub fn from_disk(id: ProjectId, ps: &ProjectState, ov: &ProjectOverride) -> Self {
        // Non-empty `lanes` is an invariant every other construction
        // path (`bootstrap` / `new_with_uuid`) upholds; restore is the
        // only one reading an externally-supplied list, so it must
        // enforce the same floor. Falling back to `bootstrap_from_project`
        // re-discovers the main worktree (plus any linked ones) and the
        // next save records the recovered list, breaking the otherwise
        // self-perpetuating empty state.
        //
        // When re-bootstrapping, also reset `last_active_lane_id` to the
        // first synthesized lane: the persisted hint refers to a lane id
        // that no longer exists in the freshly-bootstrapped set (ids
        // restart at 0), so keeping it would write the stale id back to
        // disk on the next save. `snap_target()` would mask the mismatch
        // at read time, but a later session that happens to allocate the
        // same id would inherit a hint that means nothing.
        // Note: the empty-lanes re-bootstrap branch re-discovers lanes
        // from disk but does NOT re-detect `default_branch` — the
        // persisted value is read verbatim here. Task 3's reconcile pass
        // owns refreshing it against the live repo.
        let (lanes, last_active_lane_id) = if ps.lanes.is_empty() {
            let lanes = Lane::bootstrap_from_project(&ps.root);
            let head = lanes.first().map(|w| w.id).unwrap_or(0);
            (lanes, head)
        } else {
            let lanes: Vec<Lane> = ps.lanes.iter().map(Lane::from_serialized).collect();
            (lanes, ps.last_active_lane_id)
        };
        Self {
            id,
            uuid: ps.uuid,
            root: ps.root.clone(),
            name: ps.name.clone().unwrap_or_default(),
            default_branch: ps.default_branch.clone(),
            base_branch: ps.base_branch.clone(),
            lanes,
            last_active_lane_id,
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
    pub fn lane_mut(&mut self, id: LaneId) -> Option<&mut Lane> {
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
    pub fn first_lane_ref(&self) -> Option<LaneRef> {
        self.lanes.first().map(|w| LaneRef {
            project: self.id,
            lane: w.id,
        })
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
    fn from_disk_empty_lanes_rebootstraps() {
        // A persisted project with an empty lane list (corrupt /
        // interrupted save) must self-heal on restore instead of
        // yielding a lane-less project the left dock cannot render.
        let dir = std::env::temp_dir().join("daruda_project_from_disk_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ps = ProjectState {
            schema_version: 0,
            uuid: ProjectUuid::new(),
            root: dir.clone(),
            name: Some("daruda".to_string()),
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
            default_branch: None,
            base_branch: None,
        };
        let p = Project::from_disk(0, &ps, &ProjectOverride::default());
        assert!(
            !p.lanes.is_empty(),
            "empty persisted lanes must re-bootstrap"
        );
        // Non-git temp dir → exactly one `Default` lane at id 0.
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(p.lanes[0].id, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_disk_empty_lanes_resets_stale_last_active_hint() {
        // Re-bootstrapping assigns lane ids from 0, but the persisted
        // `last_active_lane_id` was captured against the (now-discarded)
        // old lane set and may point at an id the new lanes never
        // produced. Keeping that stale value would let `snap_target`'s
        // fallback mask the inconsistency while the field round-trips
        // back to disk on the next save — a later session whose ids
        // happen to grow back into that range would then activate an
        // unrelated lane on a hint that just coincidentally matched.
        // The re-bootstrap branch must reset the hint to the synthesized
        // first lane.
        let dir = std::env::temp_dir().join("daruda_project_from_disk_stale_hint");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ps = ProjectState {
            schema_version: 0,
            uuid: ProjectUuid::new(),
            root: dir.clone(),
            name: Some("daruda".to_string()),
            lanes: Vec::new(),
            last_active_lane_id: 7,
            next_lane_id: 0,
            default_branch: None,
            base_branch: None,
        };
        let p = Project::from_disk(0, &ps, &ProjectOverride::default());
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(
            p.last_active_lane_id, p.lanes[0].id,
            "stale hint must follow the synthesized lane set, not survive into the next snapshot"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_disk_nonempty_lanes_hydrated_verbatim() {
        // The healthy path must hydrate the stored lanes as-is and never
        // touch the filesystem — a re-bootstrap would renumber the lane
        // to id 0, so a surviving non-zero id proves no fallback fired.
        let dir = std::env::temp_dir().join("daruda_project_from_disk_nonempty");
        let stored = crate::lane::Lane::default_for_project(3, dir.clone()).to_serialized();
        let ps = ProjectState {
            schema_version: 0,
            uuid: ProjectUuid::new(),
            root: dir.clone(),
            name: Some("daruda".to_string()),
            lanes: vec![stored],
            last_active_lane_id: 3,
            next_lane_id: 4,
            default_branch: None,
            base_branch: None,
        };
        let p = Project::from_disk(0, &ps, &ProjectOverride::default());
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(
            p.lanes[0].id, 3,
            "stored lane hydrated verbatim, not re-bootstrapped"
        );
    }

    #[test]
    fn from_disk_reads_persisted_branch_fields() {
        // `from_disk` reads the persisted `default_branch` / `base_branch`
        // verbatim — no git call. Task 3's reconcile owns refreshing them.
        let dir = std::env::temp_dir().join("daruda_project_from_disk_branches");
        let stored = crate::lane::Lane::default_for_project(0, dir.clone()).to_serialized();
        let ps = ProjectState {
            schema_version: 0,
            uuid: ProjectUuid::new(),
            root: dir.clone(),
            name: Some("daruda".to_string()),
            lanes: vec![stored],
            last_active_lane_id: 0,
            next_lane_id: 1,
            default_branch: Some("main".to_string()),
            base_branch: Some("develop".to_string()),
        };
        let p = Project::from_disk(0, &ps, &ProjectOverride::default());
        assert_eq!(p.default_branch.as_deref(), Some("main"));
        assert_eq!(p.base_branch.as_deref(), Some("develop"));
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
}
