//! A workspace repository whose location is private and fixed at construction.

use std::io;
use std::path::PathBuf;

use super::{ProjectState, ProjectUuid, RecentEntry, WorkspaceState, WorkspaceUuid, persistence};

mod migration;

/// Workspace, project and recent-list persistence, independent of config paths.
#[derive(Clone)]
pub struct WorkspaceStore {
    root: PathBuf,
}

impl WorkspaceStore {
    /// Prepare native state storage before exposing a repository. The caller
    /// must exclude other app instances using the legacy profile's desktop lock.
    pub fn open_current() -> io::Result<Self> {
        let layout = crate::storage::StorageLayout::current();
        let root = layout.workspace_state()?;
        migration::WorkspaceMigration::new(layout.data(), root.clone()).run()?;
        Ok(Self { root })
    }

    /// Bind an exact existing-layout directory, without discovery or migration.
    /// Useful for injected repositories, tests and explicit legacy access.
    pub fn in_directory(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Load a workspace by its stable identifier.
    pub fn load_workspace(&self, uuid: WorkspaceUuid) -> Option<WorkspaceState> {
        persistence::load_workspace_state_in(&self.root, uuid)
    }

    /// Atomically persist a workspace.
    pub fn save_workspace(&self, state: &WorkspaceState) -> io::Result<()> {
        persistence::save_workspace_state_in(&self.root, state)
    }

    /// Remove a workspace that can no longer be reached.
    pub fn delete_workspace(&self, uuid: WorkspaceUuid) -> io::Result<()> {
        persistence::delete_workspace_state_in(&self.root, uuid)
    }

    /// Load a project's intrinsic state.
    pub fn load_project(&self, uuid: ProjectUuid) -> Option<ProjectState> {
        persistence::load_project_state_in(&self.root, uuid)
    }

    /// Atomically persist a project's intrinsic state.
    pub fn save_project(&self, state: &ProjectState) -> io::Result<()> {
        persistence::save_project_state_in(&self.root, state)
    }

    /// Visit projects without exposing their directory or file names.
    pub fn for_each_project(&self, visit: impl FnMut(ProjectState)) {
        persistence::for_each_project_state_in(&self.root, visit);
    }

    /// Load the recent workspace list.
    pub fn load_recent(&self) -> Vec<RecentEntry> {
        persistence::load_recent_in(&self.root)
    }

    /// Replace the recent workspace list atomically.
    pub fn save_recent(&self, entries: &[RecentEntry]) -> io::Result<()> {
        persistence::save_recent_in(&self.root, entries)
    }

    /// Move a nonempty workspace to the front of the recent list.
    pub fn touch_recent(&self, uuid: WorkspaceUuid, name: String) -> io::Result<()> {
        persistence::touch_recent_in(&self.root, uuid, name)
    }

    /// Refresh an existing row without inserting or reordering it.
    pub fn refresh_recent_if_present(&self, uuid: WorkspaceUuid, name: String) -> io::Result<bool> {
        persistence::refresh_recent_if_present_in(&self.root, uuid, name)
    }
}
