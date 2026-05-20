//! Disk persistence for the UUID-keyed workspace/project schema.
//!
//! Layout under `<data_dir>/`:
//! ```text
//! workspaces/<workspace_uuid>.json
//! projects/<project_uuid>.json        # legacy <hex_hash>.json files
//!                                     # coexist here but are skipped
//! recent-workspaces.json
//! ```
//!
//! UUID files are filtered by filename pattern (36 chars with hyphens
//! at positions 8-13-18-23). Anything else in `projects/` (including
//! legacy fnv1a-hashed files) is ignored during scans.

use std::path::{Path, PathBuf};

use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

use super::types::{ProjectState, ProjectUuid, RecentEntry, WorkspaceState, WorkspaceUuid};

const WORKSPACES_DIRNAME: &str = "workspaces";
const PROJECTS_DIRNAME: &str = "projects";
const RECENT_FILENAME: &str = "recent-workspaces.json";

pub const RECENT_MAX: usize = 20;

pub fn workspaces_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join(WORKSPACES_DIRNAME)
}

pub fn projects_dir_in(data_dir: &Path) -> PathBuf {
    data_dir.join(PROJECTS_DIRNAME)
}

pub fn recent_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join(RECENT_FILENAME)
}

fn workspace_path_in(data_dir: &Path, uuid: WorkspaceUuid) -> PathBuf {
    workspaces_dir_in(data_dir).join(format!("{}.json", uuid.as_inner()))
}

fn project_path_in(data_dir: &Path, uuid: ProjectUuid) -> PathBuf {
    projects_dir_in(data_dir).join(format!("{}.json", uuid.as_inner()))
}

/// Returns true iff `stem` is a canonical lowercase UUID (8-4-4-4-12
/// hex with hyphens). Used to skip legacy `<hex_hash>.json` files
/// coexisting in `projects/`.
pub fn is_uuid_filename_stem(stem: &str) -> bool {
    if stem.len() != 36 {
        return false;
    }
    let bytes = stem.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        let want_hyphen = matches!(i, 8 | 13 | 18 | 23);
        if want_hyphen {
            if *b != b'-' {
                return false;
            }
        } else if !matches!(b, b'0'..=b'9' | b'a'..=b'f') {
            return false;
        }
    }
    true
}

// ---- WorkspaceState ----

pub fn save_workspace_state_in(data_dir: &Path, state: &WorkspaceState) -> std::io::Result<()> {
    let dir = workspaces_dir_in(data_dir);
    let path = workspace_path_in(data_dir, state.uuid);
    save_json_atomic(&dir, &path, state)
}

pub fn load_workspace_state_in(data_dir: &Path, uuid: WorkspaceUuid) -> Option<WorkspaceState> {
    let path = workspace_path_in(data_dir, uuid);
    match load_json_file::<WorkspaceState>("workspace", &path) {
        LoadOutcome::Parsed(ws) => Some(ws),
        LoadOutcome::Missing | LoadOutcome::Corrupt => None,
    }
}

pub fn delete_workspace_state_in(data_dir: &Path, uuid: WorkspaceUuid) -> std::io::Result<()> {
    let path = workspace_path_in(data_dir, uuid);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Iterate every workspace file in `<data_dir>/workspaces/`. Used
/// by lookup paths (e.g. `find_existing_project_uuid_for_root`).
pub fn for_each_workspace_state_in<F: FnMut(WorkspaceState)>(data_dir: &Path, mut f: F) {
    let dir = workspaces_dir_in(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_uuid_filename_stem(stem) {
            continue;
        }
        if let LoadOutcome::Parsed(ws) = load_json_file::<WorkspaceState>("workspace", &path) {
            f(ws);
        }
    }
}

// ---- ProjectState ----

pub fn save_project_state_in(data_dir: &Path, state: &ProjectState) -> std::io::Result<()> {
    let dir = projects_dir_in(data_dir);
    let path = project_path_in(data_dir, state.uuid);
    save_json_atomic(&dir, &path, state)
}

pub fn load_project_state_in(data_dir: &Path, uuid: ProjectUuid) -> Option<ProjectState> {
    let path = project_path_in(data_dir, uuid);
    match load_json_file::<ProjectState>("project", &path) {
        LoadOutcome::Parsed(p) => Some(p),
        LoadOutcome::Missing | LoadOutcome::Corrupt => None,
    }
}

pub fn delete_project_state_in(data_dir: &Path, uuid: ProjectUuid) -> std::io::Result<()> {
    let path = project_path_in(data_dir, uuid);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Iterate every project file in `<data_dir>/projects/` that has a
/// canonical-UUID stem. Legacy hex-hash files in the same folder are
/// silently skipped.
pub fn for_each_project_state_in<F: FnMut(ProjectState)>(data_dir: &Path, mut f: F) {
    let dir = projects_dir_in(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_uuid_filename_stem(stem) {
            continue;
        }
        if let LoadOutcome::Parsed(p) = load_json_file::<ProjectState>("project", &path) {
            f(p);
        }
    }
}

// ---- Recent ----

pub fn load_recent_in(data_dir: &Path) -> Vec<RecentEntry> {
    match load_json_file::<Vec<RecentEntry>>("recent_workspaces", &recent_path_in(data_dir)) {
        LoadOutcome::Parsed(v) => v,
        LoadOutcome::Missing | LoadOutcome::Corrupt => Vec::new(),
    }
}

pub fn save_recent_in(data_dir: &Path, entries: &[RecentEntry]) -> std::io::Result<()> {
    save_json_atomic(data_dir, &recent_path_in(data_dir), &entries)
}

pub fn touch_recent_in(
    data_dir: &Path,
    workspace_uuid: WorkspaceUuid,
    display_name: String,
) -> std::io::Result<()> {
    let mut entries = load_recent_in(data_dir);
    entries.retain(|e| e.workspace_uuid != workspace_uuid);
    entries.insert(0, RecentEntry::now(workspace_uuid, display_name));
    entries.truncate(RECENT_MAX);
    save_recent_in(data_dir, &entries)
}
