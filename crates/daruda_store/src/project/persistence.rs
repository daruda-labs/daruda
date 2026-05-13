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

use super::{ProjectState, RECENT_MAX, RecentEntry, path_hash};

/// Back-compat shim — new code should call
/// [`crate::persistence::default_data_dir`] directly.
pub fn default_data_dir() -> PathBuf {
    crate::persistence::default_data_dir()
}

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

/// Save project state under `data_dir` atomically (tempfile + rename).
pub fn save_state_in(data_dir: &Path, state: &ProjectState) -> std::io::Result<()> {
    let dir = projects_dir_in(data_dir);
    let path = state_path_in(data_dir, &state.root);
    save_json_atomic(&dir, &path, state)
}

/// Load project state from `data_dir`. Missing file → `None`. Parse
/// error → logged then `None` so the caller falls back to a fresh
/// session. Applies `migrate_legacy` so the returned state always uses
/// the worktree shape.
pub fn load_state_in(data_dir: &Path, root: &Path) -> Option<ProjectState> {
    let path = state_path_in(data_dir, root);
    match load_json_file::<ProjectState>("project", &path) {
        LoadOutcome::Parsed(mut state) => {
            state.migrate_legacy();
            Some(state)
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
    save_state_in(&default_data_dir(), state)
}

/// Load project state from disk. Returns `None` on any error. Applies
/// `migrate_legacy` so the returned state always uses the worktree shape.
pub fn load_state(root: &Path) -> Option<ProjectState> {
    load_state_in(&default_data_dir(), root)
}

/// Delete a project's state file.
pub fn delete_state(root: &Path) -> std::io::Result<()> {
    delete_state_in(&default_data_dir(), root)
}

/// Load the recent projects list. Returns empty vec on any error.
pub fn load_recent() -> Vec<RecentEntry> {
    load_recent_in(&default_data_dir())
}

/// Save the recent projects list to disk.
pub fn save_recent(entries: &[RecentEntry]) -> std::io::Result<()> {
    save_recent_in(&default_data_dir(), entries)
}

/// Add or update a project in the recent list. Moves existing entries
/// to the front; trims to RECENT_MAX.
pub fn touch_recent(root: &Path) -> std::io::Result<()> {
    touch_recent_in(&default_data_dir(), root)
}

/// Remove stale entries (directories that no longer exist).
pub fn prune_recent() -> std::io::Result<()> {
    prune_recent_in(&default_data_dir())
}
