use super::*;
use crate::project::{
    WorkspaceStore,
    tests::new_schema_fixtures::{sample_project, sample_workspace},
};
use std::fs::File;

fn migration(root: &Path) -> WorkspaceMigration {
    WorkspaceMigration::new(root.join("legacy"), root.join("state").join("workspace"))
}

fn seed(migration: &WorkspaceMigration) -> (ProjectState, WorkspaceState) {
    let project = sample_project();
    let workspace = sample_workspace(project.uuid);
    persistence::save_project_state_in(&migration.source, &project).unwrap();
    persistence::save_workspace_state_in(&migration.source, &workspace).unwrap();
    persistence::touch_recent_in(&migration.source, workspace.uuid, "example".into()).unwrap();
    (project, workspace)
}

#[test]
fn imports_only_workspace_records_and_preserves_original_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    let (project, workspace) = seed(&migration);
    let project_file = PathBuf::from("projects").join(format!("{}.json", project.uuid.as_inner()));
    let original = fs::read(migration.source.join(&project_file)).unwrap();
    fs::write(migration.source.join("config.toml"), "[font]").unwrap();
    fs::write(
        migration.source.join("projects/abcdef.json"),
        "legacy hash record",
    )
    .unwrap();
    fs::create_dir_all(migration.source.join("projects/repo-hash/flows")).unwrap();
    fs::write(
        migration.source.join("projects/repo-hash/config.toml"),
        "[shell]",
    )
    .unwrap();
    migration.run().unwrap();
    let store = WorkspaceStore::in_directory(migration.destination.clone());
    assert_eq!(
        store.load_workspace(workspace.uuid),
        Some(workspace.clone())
    );
    assert_eq!(store.load_project(project.uuid), Some(project.clone()));
    assert_eq!(store.load_recent()[0].workspace_uuid, workspace.uuid);
    assert_eq!(
        fs::read(migration.destination.join(&project_file)).unwrap(),
        original
    );
    assert_eq!(
        fs::read(migration.source.join(&project_file)).unwrap(),
        original
    );
    assert!(!migration.destination.join("config.toml").exists());
    assert!(!migration.destination.join("projects/repo-hash").exists());
    assert!(!migration.destination.join("projects/abcdef.json").exists());
    let mut updated = project.clone();
    updated.name = Some("new state".into());
    store.save_project(&updated).unwrap();
    migration.run().unwrap();
    assert_eq!(store.load_project(project.uuid), Some(updated));
    assert_eq!(
        persistence::load_project_state_in(&migration.source, project.uuid),
        Some(project)
    );
}

#[test]
fn corrupt_input_never_publishes_and_can_be_retried_after_repair() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    seed(&migration);
    let recent = persistence::recent_path_in(&migration.source);
    let original = fs::read(&recent).unwrap();
    fs::write(&recent, b"{ broken").unwrap();
    let error = migration.run().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains(&recent.display().to_string()));
    assert!(error.to_string().contains("Cannot import workspace record"));
    assert!(!migration.destination.exists());
    assert_eq!(fs::read(&recent).unwrap(), b"{ broken");
    fs::write(&recent, original).unwrap();
    migration.run().unwrap();
    assert!(migration.destination.join(MARKER).is_file());
}

#[test]
fn abandoned_staging_is_never_treated_as_published_state() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    let (_, workspace) = seed(&migration);
    let abandoned = temporary
        .path()
        .join("state/.workspace-migration-abandoned");
    fs::create_dir_all(&abandoned).unwrap();
    fs::write(abandoned.join("partial.json"), "incomplete").unwrap();
    migration.run().unwrap();
    let store = WorkspaceStore::in_directory(migration.destination.clone());
    assert!(store.load_workspace(workspace.uuid).is_some());
    assert!(!migration.destination.join("partial.json").exists());
    assert!(abandoned.join("partial.json").exists());
}

#[test]
fn existing_unrecognized_destination_is_never_overwritten() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    seed(&migration);
    fs::create_dir_all(&migration.destination).unwrap();
    let existing = migration.destination.join("keep.txt");
    fs::write(&existing, "user data").unwrap();
    assert!(migration.run().is_err());
    assert_eq!(fs::read_to_string(existing).unwrap(), "user data");
    assert!(!migration.destination.join(MARKER).exists());
}

#[test]
fn malformed_or_future_layout_marker_blocks_migration() {
    for contents in ["invalid", r#"{"version":99}"#] {
        let temporary = tempfile::tempdir().unwrap();
        let migration = migration(temporary.path());
        fs::create_dir_all(&migration.destination).unwrap();
        fs::write(migration.destination.join(MARKER), contents).unwrap();
        assert_eq!(
            migration.run().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            fs::read_to_string(migration.destination.join(MARKER)).unwrap(),
            contents
        );
    }
}

#[test]
fn future_schema_and_mismatched_identity_do_not_become_current_state() {
    for future in [false, true] {
        let temporary = tempfile::tempdir().unwrap();
        let migration = migration(temporary.path());
        let (mut project, _) = seed(&migration);
        let path = persistence::projects_dir_in(&migration.source)
            .join(format!("{}.json", project.uuid.as_inner()));
        if future {
            project.schema_version = WORKSPACE_SCHEMA_VERSION + 1;
        } else {
            project.uuid = crate::project::ProjectUuid::new();
        }
        fs::write(&path, serde_json::to_vec(&project).unwrap()).unwrap();
        let error = migration.run().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains(&path.display().to_string()));
        assert!(!migration.destination.exists());
    }
}

#[test]
fn selected_records_must_be_regular_files() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    let (project, _) = seed(&migration);
    let path = persistence::projects_dir_in(&migration.source)
        .join(format!("{}.json", project.uuid.as_inner()));
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert_eq!(
        migration.run().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!migration.destination.exists());
}

#[test]
fn migration_lock_prevents_concurrent_publication() {
    let temporary = tempfile::tempdir().unwrap();
    let migration = migration(temporary.path());
    fs::create_dir_all(migration.destination.parent().unwrap()).unwrap();
    let lock = File::create(
        migration
            .destination
            .parent()
            .unwrap()
            .join(".workspace-migration.lock"),
    )
    .unwrap();
    lock.lock().unwrap();
    assert_eq!(
        migration.run().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    assert!(!migration.destination.exists());
}

#[test]
fn two_repositories_do_not_share_mutable_state() {
    let temporary = tempfile::tempdir().unwrap();
    let one = WorkspaceStore::in_directory(temporary.path().join("one"));
    let two = WorkspaceStore::in_directory(temporary.path().join("two"));
    let uuid = crate::project::WorkspaceUuid::new();
    one.touch_recent(uuid, "one".into()).unwrap();
    assert!(two.load_recent().is_empty());
    two.touch_recent(uuid, "two".into()).unwrap();
    assert_eq!(one.load_recent()[0].display_name, "one");
    assert_eq!(two.load_recent()[0].display_name, "two");
}
