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
use super::SessionHostId;

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
    /// Path on a *different* machine this lane's session connects
    /// to (e.g. an SSH-reachable VM). Plain `String`, not `PathBuf` —
    /// the local filesystem has no opinion on whether a remote path
    /// exists, so it must not be validated or canonicalized locally.
    /// `None` means the lane runs against the local filesystem.
    ///
    /// Legacy: it names a path without naming the host, which lived on
    /// the agent instead. Superseded by [`Self::session_host`], which
    /// carries both. Only consulted while `session_host` is `None`.
    #[serde(default)]
    pub remote_cwd: Option<String>,
    /// Where this lane's agent session attaches. `None` means the user
    /// has never answered the question on this lane, which is what lets
    /// the legacy `remote_cwd` + agent-side host pair still apply;
    /// `Some(LaneSessionHost::Local)` is an explicit "run it here" and
    /// retires that pair for good.
    #[serde(default)]
    pub session_host: Option<LaneSessionHost>,
}

/// Where a lane's agent session attaches. `Local` runs the adapter
/// against the lane's own [`SerializedLane::path`]; the other two run it
/// on another machine, so they carry the path over there — a host with
/// no path, or a path with no host, cannot be expressed.
///
/// Paths are plain `String`, not `PathBuf`: the local filesystem has no
/// opinion on a path that lives elsewhere, so it must not be validated
/// or canonicalized here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaneSessionHost {
    #[default]
    Local,
    Ssh {
        /// Host to `ssh` to — an alias from the user's SSH config, or
        /// `user@host`.
        target: String,
        /// Working directory on that host.
        session_path: String,
        /// Registry entry (`daruda_config::SessionHostEntry`) this host was
        /// picked from, if any. `None` for a free-text host never linked to
        /// a catalog row, and for every lane persisted before the registry
        /// existed — `#[serde(default)]` keeps those loading as `None`
        /// instead of failing to deserialize.
        #[serde(default)]
        registry_id: Option<SessionHostId>,
    },
    Docker {
        /// Name of an already-running container.
        container: String,
        /// Working directory inside that container.
        session_path: String,
        /// Registry entry this host was picked from, if any — see the
        /// `Ssh` variant's `registry_id` doc.
        #[serde(default)]
        registry_id: Option<SessionHostId>,
    },
}

impl LaneSessionHost {
    /// Whether the session runs somewhere other than this machine — the
    /// question every "can this be done locally?" caller is really
    /// asking (browser OAuth, a local `PATH` probe, a spawned process).
    pub fn is_remote(&self) -> bool {
        !matches!(self, LaneSessionHost::Local)
    }

    /// The working directory the session is rooted at, for a remote
    /// host. `None` for `Local`, whose directory is the lane's own path.
    pub fn session_path(&self) -> Option<&str> {
        match self {
            LaneSessionHost::Local => None,
            LaneSessionHost::Ssh { session_path, .. }
            | LaneSessionHost::Docker { session_path, .. } => Some(session_path),
        }
    }
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
            remote_cwd: None,
            session_host: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_session_host_round_trips_every_variant_with_registry_id() {
        let cases = [
            LaneSessionHost::Local,
            LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            },
            LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(SessionHostId::new()),
            },
            LaneSessionHost::Docker {
                container: "dev-1".into(),
                session_path: "/workspace".into(),
                registry_id: None,
            },
            LaneSessionHost::Docker {
                container: "dev-1".into(),
                session_path: "/workspace".into(),
                registry_id: Some(SessionHostId::new()),
            },
        ];
        for case in cases {
            let json = serde_json::to_string(&case).expect("serialize");
            let back: LaneSessionHost = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, case, "{json}");
        }
    }

    /// Regression anchor: a lane JSON file persisted before the registry
    /// existed has no `registry_id` key at all — it must still deserialize,
    /// with `registry_id` defaulting to `None`.
    #[test]
    fn missing_registry_id_key_deserializes_as_none() {
        let json = r#"{"type":"ssh","target":"vm-work","session_path":"/srv/app"}"#;
        let host: LaneSessionHost = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            host,
            LaneSessionHost::Ssh {
                target: "vm-work".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            }
        );

        let json = r#"{"type":"docker","container":"dev-1","session_path":"/workspace"}"#;
        let host: LaneSessionHost = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            host,
            LaneSessionHost::Docker {
                container: "dev-1".into(),
                session_path: "/workspace".into(),
                registry_id: None,
            }
        );
    }
}
