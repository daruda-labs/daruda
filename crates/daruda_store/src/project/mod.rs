//! Project management and state persistence for daruda.
//!
//! A **project** is a root directory plus saved workspace state. This
//! crate is GPUI-free so persistence logic stays unit-testable.
//!
//! Schema-entry-point types ([`WorkspaceState`], [`ProjectState`]) and
//! per-file persistence live in [`types`] and [`persistence`]. Low-level
//! building blocks ([`SerializedWorktree`], [`SerializedGroup`],
//! [`SerializedProject`], [`WorktreeRef`], dock / window state, etc.)
//! stay at this level so views and other consumers can import them
//! without pulling in the persistence layer.

pub mod persistence;
pub mod types;
pub mod worktree;

#[cfg(test)]
mod tests;

pub use persistence::{
    RECENT_MAX, delete_project_state_in, delete_workspace_state_in, for_each_project_state_in,
    for_each_workspace_state_in, is_uuid_filename_stem, load_project_state_in, load_recent_in,
    load_workspace_state_in, projects_dir_in, recent_path_in, save_project_state_in,
    save_recent_in, save_workspace_state_in, touch_recent_in, workspaces_dir_in,
};
pub use types::{
    PaneId, PaneLayout, ProjectOverride, ProjectState, ProjectUuid, RecentEntry,
    WORKSPACE_SCHEMA_VERSION, WorkspaceState, WorkspaceUuid,
};
pub use worktree::{
    LeftDockView, RightDockView, SerializedWorktree, UsageWindow, WorktreeId, WorktreeKind,
    WorktreeStatus,
};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stable identifier for a project within a workspace. Monotonic per
/// workspace — deleted IDs are never reused so stale references fail
/// the active-pointer fallback ladder (in
/// `Workspace::restore_from_disk::resolve_active`) instead of silently
/// targeting a different project.
pub type ProjectId = u64;

/// Stable identifier for a group within a workspace. Same monotonic
/// rule as [`ProjectId`].
pub type GroupId = u64;

/// A project = a root directory.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
}

impl Project {
    /// Create a project from a directory path. Name = last path component.
    pub fn from_path(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let name = derive_name_from_path(&root);
        Self { root, name }
    }
}

/// Compute a display name from a filesystem path — last path component,
/// or `"untitled"` for root / empty paths. Shared by [`Project::from_path`]
/// and the on-disk-state hydration path so all sources produce identical
/// names.
pub fn derive_name_from_path(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("untitled")
        .to_string()
}

// ============================================================================
// Multi-project / Group shape (the on-disk schema).
// ============================================================================

/// Active-tab pointer in the multi-project model. A workspace always
/// points at exactly one (project, worktree) pair; an invalid pair is
/// repaired by the runtime restore path
/// (`Workspace::restore_from_disk::resolve_active`) at load time so
/// downstream code never has to handle dangling refs.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRef {
    pub project: ProjectId,
    pub worktree: WorktreeId,
}

/// User policy for the "Open Project…" affordance. Persists across
/// launches so the user can opt out of the modal by ticking "Don't
/// ask again" once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowOpenPolicy {
    /// Show the chooser modal (default).
    #[default]
    Ask,
    /// Always add the new project to the current window.
    AddHere,
    /// Always open the new project in a fresh window.
    NewWindow,
}

/// User-defined group of projects in the left dock. Groups carry only
/// visual metadata (name, optional color, collapsed state, tab order);
/// projects reference their group via [`SerializedProject::group_id`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedGroup {
    pub id: GroupId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub tab_order: u32,
    #[serde(default)]
    pub is_collapsed: bool,
}

/// Serializable per-project payload bundled inside a workspace
/// snapshot. Each project owns its own worktrees and tracks which
/// worktree was last active so clicking the project header in the
/// left dock can snap to that worktree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializedProject {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub tab_order: u32,
    /// `None` = ungrouped (rendered at top level alongside groups in
    /// the same `tab_order` pool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(default)]
    pub worktrees: Vec<SerializedWorktree>,
    /// Last worktree the user activated inside this project. Used as a
    /// snap hint when the project becomes active without a specific
    /// worktree pick.
    #[serde(default)]
    pub last_active_worktree_id: WorktreeId,
    /// True when the project header is rendered in the left dock with
    /// its worktree list hidden. Click on the chevron toggles.
    /// `#[serde(default)]` so older state files load as expanded.
    #[serde(default)]
    pub is_collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedTab {
    pub layout: SerializedLayout,
    pub last_focused_pane: u64,
    /// User-set tab title (Window > Edit Tab Title…). When present
    /// it overrides the auto-derived title (cwd basename / PTY title)
    /// in the tab strip. Old state files without the field decode as
    /// `None` via `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SerializedLayout {
    Leaf {
        pane_id: u64,
        /// Terminal cwd (last value reported via OSC 7). Ignored when
        /// `file` is `Some` — a File pane derives its cwd from
        /// `path.parent()` at runtime.
        cwd: Option<PathBuf>,
        /// File-viewer state for File panes. `None` (the default and
        /// the format used pre-Plan-B) means the leaf is a Terminal
        /// pane; `Some` means the leaf restores as a `PaneContent::File`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<SerializedFileContent>,
    },
    Split {
        direction: SplitDirectionSerde,
        children: Vec<SerializedLayout>,
        ratios: Vec<f32>,
    },
}

/// Persisted state for a `PaneContent::File` leaf — enough to
/// reconstruct the file viewer on the next launch. `file_status`
/// (the git badge) is intentionally omitted: it depends on live git
/// state and re-derives when the worktree's git status refreshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedFileContent {
    pub worktree_id: WorktreeId,
    pub path: PathBuf,
    #[serde(default)]
    pub staged: bool,
    pub view_mode: SerializedFileViewMode,
}

/// Serializable mirror of `daruda::workspace::pane_file_view::FileViewMode`.
/// Lives in `daruda_project` so the persistence layer stays free of
/// app-side types; the conversion is one-to-one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedFileViewMode {
    Raw,
    Preview,
    Changes,
}

/// Serializable split direction — validated on deserialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirectionSerde {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DockStates {
    pub left_open: bool,
    pub left_size: f32,
    pub bottom_open: bool,
    pub bottom_size: f32,
    pub right_open: bool,
    pub right_size: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowState {
    /// True when the state has usable values (not all-zero default).
    pub fn is_valid(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}
