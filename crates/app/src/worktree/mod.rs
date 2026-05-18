//! Runtime representation of a worktree — the Workspace-visible
//! counterpart to `daruda_store::project::SerializedWorktree`.
//!
//! Persistable fields mirror the serialized form; runtime-only fields
//! (e.g. `status`) are recomputed and never written to disk. Tabs and
//! panes move into this struct in W-3; until then, `Workspace` keeps
//! its single top-level tab list and this struct carries only
//! metadata.

pub mod git;
pub mod paths;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use daruda_store::project::{SerializedWorktree, WorktreeId, WorktreeKind, WorktreeStatus};

#[derive(Clone, Debug)]
pub struct Worktree {
    pub id: WorktreeId,
    pub kind: WorktreeKind,
    pub path: PathBuf,
    pub name: Option<String>,
    pub tab_order: u32,
    pub is_unread: bool,
    /// Unix timestamp (seconds). `0` = unknown / never touched.
    pub last_activity: u64,
    /// Runtime-only. Rebuilt from live PTY/agent signals on each
    /// render; never serialized.
    pub status: WorktreeStatus,
    /// Ref the worktree was branched from (e.g. `main`,
    /// `origin/main`). `None` = current HEAD at creation. Persisted.
    pub base_ref: Option<String>,
    /// Free-form description shown as the dock row sublabel.
    pub description: Option<String>,
}

impl Worktree {
    /// Fresh default worktree for a non-git project path. Status
    /// starts `Idle`; `last_activity` = now.
    pub fn default_for_project(id: WorktreeId, path: PathBuf) -> Self {
        Self {
            id,
            kind: WorktreeKind::Default,
            path,
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: now_secs(),
            status: WorktreeStatus::Idle,
            base_ref: None,
            description: None,
        }
    }

    /// Fresh worktree entry backed by git.
    ///
    /// - `path`: initial value for `Worktree.path` — the terminal-cwd
    ///   anchor (caller may anchor it to a subdir afterwards).
    /// - `repo_root`: shared common-dir top, same across every worktree
    ///   of the repo. Used by `git worktree add/remove/branch`.
    /// - `worktree_root`: this worktree's filesystem toplevel. Used by
    ///   every per-worktree git CLI (`status`, `add`, `restore`).
    ///   Typically equal to `path` at construction time; immutable
    ///   afterwards.
    /// - `branch`: `None` for detached HEAD.
    pub fn git(
        id: WorktreeId,
        path: PathBuf,
        branch: Option<String>,
        repo_root: PathBuf,
        worktree_root: PathBuf,
        tab_order: u32,
    ) -> Self {
        Self {
            id,
            kind: WorktreeKind::Git {
                branch,
                repo_root,
                worktree_root,
            },
            path,
            name: None,
            tab_order,
            is_unread: false,
            last_activity: now_secs(),
            status: WorktreeStatus::Idle,
            base_ref: None,
            description: None,
        }
    }

    /// Inspect the filesystem at `project_root` and build the initial
    /// worktree list. If git is installed and the path is a repo,
    /// every linked worktree becomes an entry (bare checkouts are
    /// filtered out); the one at `project_root` sorts first so it
    /// becomes the active one. Non-git paths yield a single
    /// `Default` worktree so the left dock always has at least one row.
    pub fn bootstrap_from_project(project_root: &std::path::Path) -> Vec<Worktree> {
        match git::probe_repo(project_root) {
            Some(probe) => Self::from_repo_probe(project_root, probe).unwrap_or_else(|| {
                vec![Worktree::default_for_project(0, project_root.to_path_buf())]
            }),
            None => vec![Worktree::default_for_project(0, project_root.to_path_buf())],
        }
    }

    /// Convert a successful repo probe into ordered runtime worktrees.
    /// Returns `None` when the probe yielded no usable (non-bare)
    /// worktrees, so the caller can fall back to a `Default`.
    fn from_repo_probe(
        project_root: &std::path::Path,
        probe: git::RepoProbe,
    ) -> Option<Vec<Worktree>> {
        // Try to canonicalize the project_root so comparisons against
        // git's resolved paths succeed on macOS (/tmp → /private/tmp).
        let canonical_target =
            std::fs::canonicalize(project_root).unwrap_or_else(|_| project_root.to_path_buf());

        let mut worktrees: Vec<Worktree> = probe
            .worktrees
            .into_iter()
            .filter(|e| !e.bare)
            .enumerate()
            .map(|(i, entry)| {
                // `entry.path` is this worktree's actual git toplevel
                // straight from `git worktree list --porcelain`. Capture
                // it now — the anchoring loop below may overwrite the
                // initial `Worktree.path` with a sub-directory, but
                // `worktree_root` must stay at the real toplevel for
                // every per-worktree git CLI to address the right index.
                let worktree_root = entry.path.clone();
                Worktree::git(
                    i as WorktreeId,
                    entry.path,
                    entry.branch,
                    probe.repo_root.clone(),
                    worktree_root,
                    i as u32,
                )
            })
            .collect();

        if worktrees.is_empty() {
            return None;
        }

        // When the user opens a subdirectory of a git repo (e.g. opens
        // `term/daruda` inside the `term` git root), the main worktree's
        // path comes back from `git worktree list` as the repo root.
        // Anchor it to the project root so new panes start inside the
        // user's intended directory rather than the repo root.
        for w in worktrees.iter_mut() {
            if canonical_target.starts_with(&w.path) && canonical_target != w.path {
                w.path = canonical_target.clone();
                break;
            }
        }

        // Put the worktree matching `project_root` first so the
        // caller's `active_worktree_id = 0` lands on it.
        if let Some(idx) = worktrees.iter().position(|w| w.path == canonical_target)
            && idx != 0
        {
            let chosen = worktrees.remove(idx);
            worktrees.insert(0, chosen);
        }
        // Reassign ids/tab_order after reordering so id = position.
        for (i, w) in worktrees.iter_mut().enumerate() {
            w.id = i as WorktreeId;
            w.tab_order = i as u32;
        }
        Some(worktrees)
    }

