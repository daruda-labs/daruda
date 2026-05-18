//! Project management and state persistence for daruda.
//!
//! A **project** is a root directory plus saved workspace state. This
//! crate is GPUI-free so persistence logic stays unit-testable.
//!
//! Two on-disk shapes coexist during the multi-project rollout:
//!
//! - [`ProjectState`] — the legacy flat shape (single project per file).
//!   Still used by the app crate's runtime; persistence converts to/from
//!   it transparently.
//! - [`WorkspaceState`] — the new shape with `projects: Vec<SerializedProject>`,
//!   `groups: Vec<SerializedGroup>`, and a [`WorktreeRef`]-based active
//!   pointer. Written to disk going forward; legacy files are migrated
//!   on load.

pub mod persistence;
pub mod worktree;

#[cfg(test)]
mod tests;

pub use worktree::{
    LeftDockView, RightDockView, SerializedWorktree, UsageWindow, WorktreeId, WorktreeKind,
    WorktreeStatus,
};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stable identifier for a project within a workspace. Monotonic per
/// workspace — deleted IDs are never reused so stale references fail
/// the [`WorkspaceState::normalize_active`] lookup instead of silently
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
/// and [`WorkspaceState::from_legacy`] so both paths produce identical names.
pub fn derive_name_from_path(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("untitled")
        .to_string()
}

// ============================================================================
// Serializable workspace state
// ============================================================================

/// Everything needed to restore a workspace session.
///
/// Tabs live **inside** worktrees (new model). The top-level `tabs` /
/// `active_tab_index` fields are retained for backward compatibility
/// with files written before worktrees existed — `migrate_legacy()`
/// folds them into a single `Default` worktree and `save_state` skips
/// writing them when empty.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectState {
    pub root: PathBuf,

    /// Worktrees in user-visible order. Non-empty after migration.
    #[serde(default)]
    pub worktrees: Vec<SerializedWorktree>,

    /// ID of the worktree whose tab bar / panes are currently visible.
    #[serde(default)]
    pub active_worktree_id: WorktreeId,

    /// Which left-dock view (Worktrees / Git / Files) was last shown.
    #[serde(default)]
    pub active_dock_view: LeftDockView,

    /// Which right-panel tab (Usage / Skills / Tools / Tasks) was last
    /// shown. Backwards compatible — older state files without the
    /// field decode as `RightDockView::default()` (= Usage).
    #[serde(default)]
    pub active_right_panel_view: RightDockView,

    /// Time window applied to the Usage tab. `#[serde(default)]`
    /// makes pre-filter state files load as `UsageWindow::default()`
    /// (= Last7d), which is the most useful value for users coming
    /// from the lifetime-cumulative behaviour without losing data.
    #[serde(default)]
    pub active_usage_window: UsageWindow,

    /// Legacy: top-level tabs. Migrated into a single `Default`
    /// worktree by `migrate_legacy`. Omitted from new JSON once empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<SerializedTab>,

    /// Legacy counterpart to `tabs` — see above.
    #[serde(default)]
    pub active_tab_index: usize,

    pub focused_pane_id: u64,
    pub docks: DockStates,
    pub window: WindowState,
    /// User-set window title (Window > Edit Window Title…). When
    /// present, the in-memory `Workspace::window_user_label` carries
    /// this value and overrides the auto-derived `<pane title> — <cwd>`
    /// at the next render. Old state files without the field decode
    /// as `None` via `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_user_label: Option<String>,
    pub font_size: f32,
    pub vertical_spacing: f32,
    pub horizontal_spacing: f32,
}

