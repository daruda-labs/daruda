//! Runtime representation of a lane — the Workspace-visible
//! counterpart to `daruda_store::project::SerializedLane`.
//!
//! Persistable fields mirror the serialized form; runtime-only fields
//! (e.g. `status`) are recomputed and never written to disk.

pub mod availability;
pub mod git;
pub mod history;
pub mod paths;
pub mod port_attribution;
pub mod session_host;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use daruda_store::project::{LaneId, LaneKind, LaneStatus, SerializedLane};

use self::availability::LaneAvailability;

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub id: LaneId,
    pub kind: LaneKind,
    pub path: PathBuf,
    pub name: Option<String>,
    pub tab_order: u32,
    pub is_unread: bool,
    /// Unix timestamp (seconds). `0` = unknown / never touched.
    pub last_activity: u64,
    /// Runtime-only. Rebuilt from live PTY/agent signals on each
    /// render; never serialized.
    pub status: LaneStatus,
    /// Runtime-only. True when this lane's `worktree_root` equals
    /// `repo_root` — i.e. this is the main (non-linked) working tree.
    /// Derived at construction time from a pure path comparison;
    /// never serialized.
    pub is_main: bool,
    /// Ref the lane was branched from (e.g. `main`,
    /// `origin/main`). `None` = current HEAD at creation. Persisted.
    pub base_ref: Option<String>,
    /// Free-form description shown as the dock row sublabel.
    pub description: Option<String>,
    /// Path on a *different* machine this lane's session connects to
    /// (e.g. an SSH-reachable VM). `None` means the lane runs against
    /// the local filesystem. Persisted.
    ///
    /// Legacy — names a path without its host. Superseded by
    /// [`Self::session_host`] and only consulted while that is `None`.
    pub remote_cwd: Option<String>,
    /// Where this lane's agent session attaches. `None` = never answered
    /// on this lane, which is what keeps the legacy `remote_cwd` pair
    /// alive; `Some(Local)` explicitly retires it. Persisted.
    pub session_host: Option<daruda_store::project::LaneSessionHost>,
    /// Runtime-only. Whether this lane's root directory can be read on
    /// disk. Recomputed from the live filesystem on restore / activate /
    /// failed load; never serialized. Defaults to `Present`.
    pub availability: LaneAvailability,
}

impl Lane {
    /// Fresh default lane for a non-git project path. Status
    /// starts `Idle`; `last_activity` = now.
    pub fn default_for_project(id: LaneId, path: PathBuf) -> Self {
        Self {
            id,
            kind: LaneKind::Default,
            path,
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: now_secs(),
            status: LaneStatus::Idle,
            // Default kind is never a git main worktree.
            is_main: false,
            base_ref: None,
            description: None,
            remote_cwd: None,
            session_host: None,
            availability: LaneAvailability::Present,
        }
    }

    /// Fresh lane entry backed by git.
    ///
    /// - `path`: initial value for `Lane.path` — the terminal-cwd
    ///   anchor (caller may anchor it to a subdir afterwards).
    /// - `repo_root`: shared common-dir top, same across every lane
    ///   of the repo. Used by `git worktree add/remove/branch`.
    /// - `worktree_root`: this lane's filesystem toplevel. Used by
    ///   every per-lane git CLI (`status`, `add`, `restore`).
    ///   Typically equal to `path` at construction time; immutable
    ///   afterwards.
    /// - `branch`: `None` for detached HEAD.
    pub fn git(
        id: LaneId,
        path: PathBuf,
        branch: Option<String>,
        repo_root: PathBuf,
        worktree_root: PathBuf,
        tab_order: u32,
    ) -> Self {
        let is_main = !worktree_root.as_os_str().is_empty() && worktree_root == repo_root;
        Self {
            id,
            kind: LaneKind::Git {
                branch,
                repo_root,
                worktree_root,
            },
            path,
            name: None,
            tab_order,
            is_unread: false,
            last_activity: now_secs(),
            status: LaneStatus::Idle,
            is_main,
            base_ref: None,
            description: None,
            remote_cwd: None,
            session_host: None,
            availability: LaneAvailability::Present,
        }
    }

    /// Inspect the filesystem at `project_root` and build the initial
    /// lane list. If git is installed and the path is a repo,
    /// every linked lane becomes an entry (bare checkouts are
    /// filtered out); the one at `project_root` sorts first so it
    /// becomes the active one. Non-git paths yield a single
    /// `Default` lane so the left dock always has at least one row.
    pub fn bootstrap_from_project(project_root: &std::path::Path) -> Vec<Lane> {
        match git::probe_repo(project_root) {
            Some(probe) => Self::from_repo_probe(project_root, probe)
                .unwrap_or_else(|| vec![Lane::default_for_project(0, project_root.to_path_buf())]),
            None => vec![Lane::default_for_project(0, project_root.to_path_buf())],
        }
    }