    /// Hydrate runtime state from the on-disk form. Tabs stored on
    /// the serialized record are ignored here — the caller (workspace
    /// restore) is responsible for rebuilding `Workspace.tabs` from
    /// them. Status always defaults to `Idle`.
    ///
    /// Legacy state files (saved before the `worktree_root` field was
    /// introduced) deserialize with an empty `PathBuf` for that slot.
    /// `Self::backfill_worktree_root` re-derives it via a one-stat
    /// filesystem walk so existing projects keep working without a
    /// schema migration step.
    pub fn from_serialized(s: &SerializedWorktree) -> Self {
        let mut kind = s.kind.clone();
        Self::backfill_worktree_root(&mut kind, &s.path);
        Self {
            id: s.id,
            kind,
            path: s.path.clone(),
            name: s.name.clone(),
            tab_order: s.tab_order,
            is_unread: s.is_unread,
            last_activity: s.last_activity,
            status: WorktreeStatus::default(),
            base_ref: s.base_ref.clone(),
            description: s.description.clone(),
        }
    }

    /// If `kind` is a Git variant with no recorded `worktree_root`
    /// (legacy persisted state from before the field existed), walk up
    /// from `wt_path` to find the first ancestor containing a `.git`
    /// entry and use that as the toplevel. Silent no-op when the field
    /// is already populated or when no `.git` exists above `wt_path`
    /// (e.g. the worktree was deleted on disk).
    fn backfill_worktree_root(kind: &mut WorktreeKind, wt_path: &std::path::Path) {
        let WorktreeKind::Git { worktree_root, .. } = kind else {
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
    pub fn to_serialized(&self) -> SerializedWorktree {
        SerializedWorktree {
            id: self.id,
            kind: self.kind.clone(),
            path: self.path.clone(),
            name: self.name.clone(),
            tab_order: self.tab_order,
            is_unread: self.is_unread,
            last_activity: self.last_activity,
            base_ref: self.base_ref.clone(),
            description: self.description.clone(),
            tabs: Vec::new(),
            active_tab_index: 0,
        }
    }

    /// User-facing display name — same resolution rules as
    /// `SerializedWorktree::display_name`.
    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref() {
            return name.to_string();
        }
        match &self.kind {
            WorktreeKind::Git {
                branch: Some(b), ..
            } => b.clone(),
            WorktreeKind::Git { branch: None, .. } => "(detached)".to_string(),
            WorktreeKind::Default => self
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string(),
        }
    }

    /// Overwrite the user-set display name. `None` clears back to the
    /// derived default (`display_name` falls through to branch / path).
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// Overwrite the free-form description shown in the left dock
    /// sublabel. `None` removes it and reverts to the worktree path.
    pub fn set_description(&mut self, description: Option<String>) {
        self.description = description;
    }

    /// `true` when this worktree is git-backed.
    pub fn is_git(&self) -> bool {
        self.kind.is_git()
    }

    /// This worktree's git toplevel — the directory holding its `.git`
    /// entry (a regular `.git/` for the main worktree, a `.git` pointer
    /// file for linked worktrees). Stored on the kind at probe time
    /// and persisted, so this is a constant-time field access — no
    /// filesystem walk on every stage / unstage.
    ///
    /// `git status --porcelain` returns paths relative to this
    /// toplevel, so every per-worktree git CLI (`status`, `add`,
    /// `restore --staged`) must cwd here to keep porcelain paths and
    /// the per-worktree index aligned.
    ///
    /// Returns `None` for `Default` (non-git) worktrees and for legacy
    /// persisted Git worktrees whose backfill walk failed (the worktree
    /// was deleted on disk between save and reload). Callers treat both
    /// as "no git ops possible on this worktree".
    pub fn git_worktree_root(&self) -> Option<&std::path::Path> {
        match &self.kind {
            WorktreeKind::Git { worktree_root, .. } => {
                if worktree_root.as_os_str().is_empty() {
                    None
                } else {
                    Some(worktree_root.as_path())
                }
            }
            WorktreeKind::Default => None,
        }
    }

    /// Path-conversion helper for this worktree.
    ///
    /// Captures `wt.path` and this worktree's git toplevel so callers
    /// can convert among the three path spaces (git-status-relative,
    /// wt-relative, absolute) without raw `.join()` arithmetic. The
    /// toplevel — not the shared `repo_root` — is what porcelain paths
    /// resolve against, so linked-worktree path conversion lands on
    /// the right working tree.
    pub fn paths(&self) -> paths::WorktreePaths<'_> {
        paths::WorktreePaths {
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
/// `.git` entry — a regular directory for the main worktree or a
/// pointer file for linked worktrees. Used only by the legacy-state
/// migration in [`Worktree::backfill_worktree_root`]; production paths
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_for_project_is_default_kind() {
        let w = Worktree::default_for_project(0, PathBuf::from("/tmp/scratch"));
        assert_eq!(w.kind, WorktreeKind::Default);
        assert!(!w.is_git());
        assert_eq!(w.status, WorktreeStatus::Idle);
        assert!(w.last_activity > 0);
    }

    #[test]
    fn display_name_uses_path_basename_for_default() {
        let w = Worktree::default_for_project(0, PathBuf::from("/Users/alice/scratch"));
        assert_eq!(w.display_name(), "scratch");
    }

    #[test]
    fn display_name_uses_branch_for_git() {
        let mut w = Worktree::default_for_project(0, PathBuf::from("/repo"));
        w.kind = WorktreeKind::Git {
            branch: Some("feat/sidebar".into()),
            repo_root: PathBuf::from("/repo"),
            worktree_root: PathBuf::from("/repo"),
        };
        assert_eq!(w.display_name(), "feat/sidebar");
    }

    #[test]
    fn display_name_prefers_user_name() {
        let mut w = Worktree::default_for_project(0, PathBuf::from("/tmp"));
        w.name = Some("My Work".into());
        assert_eq!(w.display_name(), "My Work");
    }

    #[test]
    fn bootstrap_non_git_returns_single_default() {
        let dir = std::env::temp_dir().join("daruda_boot_non_git");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wts = Worktree::bootstrap_from_project(&dir);
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].id, 0);
        assert_eq!(wts[0].kind, WorktreeKind::Default);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bootstrap_git_repo_single_worktree() {
        if !git::has_git() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("daruda_boot_git_single_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        git::init(&dir).unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.email", "daruda@test"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.name", "daruda"])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "--allow-empty", "-m", "initial"])
            .output()
            .unwrap();

        let wts = Worktree::bootstrap_from_project(&dir);
        assert_eq!(wts.len(), 1);
        assert!(wts[0].is_git());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bootstrap_git_repo_multi_worktree_sorts_main_first() {
        if !git::has_git() {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("daruda_boot_git_multi_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        git::init(&dir).unwrap();
        let _ = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.email", "daruda@test"])
            .output();
        let _ = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["config", "user.name", "daruda"])
            .output();
        let _ = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "--allow-empty", "-m", "initial"])
            .output();

        let extra = dir.join("wt-side");
        git::add_worktree(&dir, &extra, Some("side"), None).unwrap();

        let wts = Worktree::bootstrap_from_project(&dir);
        assert_eq!(wts.len(), 2);
        // The project_root's worktree must sort to id 0.
        assert_eq!(wts[0].path, dir);
        assert_eq!(wts[0].id, 0);
        assert_eq!(wts[1].id, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_through_serialized_drops_tabs_only() {
        let w = Worktree {
            id: 7,
            kind: WorktreeKind::Git {
                branch: Some("main".into()),
                repo_root: PathBuf::from("/repo"),
                worktree_root: PathBuf::from("/repo/wt-feat"),
            },
            path: PathBuf::from("/repo/wt-feat"),
            name: Some("Feature branch".into()),
            tab_order: 2,
            is_unread: true,
            last_activity: 12345,
            status: WorktreeStatus::Running,
            base_ref: Some("origin/main".into()),
            description: Some("PR #123".into()),
        };
        let s = w.to_serialized();
        let back = Worktree::from_serialized(&s);
        assert_eq!(back.id, 7);
        assert_eq!(back.kind, w.kind);
        assert_eq!(back.path, w.path);
        assert_eq!(back.name, w.name);
        assert_eq!(back.tab_order, 2);
        assert!(back.is_unread);
        assert_eq!(back.last_activity, 12345);
        // Status is runtime-only and always resets to Idle.
        assert_eq!(back.status, WorktreeStatus::Idle);
        // Serialized form carries no tabs yet (W-3 wires them).
        assert!(s.tabs.is_empty());
        assert_eq!(s.active_tab_index, 0);
        // base_ref + description survive the round-trip.
        assert_eq!(back.base_ref, Some("origin/main".to_string()));
        assert_eq!(back.description, Some("PR #123".to_string()));
    }
}
