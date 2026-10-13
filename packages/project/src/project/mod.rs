//! Runtime [`Project`] — the workspace-visible counterpart to the
//! persisted [`daruda_store::project::ProjectState`] plus the
//! per-workspace [`daruda_store::project::ProjectOverride`].
//!
//! A project owns a root directory, a non-empty list of runtime
//! [`crate::lane::Lane`]s, a "last active lane" hint for
//! the left dock, and visual metadata (color, tab order, group).
//!
//! GPUI-free: lives alongside [`crate::lane`] in dependency order
//! `project → lane`. The app's `Workspace` assembles a `Vec<Project>` at
//! construction time and owns its persistence (`snapshot_for_disk` /
//! `restore_from_disk`).

use std::path::PathBuf;

use daruda_store::project::{
    GroupId, LaneId, LaneRef, ProjectId, ProjectOverride, ProjectState, ProjectUuid,
    derive_name_from_path,
};

use crate::lane::Lane;
use crate::lane::availability::LaneAvailability;

mod projects;
pub use projects::Projects;

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
    /// Runtime-only. Whether this project's root directory can be read
    /// on disk. Recomputed from the live filesystem on restore /
    /// activate; never serialized. Defaults to `Present`.
    pub availability: LaneAvailability,
}

impl Project {
    /// The branch this project's lanes start from and are compared against:
    /// the user's chosen base, else the detected default. One answer, so a
    /// new lane branches from the base its against-base view later reads.
    pub fn effective_base_branch(&self) -> Option<&str> {
        self.base_branch
            .as_deref()
            .or(self.default_branch.as_deref())
    }

    /// Build a runtime project from a freshly opened directory. Walks
    /// the filesystem to discover git worktrees (or falls back to a
    /// single `Default` lane for non-git paths) via
    /// [`Lane::bootstrap_from_project`]. The default `id` is `0` —
    /// `Workspace::add_project` overwrites it with the monotonic id.
    pub fn bootstrap(id: ProjectId, root: PathBuf) -> Self {
        Self::new_with_uuid(id, ProjectUuid::new(), root)
    }

    /// Build a runtime project without touching git — a single
    /// `Default` placeholder lane that
    /// `Workspace::reconcile_bootstrapped_lanes` upgrades to the
    /// git-discovered list asynchronously after window creation.
    /// Keeps the window-construction path free of blocking git CLI
    /// calls ([`Lane::bootstrap_from_project`] spawns up to four).
    pub fn bootstrap_placeholder(id: ProjectId, root: PathBuf) -> Self {
        let lanes = vec![Lane::default_for_project(0, root.clone())];
        Self::from_parts(id, ProjectUuid::new(), root, lanes)
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
        let lanes = Lane::bootstrap_from_project(&root);
        Self::from_parts(id, uuid, root, lanes)
    }

    /// Shared constructor tail — the field defaults every fresh-project
    /// path uses. Callers differ only in how `lanes` was produced
    /// (git discovery vs. pure placeholder).
    fn from_parts(id: ProjectId, uuid: ProjectUuid, root: PathBuf, lanes: Vec<Lane>) -> Self {
        let name = derive_name_from_path(&root);
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
            availability: LaneAvailability::Present,
        }
    }

    /// Hydrate a runtime project from the UUID-keyed on-disk shape —
    /// a [`ProjectState`] (intrinsic per-project fields) plus the
    /// per-workspace [`ProjectOverride`] (cosmetic decoration). Used
    /// by the app's `Workspace::restore_from_disk`.
    ///
    /// A project must always carry at least one lane. When the persisted
    /// list is empty — a corrupt or interrupted save — the lanes are
    /// re-discovered from the filesystem so the project self-heals
    /// rather than restoring with zero lanes (which would leave the
    /// left dock with no row to activate).
    pub fn from_disk(id: ProjectId, ps: &ProjectState, ov: &ProjectOverride) -> Self {
        // Non-empty `lanes` is an invariant every other construction path
        // upholds; restore is the only one reading an externally-supplied
        // list, so an empty one triggers `bootstrap_from_project` to
        // re-discover worktrees from disk (the next save persists the
        // recovered list). `last_active_lane_id` is reset to the first
        // synthesized lane in that case, since the persisted hint refers to
        // an id that no longer exists once ids restart at 0.
        //
        // The re-bootstrap branch re-discovers lanes but does not re-detect
        // `default_branch` — that value is read verbatim; refreshing it
        // against the live repo is a separate reconcile pass.
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
            // Recomputed from disk by the workspace restore path.
            availability: LaneAvailability::Present,
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
