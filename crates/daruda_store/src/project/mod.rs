//! Project management and state persistence for daruda.
//!
//! A **project** is a root directory plus saved workspace state. This
//! crate is GPUI-free so persistence logic stays unit-testable.
//!
//! Schema-entry-point types ([`WorkspaceState`], [`ProjectState`]) and
//! per-file persistence live in [`types`] and [`persistence`]. Low-level
//! building blocks ([`SerializedLane`], [`SerializedGroup`],
//! [`SerializedProject`], [`LaneRef`], dock / window state, etc.)
//! stay at this level so views and other consumers can import them
//! without pulling in the persistence layer.

pub mod lane;
pub mod persistence;
pub mod types;

#[cfg(test)]
mod tests;

pub use lane::{LaneId, LaneKind, LaneStatus, LeftDockView, RightDockView, SerializedLane};
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
/// points at exactly one (project, lane) pair; an invalid pair is
/// repaired by the runtime restore path
/// (`Workspace::restore_from_disk::resolve_active`) at load time so
/// downstream code never has to handle dangling refs.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneRef {
    pub project: ProjectId,
    #[serde(rename = "worktree", alias = "lane")]
    pub lane: LaneId,
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
/// snapshot. Each project owns its own lanes and tracks which
/// lane was last active so clicking the project header in the
/// left dock can snap to that lane.
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
    #[serde(default, rename = "worktrees", alias = "lanes")]
    pub lanes: Vec<SerializedLane>,
    /// Last lane the user activated inside this project. Used as a
    /// snap hint when the project becomes active without a specific
    /// lane pick.
    #[serde(
        default,
        rename = "last_active_worktree_id",
        alias = "last_active_lane_id"
    )]
    pub last_active_lane_id: LaneId,
    /// True when the project header is rendered in the left dock with
    /// its lane list hidden. Click on the chevron toggles.
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
        /// Agent-chat state for `PaneContent::AgentChat` leaves. `None`
        /// (the default and the format used before AgentChat panes
        /// existed) means the leaf restores as a Terminal / File pane;
        /// `Some` means the leaf restores as a `PaneContent::AgentChat`.
        /// Mutually exclusive with `file`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_chat: Option<SerializedAgentChatContent>,
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
/// state and re-derives when the lane's git status refreshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedFileContent {
    #[serde(rename = "worktree_id", alias = "lane_id")]
    pub lane_id: LaneId,
    pub path: PathBuf,
    #[serde(default)]
    pub staged: bool,
    pub view_mode: SerializedFileViewMode,
}

/// Persisted state for a `PaneContent::AgentChat` leaf. The lane working
/// directory anchors the pane to the right lane on the next launch; the
/// ACP `session_id` (when present) lets the pane resume the prior
/// conversation via `session/load` on first focus rather than starting a
/// fresh session. The conversation itself is not stored — the adapter
/// replays it from the resumed session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedAgentChatContent {
    /// Lane working directory the agent session is rooted at. `None`
    /// when the pane was opened without a resolvable lane cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Persisted ACP session id. `Some` once a live session has been
    /// established; on the next launch the pane resumes it via
    /// `session/load` (replaying the prior conversation) instead of
    /// starting a fresh session. `None` for a pane that never connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Agent-provided session title, cached so a restored dormant pane
    /// shows its label before the session loads. `None` = fallback label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
