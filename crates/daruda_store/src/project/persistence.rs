//! File-based persistence for project state and recent list. Read/
//! write plumbing lives in [`crate::persistence`]; this module owns
//! only the path layout and the per-project hash → filename mapping.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! ├── projects/
//! │   ├── {hash}.json    # per-project state
//! │   └── ...
//! └── recent.json        # recent projects list
//! ```

use std::path::{Path, PathBuf};

use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

use super::{ProjectState, RECENT_MAX, RecentEntry, WorkspaceState, path_hash};

fn projects_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join("projects")
}

fn recent_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("recent.json")
}

fn state_path_in(data_dir: &Path, root: &Path) -> PathBuf {
    let hash = path_hash(root);
    projects_dir_in(data_dir).join(format!("{hash}.json"))
}

// ============================================================================
// Project state — path-explicit API (use in tests / custom data dirs)
// ============================================================================

/// Save workspace state under `data_dir` atomically (tempfile + rename).
///
/// Accepts the legacy [`ProjectState`] shape for runtime compatibility
/// during the multi-project rollout — internally promotes to the new
/// [`WorkspaceState`] format before serialization, so the on-disk file
/// is always the new shape going forward.
pub fn save_state_in(data_dir: &Path, state: &ProjectState) -> std::io::Result<()> {
    let dir = projects_dir_in(data_dir);
    let path = state_path_in(data_dir, &state.root);
    let workspace = WorkspaceState::from_legacy(state.clone());
    save_json_atomic(&dir, &path, &workspace)
}

/// Load workspace state from `data_dir`. Missing file → `None`. Parse
/// error → logged then `None` so the caller falls back to a fresh
/// session.
///
/// Tries the new [`WorkspaceState`] shape first and discriminates
/// against legacy files via the `schema_version` field — new-shape
/// files emit `WORKSPACE_SCHEMA_VERSION`, legacy files have no such
/// key and decode as `0`. A genuinely-empty new-shape file (no
/// projects, but schema_version set) therefore takes the new path,
/// not the legacy fallback. Returns the primary project's flat view
/// so runtime API stays unchanged while the disk format advances.
pub fn load_state_in(data_dir: &Path, root: &Path) -> Option<ProjectState> {
    let path = state_path_in(data_dir, root);
    // Both shapes use `#[serde(default)]` on every field, so this
    // first parse succeeds for legacy files too. We then dispatch on
    // `schema_version` (legacy = 0) rather than guessing from emptiness.
    match load_json_file::<WorkspaceState>("project", &path) {
        LoadOutcome::Parsed(mut workspace) => {
            if workspace.schema_version > 0 {
                workspace.migrate_legacy();
                return Some(workspace.into_primary_project_state());
            }
            // Legacy file: re-parse as `ProjectState` and migrate.
            match load_json_file::<ProjectState>("project", &path) {
                LoadOutcome::Parsed(mut state) => {
                    state.migrate_legacy();
                    Some(state)
                }
                LoadOutcome::Missing | LoadOutcome::Corrupt => None,
            }
        }
        LoadOutcome::Missing | LoadOutcome::Corrupt => None,
    }
}

/// Delete a project's state file under `data_dir`.
pub fn delete_state_in(data_dir: &Path, root: &Path) -> std::io::Result<()> {
    let path = state_path_in(data_dir, root);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

// ============================================================================
// Recent projects — path-explicit API
// ============================================================================

/// Load the recent projects list from `data_dir`. Empty vec on missing
/// file. A parse error is logged and treated as empty so a hand-edited
/// `recent.json` won't crash the welcome screen — but the corruption
/// is at least visible in the daruda log instead of being swallowed.
pub fn load_recent_in(data_dir: &Path) -> Vec<RecentEntry> {
    let path = recent_path_in(data_dir);
    match load_json_file::<Vec<RecentEntry>>("recent", &path) {
        LoadOutcome::Parsed(v) => v,
        LoadOutcome::Missing | LoadOutcome::Corrupt => Vec::new(),
    }
}

/// Save the recent projects list to `data_dir` atomically.
pub fn save_recent_in(data_dir: &Path, entries: &[RecentEntry]) -> std::io::Result<()> {
    let path = recent_path_in(data_dir);
    save_json_atomic(data_dir, &path, &entries)
}

/// Add or update a project in the recent list under `data_dir`.
pub fn touch_recent_in(data_dir: &Path, root: &Path) -> std::io::Result<()> {
    let mut entries = load_recent_in(data_dir);
    entries.retain(|e| e.root != root);
    entries.insert(0, RecentEntry::now(root));
    entries.truncate(RECENT_MAX);
    save_recent_in(data_dir, &entries)
}

/// Remove stale entries (directories that no longer exist) under `data_dir`.
pub fn prune_recent_in(data_dir: &Path) -> std::io::Result<()> {
    let mut entries = load_recent_in(data_dir);
    let before = entries.len();
    entries.retain(|e| e.root.is_dir());
    if entries.len() != before {
        save_recent_in(data_dir, &entries)?;
    }
    Ok(())
}

// ============================================================================
// Production convenience wrappers (use the default config dir)
// ============================================================================

/// Save project state to disk.
pub fn save_state(state: &ProjectState) -> std::io::Result<()> {
    save_state_in(&crate::persistence::default_data_dir(), state)
}

/// Load project state from disk. Returns `None` on any error. Applies
/// `migrate_legacy` so the returned state always uses the worktree shape.
pub fn load_state(root: &Path) -> Option<ProjectState> {
    load_state_in(&crate::persistence::default_data_dir(), root)
}

/// Delete a project's state file.
pub fn delete_state(root: &Path) -> std::io::Result<()> {
    delete_state_in(&crate::persistence::default_data_dir(), root)
}

/// Load the recent projects list. Returns empty vec on any error.
pub fn load_recent() -> Vec<RecentEntry> {
    load_recent_in(&crate::persistence::default_data_dir())
}

/// Save the recent projects list to disk.
pub fn save_recent(entries: &[RecentEntry]) -> std::io::Result<()> {
    save_recent_in(&crate::persistence::default_data_dir(), entries)
}

/// Add or update a project in the recent list. Moves existing entries
/// to the front; trims to RECENT_MAX.
pub fn touch_recent(root: &Path) -> std::io::Result<()> {
    touch_recent_in(&crate::persistence::default_data_dir(), root)
}

/// Remove stale entries (directories that no longer exist).
pub fn prune_recent() -> std::io::Result<()> {
    prune_recent_in(&crate::persistence::default_data_dir())
}
