//! Install the prepared repository once; injected workspace roots stay isolated.

use std::path::{Path, PathBuf};

use daruda_store::project::WorkspaceStore;
use gpui::{App, Global};

struct WorkspaceStorage {
    profile_root: PathBuf,
    repository: WorkspaceStore,
}

impl Global for WorkspaceStorage {}

pub(crate) fn install(repository: WorkspaceStore, cx: &mut App) {
    cx.set_global(WorkspaceStorage {
        profile_root: daruda_store::persistence::default_data_dir(),
        repository,
    });
}

pub(crate) fn for_root(root: &Path, cx: &App) -> WorkspaceStore {
    match cx.try_global::<WorkspaceStorage>() {
        Some(storage) if daruda_core::path::same_path(&storage.profile_root, root) => {
            storage.repository.clone()
        }
        _ => WorkspaceStore::in_directory(root.to_path_buf()),
    }
}

pub(crate) fn current(cx: &App) -> WorkspaceStore {
    match cx.try_global::<WorkspaceStorage>() {
        Some(storage) => storage.repository.clone(),
        // Tests can construct windows without running the production bootstrap.
        None => WorkspaceStore::in_directory(daruda_store::persistence::default_data_dir()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn prepared_repository_and_injected_roots_are_independent(cx: &mut gpui::TestAppContext) {
        let legacy = tempfile::tempdir().unwrap();
        let prepared = tempfile::tempdir().unwrap();
        let injected = tempfile::tempdir().unwrap();
        let uuid = daruda_store::project::WorkspaceUuid::new();
        cx.update(|cx| {
            cx.set_global(WorkspaceStorage {
                profile_root: legacy.path().to_path_buf(),
                repository: WorkspaceStore::in_directory(prepared.path()),
            });
            for_root(legacy.path(), cx)
                .touch_recent(uuid, "prepared".into())
                .unwrap();
            assert_eq!(current(cx).load_recent()[0].display_name, "prepared");
            let injected_store = for_root(injected.path(), cx);
            assert!(injected_store.load_recent().is_empty());
            injected_store
                .touch_recent(uuid, "injected".into())
                .unwrap();
            assert_eq!(current(cx).load_recent()[0].display_name, "prepared");
        });
        assert!(
            WorkspaceStore::in_directory(legacy.path())
                .load_recent()
                .is_empty()
        );
        assert_eq!(
            WorkspaceStore::in_directory(injected.path()).load_recent()[0].display_name,
            "injected"
        );
    }
}
