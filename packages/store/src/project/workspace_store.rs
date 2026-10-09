//! A workspace repository whose location is private and fixed at construction.

use std::io;
use std::path::PathBuf;

use super::{ProjectState, ProjectUuid, RecentEntry, WorkspaceState, WorkspaceUuid, persistence};

mod migration;
mod terminal_snapshots;

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

    /// Restore every referenced project, or refuse a partial workspace.
    /// Damaged records are recovered from retained backups where possible.
    pub fn load_complete_workspace(
        &self,
        uuid: WorkspaceUuid,
    ) -> io::Result<Option<(WorkspaceState, Vec<ProjectState>)>> {
        let path =
            persistence::workspaces_dir_in(&self.root).join(format!("{}.json", uuid.as_inner()));
        let Some(workspace) =
            crate::persistence::load_recoverable_json::<WorkspaceState>("workspace", &path)?
        else {
            return Ok(None);
        };
        if workspace.uuid != uuid || workspace.schema_version > super::WORKSPACE_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Workspace identity mismatch",
            ));
        }
        let mut projects = Vec::new();
        for id in &workspace.project_ids {
            let path =
                persistence::projects_dir_in(&self.root).join(format!("{}.json", id.as_inner()));
            let project =
                crate::persistence::load_recoverable_json::<ProjectState>("project", &path)?
                    .filter(|project| {
                        project.uuid == *id
                            && project.schema_version <= super::WORKSPACE_SCHEMA_VERSION
                    })
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "Referenced project {} is unavailable; workspace was preserved",
                                id.as_inner()
                            ),
                        )
                    })?;
            projects.push(project);
        }
        Ok(Some((workspace, projects)))
    }

    /// Atomically persist a workspace.
    pub fn save_workspace(&self, state: &WorkspaceState) -> io::Result<()> {
        persistence::save_workspace_state_in(&self.root, state)
    }

    /// Remove a workspace that can no longer be reached.
    pub fn delete_workspace(&self, uuid: WorkspaceUuid) -> io::Result<()> {
        self.delete_terminal_snapshots(uuid)?;
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::project::tests::new_schema_fixtures::{sample_project, sample_workspace};

    #[test]
    fn incomplete_workspace_is_preserved_instead_of_partially_restored() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::in_directory(dir.path());
        let project = sample_project();
        let workspace = sample_workspace(project.uuid);
        store.save_workspace(&workspace).unwrap();
        assert!(store.load_complete_workspace(workspace.uuid).is_err());
        assert_eq!(
            store.load_workspace(workspace.uuid),
            Some(workspace.clone())
        );
        store.save_project(&project).unwrap();
        assert_eq!(
            store.load_complete_workspace(workspace.uuid).unwrap(),
            Some((workspace, vec![project]))
        );
    }
}