    /// Convert a successful repo probe into ordered runtime lanes.
    /// Returns `None` when the probe yielded no usable (non-bare)
    /// lanes, so the caller can fall back to a `Default`.
    fn from_repo_probe(project_root: &std::path::Path, probe: git::RepoProbe) -> Option<Vec<Lane>> {
        // Canonicalize so comparisons against git's own resolved paths line up.
        let canonical_target = daruda_core::path::canonicalize_or_self(project_root);

        let mut lanes: Vec<Lane> = probe
            .lanes
            .into_iter()
            .filter(|e| !e.bare)
            .enumerate()
            .map(|(i, entry)| {
                // `entry.path` is this lane's actual git toplevel
                // straight from `git worktree list --porcelain`. Capture
                // it now — the anchoring loop below may overwrite the
                // initial `Lane.path` with a sub-directory, but
                // `worktree_root` must stay at the real toplevel for
                // every per-lane git CLI to address the right index.
                let worktree_root = entry.path.clone();
                Lane::git(
                    i as LaneId,
                    entry.path,
                    entry.branch,
                    probe.repo_root.clone(),
                    worktree_root,
                    i as u32,
                )
            })
            .collect();

        if lanes.is_empty() {
            return None;
        }

        // When the user opens a subdirectory of a git repo (e.g. opens
        // `term/daruda` inside the `term` git root), the main lane's
        // path comes back from `git worktree list` as the repo root.
        // Anchor it to the project root so new panes start inside the
        // user's intended directory rather than the repo root.
        for w in lanes.iter_mut() {
            if canonical_target.starts_with(&w.path) && canonical_target != w.path {
                w.path = canonical_target.clone();
                break;
            }
        }

        // Put the lane matching `project_root` first so the
        // caller's `active_lane_id = 0` lands on it.
        if let Some(idx) = lanes.iter().position(|w| w.path == canonical_target)
            && idx != 0
        {
            let chosen = lanes.remove(idx);
            lanes.insert(0, chosen);
        }
        // Reassign ids/tab_order after reordering so id = position.
        for (i, w) in lanes.iter_mut().enumerate() {
            w.id = i as LaneId;
            w.tab_order = i as u32;
        }
        Some(lanes)
    }

    /// Hydrate runtime state from the on-disk form. Tabs on the
    /// serialized record are ignored here — the workspace-restore
    /// caller rebuilds `Workspace.tabs` from them. Status defaults to
    /// `Idle`.
    ///
    /// Legacy state files (saved before the `worktree_root` field was
    /// introduced) deserialize with an empty `PathBuf` for that slot.
    /// `Self::backfill_lane_root` re-derives it via a one-stat
    /// filesystem walk so existing projects keep working without a
    /// schema migration step.
    pub fn from_serialized(s: &SerializedLane) -> Self {
        let mut kind = s.kind.clone();
        Self::backfill_lane_root(&mut kind, &s.path);
        // If backfill_lane_root failed (empty worktree_root — e.g. the
        // lane directory was deleted on disk), is_main resolves to false.
        // That is a degenerate, harmless case: the lane is already broken.
        let is_main = Self::is_main_kind(&kind);
        Self {
            id: s.id,
            kind,
            path: s.path.clone(),
            name: s.name.clone(),
            tab_order: s.tab_order,
            is_unread: s.is_unread,
            last_activity: s.last_activity,
            status: LaneStatus::default(),
            is_main,
            base_ref: s.base_ref.clone(),
            description: s.description.clone(),
            remote_cwd: s.remote_cwd.clone(),
            session_host: s.session_host.clone(),
            // Recomputed from disk by the workspace restore path; the
            // serialized form carries no availability.
            availability: LaneAvailability::Present,
        }
    }

    /// True when `kind` describes the git main working tree —
    /// `worktree_root` is non-empty and equals `repo_root`.
    fn is_main_kind(kind: &LaneKind) -> bool {
        matches!(kind, LaneKind::Git { repo_root, worktree_root, .. }
            if !worktree_root.as_os_str().is_empty() && worktree_root == repo_root)
    }

    /// If `kind` is a Git variant with no recorded `worktree_root`
    /// (legacy persisted state from before the field existed), walk up
    /// from `wt_path` to find the first ancestor containing a `.git`
    /// entry and use that as the toplevel. Silent no-op when the field
    /// is already populated or when no `.git` exists above `wt_path`
    /// (e.g. the lane was deleted on disk).
    fn backfill_lane_root(kind: &mut LaneKind, wt_path: &std::path::Path) {
        let LaneKind::Git { worktree_root, .. } = kind else {
            return;
        };
        if !worktree_root.as_os_str().is_empty() {
            return;
        }
        if let Some(top) = find_git_toplevel(wt_path) {
            *worktree_root = top;
        }
    }

