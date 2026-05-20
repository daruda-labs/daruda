//! UUID-keyed workspace/project persistence types — the canonical
//! on-disk schema. See `crate::project::persistence` for the disk
//! layout.
//!
//! Pane-layout types referenced below reuse the existing low-level
//! building blocks at `crate::project::*`:
//! - `PaneId` aliases the `u64` already used for pane identifiers
//!   (`focused_pane_id`, `SerializedTab::last_focused_pane`).
//! - `PaneLayout` aliases [`SerializedTab`], the per-tab structure that
//!   wraps a [`SerializedLayout`] plus per-tab chrome.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::project::{
    DockStates, GroupId, LeftDockView, RightDockView, SerializedGroup, SerializedTab,
    SerializedWorktree, UsageWindow, WindowOpenPolicy, WindowState, WorktreeId,
};

pub const WORKSPACE_SCHEMA_VERSION: u32 = 3;

/// Stable pane identifier alias. Matches the `u64` used by
/// [`SerializedTab::last_focused_pane`] and `focused_pane_id` so all
/// pane references share a single representation.
pub type PaneId = u64;

/// Tab-shaped per-pane layout alias. The new workspace shape keeps
/// tabs per-project (see [`WorkspaceState::project_tabs`]) and reuses
/// the existing [`SerializedTab`] without redefining it.
pub type PaneLayout = SerializedTab;

/// Identifies one project across the lifetime of a daruda install.
/// Distinct from runtime `ProjectId` (which is per-workspace and not
/// stable across sessions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectUuid(pub Uuid);

impl ProjectUuid {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn as_inner(&self) -> &Uuid {
        &self.0
    }
}

impl Default for ProjectUuid {
    /// Returns the nil UUID sentinel (`00000000-...`). Use this when
    /// you need a placeholder; call `ProjectUuid::new()` to mint a
    /// real one. Returning a fresh UUID here would be a footgun for
    /// `#[serde(default)]` paths.
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

/// Identifies one workspace (one window's persisted state).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceUuid(pub Uuid);

impl WorkspaceUuid {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn as_inner(&self) -> &Uuid {
        &self.0
    }
}

impl Default for WorkspaceUuid {
    /// Returns the nil UUID sentinel. Use `WorkspaceUuid::new()` to
    /// mint a real one. Returning a fresh UUID here would be a
    /// footgun for `#[serde(default)]` paths.
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

/// One project, identified by UUID. Stored at
/// `projects/<uuid>.json`. Owns the parts that are *intrinsic* to a
/// project regardless of which workspaces reference it: root path,
/// human-visible name, the worktree list, and the last-active
/// worktree pointer. Per-workspace decoration (group/color/order)
/// lives in `WorkspaceState::project_overrides`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectState {
    pub schema_version: u32,
    pub uuid: ProjectUuid,
    pub root: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub worktrees: Vec<SerializedWorktree>,
    #[serde(default)]
    pub last_active_worktree_id: WorktreeId,
    #[serde(default)]
    pub next_worktree_id: WorktreeId,
}

/// Per-workspace decoration on a project reference. Color/tab_order/
/// group_id are workspace-local (policy B): the same project can be
/// grouped one way in workspace A and another in workspace B.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ProjectOverride {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tab_order: usize,
    #[serde(default)]
    pub group_id: Option<GroupId>,
    #[serde(default)]
    pub is_collapsed: bool,
}

/// One window's persisted state. Stored at
/// `workspaces/<uuid>.json`. References projects by UUID; the
/// dereferenced ProjectStates live under `projects/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub schema_version: u32,
    pub uuid: WorkspaceUuid,

    pub project_ids: Vec<ProjectUuid>,
    #[serde(default)]
    pub project_overrides: BTreeMap<ProjectUuid, ProjectOverride>,
    #[serde(default)]
    pub groups: Vec<SerializedGroup>,

    pub active_project: Option<ProjectUuid>,
    pub active_worktree: Option<WorktreeId>,

    pub docks: DockStates,
    pub window: WindowState,
    pub font_size: f32,
    pub vertical_spacing: f32,
    pub horizontal_spacing: f32,

    pub focused_pane_id: PaneId,
    pub active_dock_view: LeftDockView,
    pub active_right_panel_view: RightDockView,
    pub active_usage_window: UsageWindow,
    pub window_open_policy: WindowOpenPolicy,

    #[serde(default)]
    pub next_group_id: GroupId,

    /// Per-workspace tabs/layout for each referenced project. Keyed
    /// by ProjectUuid so reordering / removal doesn't shift other
    /// projects' tabs.
    ///
    /// Write-only forward-compat envelope today. The canonical read
    /// source for tabs is `ProjectState::worktrees[i].tabs` — this
    /// field is populated by `Workspace::snapshot_for_disk` but not
    /// yet consumed by restore. Once a future task introduces
    /// workspace-level tab reordering that needs to override the
    /// per-worktree default, this field will become canonical and
    /// `SerializedWorktree::tabs` will become its initialization seed.
    pub project_tabs: BTreeMap<ProjectUuid, Vec<PaneLayout>>,
}

/// Entry in `recent-workspaces.json`. Keyed by workspace UUID;
/// reopening a recent entry directly restores the full multi-project
/// window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub workspace_uuid: WorkspaceUuid,
    pub display_name: String,
    pub last_opened: i64,
}

impl RecentEntry {
    pub fn now(workspace_uuid: WorkspaceUuid, display_name: String) -> Self {
        let last_opened = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self {
            workspace_uuid,
            display_name,
            last_opened,
        }
    }
}
