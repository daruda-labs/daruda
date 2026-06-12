//! Lane — a git worktree (or a default stand-in for non-git
//! folders) plus its persisted tab/pane layout and metadata.
//!
//! GPUI-free: only persistable fields live here. The runtime form
//! (with live `TerminalView` entities, PTY handles, agent tasks, etc.)
//! is assembled in the `app` crate and wraps this struct as
//! `lane.serialized`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::SerializedTab;

/// Stable identifier for a lane within a project.
pub type LaneId = u64;

/// Which view the left dock is currently showing. Persisted so the app
/// restores the user's last-used view on restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftDockView {
    #[default]
    #[serde(rename = "worktrees", alias = "lanes")]
    Lanes,
    GitChanges,
    Files,
}

/// Which view the right dock is currently showing. Persisted so the
/// app restores the user's last-used right-panel tab on restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightDockView {
    #[default]
    Usage,
    Skills,
    Tools,
    Tasks,
}

/// Distinguishes git worktrees from the fallback used when daruda
/// opens a plain directory. `Default` is reserved for non-git paths
/// and must be unique per project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaneKind {
    Git {
        /// `None` = detached HEAD.
        branch: Option<String>,
        /// Absolute path to the **shared** main repository top — the
        /// directory holding the common `.git/` for the whole repo.
        /// Same value across every lane of the repo. Use this as
        /// cwd for lane-management ops (`git worktree add/remove`,
        /// `git branch -D`) where git resolves the shared gitdir from
        /// any path inside the repo. Do **not** use it as cwd for
        /// per-lane ops (`git add`, `git restore`) — those need
        /// [`worktree_root`].
        repo_root: PathBuf,
        /// Absolute path to **this** lane's filesystem toplevel —
        /// the directory holding the per-lane `.git` entry
        /// (regular `.git/` for the main lane, a `.git` pointer
        /// file for linked lanes). Per-lane, immutable after
        /// the initial probe. Use this as cwd for every git CLI that
        /// targets a specific lane's index: `git status`,
        /// `git add`, `git restore --staged`, `git diff`. Porcelain
        /// paths returned by `git status` are relative to this path,
        /// so subsequent `git add <path>` resolves correctly only
        /// when cwd matches.
        ///
        /// Distinct from `Lane.path` because the latter is
        /// anchored to a sub-directory when the user opens one
        /// (terminal-cwd convenience); `worktree_root` always stays
        /// at the actual git toplevel.
        #[serde(default)]
        worktree_root: PathBuf,
    },
    Default,
}

impl LaneKind {
    /// `true` when this lane is backed by git (any branch state).
    pub fn is_git(&self) -> bool {
        matches!(self, Self::Git { .. })
    }
}

/// Runtime-only status indicator. Not persisted — always `Idle` after
/// restore; the runtime recomputes `Running` / `Error` from live PTY
/// and agent signals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneStatus {
    #[default]
    Idle,
    Running,
    Error,
}

/// On-disk representation of a lane. Each lane owns its own
/// tab list and active-tab index; a `ProjectState` is a collection of
/// these lanes plus shared workspace chrome state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedLane {
    pub id: LaneId,
    pub kind: LaneKind,
    pub path: PathBuf,
    #[serde(default, alias = "label")]
    pub name: Option<String>,
    #[serde(default)]
    pub tab_order: u32,
    #[serde(default)]
    pub is_unread: bool,
    /// Unix timestamp (seconds). `0` = unknown / never.
    #[serde(default)]
    pub last_activity: u64,
    #[serde(default)]
    pub tabs: Vec<SerializedTab>,
    #[serde(default)]
    pub active_tab_index: usize,
    /// Ref the lane was branched from (e.g. `main`,
    /// `origin/main`). `None` means the user accepted the default
    /// (current HEAD at creation time). Captured so the user can
    /// answer "what is this lane based on?" later.
    #[serde(default)]
    pub base_ref: Option<String>,
    /// Free-form description set at creation time (e.g.
    /// "PR #123 review", "feat/sidebar TextInput IME"). Surfaced as
    /// the lane row sublabel in the left dock so an idle lane
    /// from last week is still self-describing.
    #[serde(default, alias = "task")]
    pub description: Option<String>,
}

impl SerializedLane {
    /// Default lane for a non-git path. Caller supplies a unique
    /// `id` (typically `0` since only one default exists per project).
    pub fn default_for_path(id: LaneId, path: PathBuf) -> Self {
        Self {
            id,
            kind: LaneKind::Default,
            path,
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: Vec::new(),
            active_tab_index: 0,
            base_ref: None,
            description: None,
        }
    }

    /// Resolved display name — user-set `name` if present, otherwise
    /// branch (for git) or path basename (for default).
    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref() {
            return name.to_string();
        }
        match &self.kind {
            LaneKind::Git {
                branch: Some(b), ..
            } => b.clone(),
            LaneKind::Git { branch: None, .. } => "(detached)".to_string(),
            LaneKind::Default => self
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string(),
        }
    }
}