    /// Produce a serialized snapshot sans tabs (tabs are written by
    /// the workspace save path, which has access to the live tab
    /// list).
    pub fn to_serialized(&self) -> SerializedLane {
        SerializedLane {
            id: self.id,
            kind: self.kind.clone(),
            path: self.path.clone(),
            name: self.name.clone(),
            tab_order: self.tab_order,
            is_unread: self.is_unread,
            last_activity: self.last_activity,
            base_ref: self.base_ref.clone(),
            description: self.description.clone(),
            remote_cwd: self.remote_cwd.clone(),
            session_host: self.session_host.clone(),
            tabs: Vec::new(),
            active_tab_index: 0,
        }
    }

    /// User-facing display name — same resolution rules as
    /// `SerializedLane::display_name`.
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

    /// Replace this lane's kind and recompute the derived `is_main`
    /// flag. The one update site for in-place kind changes, so the
    /// `is_main` mirror can never drift from `worktree_root == repo_root`.
    pub fn set_kind(&mut self, kind: LaneKind) {
        self.is_main = Self::is_main_kind(&kind);
        self.kind = kind;
    }

    /// Overwrite the user-set display name. `None` clears back to the
    /// derived default (`display_name` falls through to branch / path).
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// Overwrite the free-form description shown in the left dock
    /// sublabel. `None` removes it and reverts to the lane path.
    pub fn set_description(&mut self, description: Option<String>) {
        self.description = description;
    }

    /// Overwrite the remote path this lane's session connects to.
    /// `None` reverts the lane to running against the local
    /// filesystem.
    ///
    /// Legacy — new callers set [`Self::set_session_host`], which names
    /// the host alongside the path.
    pub fn set_remote_cwd(&mut self, remote_cwd: Option<String>) {
        self.remote_cwd = remote_cwd;
    }

    /// Answer where this lane's agent session attaches. Passing
    /// `Some(LaneSessionHost::Local)` is the explicit "run it here" that
    /// retires the legacy `remote_cwd` pair for this lane; `None` puts
    /// the lane back to never having answered.
    pub fn set_session_host(
        &mut self,
        session_host: Option<daruda_store::project::LaneSessionHost>,
    ) {
        self.session_host = session_host;
    }

    /// Where this lane's session attaches, given the agent it launches under
    /// and the live registry catalog/tombstones. `launch` only matters while
    /// the lane has never answered; `catalog`/`tombstones` only matter while
    /// the resolved host carries a `registry_id` — see
    /// [`session_host::effective_session_host`]. `Err` is a host the launch
    /// quoting cannot carry; the caller refuses rather than running locally.
    pub fn effective_session_host(
        &self,
        launch: &daruda_config::AgentLaunch,
        catalog: &[daruda_config::SessionHostEntry],
        tombstones: &[daruda_config::SessionHostTombstone],
    ) -> Result<daruda_store::project::LaneSessionHost, session_host::UnusableSessionHost> {
        session_host::effective_session_host(
            self.session_host.as_ref(),
            self.remote_cwd.as_deref(),
            launch,
            catalog,
            tombstones,
        )
    }

    /// `true` when this lane is git-backed.
    pub fn is_git(&self) -> bool {
        self.kind.is_git()
    }

    /// This lane's git toplevel — the directory holding its `.git`
    /// entry (a regular `.git/` for the main lane, a `.git` pointer
    /// file for linked lanes). Stored on the kind at probe time
    /// and persisted, so this is a constant-time field access — no
    /// filesystem walk on every stage / unstage.
    ///
    /// `git status --porcelain` returns paths relative to this
    /// toplevel, so every per-lane git CLI (`status`, `add`,
    /// `restore --staged`) must cwd here to keep porcelain paths and
    /// the per-lane index aligned.
    ///
    /// Returns `None` for `Default` (non-git) lanes and for legacy
    /// persisted Git lanes whose backfill walk failed (the lane
    /// was deleted on disk between save and reload). Callers treat both
    /// as "no git ops possible on this lane".
    pub fn git_worktree_root(&self) -> Option<&std::path::Path> {
        match &self.kind {
            LaneKind::Git { worktree_root, .. } => {
                if worktree_root.as_os_str().is_empty() {
                    None
                } else {
                    Some(worktree_root.as_path())
                }
            }
            LaneKind::Default => None,
        }
    }

    /// Path-conversion helper for this lane.
    ///
    /// Captures `wt.path` and this lane's git toplevel so callers
    /// can convert among the three path spaces (git-status-relative,
    /// wt-relative, absolute) without raw `.join()` arithmetic. The
    /// toplevel — not the shared `repo_root` — is what porcelain paths
    /// resolve against, so linked-lane path conversion lands on
    /// the right working tree.
    pub fn paths(&self) -> paths::LanePaths<'_> {
        paths::LanePaths {
            wt_path: &self.path,
            repo_root: self.git_worktree_root(),
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Walk up from `start` to find the first ancestor that contains a
/// `.git` entry — a regular directory for the main lane or a
/// pointer file for linked lanes. Used only by the legacy-state
/// migration in [`Lane::backfill_lane_root`]; production paths
/// read the persisted `worktree_root` field directly.
fn find_git_toplevel(start: &std::path::Path) -> Option<PathBuf> {
    let mut current: &std::path::Path = start;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}