impl ProjectState {
    /// Fold legacy top-level `tabs` into a single `Default` worktree
    /// when the new `worktrees` field is empty. Idempotent — no-op
    /// once worktrees are populated. Callers should run this right
    /// after deserializing so downstream code only sees the new shape.
    ///
    /// Half-written save files (e.g. a crash after `worktrees` was
    /// written but before the legacy `tabs` field was cleared) take
    /// the new-shape branch: `worktrees` is non-empty, so `tabs` is
    /// silently ignored. That means at most one session of stray
    /// tabs is lost — preferable to duplicating them into a second
    /// default worktree or failing to deserialize entirely.
    pub fn migrate_legacy(&mut self) {
        if !self.worktrees.is_empty() {
            return;
        }
        if self.tabs.is_empty() {
            // Nothing to migrate — leave worktrees empty so the caller
            // can bootstrap a fresh default worktree at runtime.
            return;
        }
        let legacy_tabs = std::mem::take(&mut self.tabs);
        let legacy_active = std::mem::take(&mut self.active_tab_index);
        let worktree = SerializedWorktree {
            id: 0,
            kind: WorktreeKind::Default,
            path: self.root.clone(),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: legacy_tabs,
            active_tab_index: legacy_active,
            base_ref: None,
            description: None,
        };
        self.active_worktree_id = worktree.id;
        self.worktrees.push(worktree);
    }
}

// ============================================================================
// Multi-project / Group shape (forward-only — written to disk going forward;
// legacy `ProjectState` files migrate on load via `WorkspaceState::from_legacy`).
// ============================================================================

/// Active-tab pointer in the multi-project model. A workspace always
/// points at exactly one (project, worktree) pair; an invalid pair is
/// repaired by [`WorkspaceState::normalize_active`] at load time so
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// A project entry inside [`WorkspaceState`]. Each project owns its own
/// worktrees and tracks which worktree was last active so clicking the
/// project header in the left dock can snap to that worktree.
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
}

/// Current on-disk schema version for [`WorkspaceState`]. Bumped
/// whenever the persisted shape changes in a way callers need to
/// detect (added projects/groups in v2). Legacy [`ProjectState`] files
/// have no `schema_version` field and are detected by its absence.
pub const WORKSPACE_SCHEMA_VERSION: u32 = 2;

/// Workspace-level persisted state — a list of projects + optional
/// groups + workspace chrome. Replaces the flat per-project [`ProjectState`]
/// as the on-disk shape going forward.
///
/// `migrate_legacy` and the persistence layer translate older JSON
/// (top-level `tabs` only, or `worktrees` without project framing) into
/// this shape transparently, so callers always see a normalized struct.
///
/// Always serialized with an explicit `schema_version` so the loader
/// can distinguish a deliberately-empty new-shape file from a legacy
/// one without resorting to "is the projects array empty" heuristics.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Discriminator for legacy vs. new on-disk shape — see
    /// [`WORKSPACE_SCHEMA_VERSION`]. Always serialized; absent in
    /// legacy files (decodes as `0` via `#[serde(default)]`).
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub projects: Vec<SerializedProject>,
    #[serde(default)]
    pub groups: Vec<SerializedGroup>,
    #[serde(default)]
    pub active: WorktreeRef,
    /// Monotonic counter for the next [`ProjectId`] to mint. Persisted
    /// so deletions never recycle IDs across sessions — stale refs from
    /// other state (recents, error reports, ...) stay distinguishable.
    #[serde(default)]
    pub next_project_id: ProjectId,
    /// Same monotonic rule as `next_project_id`, for groups.
    #[serde(default)]
    pub next_group_id: GroupId,
    #[serde(default)]
    pub window_open_policy: WindowOpenPolicy,

    // -- Workspace chrome — shared across projects within this window.
    #[serde(default)]
    pub focused_pane_id: u64,
    #[serde(default)]
    pub active_dock_view: LeftDockView,
    #[serde(default)]
    pub active_right_panel_view: RightDockView,
    #[serde(default)]
    pub active_usage_window: UsageWindow,
    #[serde(default)]
    pub docks: DockStates,
    #[serde(default)]
    pub window: WindowState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_user_label: Option<String>,
    #[serde(default)]
    pub font_size: f32,
    #[serde(default)]
    pub vertical_spacing: f32,
    #[serde(default)]
    pub horizontal_spacing: f32,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            projects: Vec::new(),
            groups: Vec::new(),
            active: WorktreeRef::default(),
            next_project_id: 0,
            next_group_id: 0,
            window_open_policy: WindowOpenPolicy::default(),
            focused_pane_id: 0,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            active_usage_window: UsageWindow::default(),
            docks: DockStates::default(),
            window: WindowState::default(),
            window_user_label: None,
            font_size: 0.0,
            vertical_spacing: 0.0,
            horizontal_spacing: 0.0,
        }
    }
}

