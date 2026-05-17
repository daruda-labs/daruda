//! Project management and state persistence for daruda.
//!
//! A **project** is a root directory plus saved workspace state. This
//! crate is GPUI-free so persistence logic stays unit-testable.

pub mod persistence;
pub mod worktree;

#[cfg(test)]
mod tests;

pub use worktree::{
    LeftSidebarView, RightSidebarView, SerializedWorktree, UsageWindow, WorktreeId, WorktreeKind,
    WorktreeStatus,
};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string();
        Self { root, name }
    }
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

    /// Which sidebar view (Worktrees / Git / Files) was last shown.
    #[serde(default)]
    pub active_sidebar_view: LeftSidebarView,

    /// Which right-panel tab (Usage / Skills / Tools / Tasks) was last
    /// shown. Backwards compatible — older state files without the
    /// field decode as `RightSidebarView::default()` (= Usage).
    #[serde(default)]
    pub active_right_panel_view: RightSidebarView,

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
