//! `WorkspaceStore` persistence through its public API.

use std::collections::BTreeMap;

use daruda_store::project::{WorkspaceStore, WorkspaceUuid};

use crate::support::{sample_project, sample_workspace};

#[test]
fn snapshot_history_is_workspace_scoped_and_intentional_deletion_is_final() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkspaceStore::in_directory(dir.path());
    let a = WorkspaceUuid::new();
    let b = WorkspaceUuid::new();
    let snapshots = BTreeMap::from([(3, "한글🙂".into())]);
    store.save_terminal_snapshots(a, &snapshots).unwrap();
    store
        .save_terminal_snapshots(a, &BTreeMap::from([(3, "next".into())]))
        .unwrap();
    assert!(store.load_terminal_snapshots(b).unwrap().is_empty());
    store.delete_terminal_snapshots(a).unwrap();
    assert!(store.load_terminal_snapshots(a).unwrap().is_empty());
}

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