impl WorkspaceState {
    /// Convert a legacy [`ProjectState`] into the new shape. Runs the
    /// caller's `migrate_legacy` so any pre-worktree `tabs` are folded
    /// into a default worktree first, then wraps the result in a single
    /// `SerializedProject` (id `0`, derived name).
    pub fn from_legacy(mut legacy: ProjectState) -> Self {
        legacy.migrate_legacy();
        let project_id: ProjectId = 0;
        let active_worktree = legacy.active_worktree_id;
        let project = SerializedProject {
            id: project_id,
            root: legacy.root.clone(),
            name: derive_name_from_path(&legacy.root),
            color: None,
            tab_order: 0,
            group_id: None,
            last_active_worktree_id: active_worktree,
            worktrees: legacy.worktrees,
        };
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            projects: vec![project],
            groups: Vec::new(),
            active: WorktreeRef {
                project: project_id,
                worktree: active_worktree,
            },
            next_project_id: 1,
            next_group_id: 0,
            window_open_policy: WindowOpenPolicy::default(),
            focused_pane_id: legacy.focused_pane_id,
            active_dock_view: legacy.active_dock_view,
            active_right_panel_view: legacy.active_right_panel_view,
            active_usage_window: legacy.active_usage_window,
            docks: legacy.docks,
            window: legacy.window,
            window_user_label: legacy.window_user_label,
            font_size: legacy.font_size,
            vertical_spacing: legacy.vertical_spacing,
            horizontal_spacing: legacy.horizontal_spacing,
        }
    }

    /// Collapse to the legacy single-project shape — used by the
    /// persistence layer to keep the app crate's runtime API stable
    /// during the multi-project rollout. Picks the primary project
    /// (first by `tab_order`, then by index) and projects its worktrees
    /// onto the flat fields.
    ///
    /// An empty workspace produces a fully-default `ProjectState` so the
    /// caller can detect "no project" via `state.root.as_os_str().is_empty()`
    /// or `state.worktrees.is_empty()`.
    pub fn into_primary_project_state(mut self) -> ProjectState {
        self.normalize_active();
        let primary = self
            .projects
            .iter()
            .position(|p| p.id == self.active.project)
            .or_else(|| (!self.projects.is_empty()).then_some(0));
        let (root, worktrees, active_worktree_id) = match primary {
            Some(idx) => {
                let p = self.projects.swap_remove(idx);
                (p.root, p.worktrees, self.active.worktree)
            }
            None => (PathBuf::new(), Vec::new(), 0),
        };
        ProjectState {
            root,
            worktrees,
            active_worktree_id,
            active_dock_view: self.active_dock_view,
            active_right_panel_view: self.active_right_panel_view,
            active_usage_window: self.active_usage_window,
            tabs: Vec::new(),
            active_tab_index: 0,
            focused_pane_id: self.focused_pane_id,
            docks: self.docks,
            window: self.window,
            window_user_label: self.window_user_label,
            font_size: self.font_size,
            vertical_spacing: self.vertical_spacing,
            horizontal_spacing: self.horizontal_spacing,
        }
    }

    /// Idempotent post-load normalization:
    /// 1. delegates to each worktree's `migrate_legacy_tabs` (no-op today
    ///    — placeholder for future per-worktree shape changes),
    /// 2. fixes invalid `active` references via [`normalize_active`](Self::normalize_active),
    /// 3. ratchets monotonic counters past the max observed ID so
    ///    subsequent inserts never collide with surviving entries.
    pub fn migrate_legacy(&mut self) {
        self.normalize_active();
        self.ensure_counters_advance();
    }

    /// Repair `active` when it points at a project or worktree that
    /// doesn't exist anymore (deleted between sessions, hand-edited
    /// state file, etc.). Order:
    /// 1. Missing project → fall back to `projects[0]`.
    /// 2. Missing worktree inside the chosen project → fall back to
    ///    the project's `last_active_worktree_id`, then `worktrees[0]`.
    /// 3. Chosen project has no worktrees at all → reset
    ///    `active.worktree` to `0` so the stale id cannot leak into
    ///    runtime via [`into_primary_project_state`](Self::into_primary_project_state).
    /// 4. No projects at all → leave `active` at its default (caller
    ///    routes to the Welcome screen).
    pub fn normalize_active(&mut self) {
        if self.projects.is_empty() {
            self.active = WorktreeRef::default();
            return;
        }
        if !self.projects.iter().any(|p| p.id == self.active.project) {
            self.active.project = self.projects[0].id;
        }
        let Some(project) = self.projects.iter().find(|p| p.id == self.active.project) else {
            return;
        };
        if project
            .worktrees
            .iter()
            .any(|w| w.id == self.active.worktree)
        {
            return;
        }
        if project
            .worktrees
            .iter()
            .any(|w| w.id == project.last_active_worktree_id)
        {
            self.active.worktree = project.last_active_worktree_id;
        } else if let Some(first) = project.worktrees.first() {
            self.active.worktree = first.id;
        } else {
            // Project exists but has no worktrees — a stale `active.worktree`
            // here would survive into the legacy adapter and surface as a
            // wrong worktree id at the next restore. Reset to 0 so the
            // adapter routes to a fresh default worktree instead.
            self.active.worktree = 0;
        }
    }

    /// Bump monotonic counters past the largest observed ID. Keeps
    /// `next_*_id` strictly greater than any existing entry so inserts
    /// never collide with surviving state.
    pub fn ensure_counters_advance(&mut self) {
        let max_project = self.projects.iter().map(|p| p.id).max();
        if let Some(m) = max_project
            && self.next_project_id <= m
        {
            self.next_project_id = m + 1;
        }
        let max_group = self.groups.iter().map(|g| g.id).max();
        if let Some(m) = max_group
            && self.next_group_id <= m
        {
            self.next_group_id = m + 1;
        }
    }

    /// Reference to the workspace's primary project (the project the
    /// `active` ref points at, with `projects[0]` as a fallback). `None`
    /// when the workspace has no projects — caller renders Welcome.
    pub fn primary_project(&self) -> Option<&SerializedProject> {
        self.projects
            .iter()
            .find(|p| p.id == self.active.project)
            .or_else(|| self.projects.first())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DockStates {
    pub left_open: bool,
    pub left_size: f32,
    pub bottom_open: bool,
    pub bottom_size: f32,
    pub right_open: bool,
    pub right_size: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

// ============================================================================
// Recent projects
// ============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentEntry {
    pub root: PathBuf,
    pub name: String,
    pub last_opened: u64,
}

impl RecentEntry {
    /// Create a new entry with the current timestamp.
    pub fn now(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();
        let last_opened = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            root,
            name,
            last_opened,
        }
    }
}

/// Maximum entries in the recent projects list.
pub const RECENT_MAX: usize = 20;

/// Compute a filesystem-safe hash for a project root path.
/// Uses FNV-1a constants for cross-version stability (std's
/// DefaultHasher is not guaranteed stable across Rust releases).
pub fn path_hash(root: &Path) -> String {
    let bytes = root.to_string_lossy();
    let mut hash: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in bytes.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{hash:016x}")
}
