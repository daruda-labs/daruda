//! Fan-out regression: confirm `persist_state` writes one workspace
//! file (not one per project) and one project file per project.
//! Includes a shared-root scenario: two workspaces holding the
//! same project root should not pollute each other's snapshot.

use super::*;
use gpui::BorrowAppContext as _;

#[gpui::test]
fn terminal_output_restore_uses_the_owning_project_and_profile(cx: &mut TestAppContext) {
    let data = tempfile::tempdir().unwrap();
    let other_profile = tempfile::tempdir().unwrap();
    let root_a = tempfile::tempdir().unwrap();
    let root_b = tempfile::tempdir().unwrap();
    let write_override = |data: &std::path::Path, root: &std::path::Path, enabled: bool| {
        let path = daruda_config::project::project_config_path_in(data, root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("[shell]\nrestore_output = {enabled}\n")).unwrap();
    };
    write_override(data.path(), root_a.path(), true);
    write_override(data.path(), root_b.path(), false);
    write_override(other_profile.path(), root_b.path(), true);
    let handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &daruda_config::Config::default(),
            Some(daruda_store::project::Project::from_path(root_a.path())),
            data.path().to_owned(),
            window,
            cx,
        )
    });
    let ws = handle.root(cx).unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(root_b.path().to_owned(), window, cx);
            ws.add_tab(window, cx);
            assert_eq!(ws.projects.len(), 2);
            let enabled_project = ws.projects[0].id;
            let expected: std::collections::BTreeSet<_> = ws
                .main_area
                .runtimes
                .iter()
                .filter(|(lane, _)| lane.project == enabled_project)
                .flat_map(|(_, runtime)| &runtime.panes)
                .filter(|pane| pane.terminal_view().is_some())
                .map(|pane| pane.id)
                .collect();
            assert!(!expected.is_empty());
            ws.persist_state(cx);
            let store = daruda_store::project::WorkspaceStore::in_directory(data.path());
            let saved = store.load_terminal_snapshots(ws.uuid).unwrap();
            assert_eq!(
                saved
                    .keys()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            write_override(data.path(), root_a.path(), false);
            ws.persist_state(cx);
            assert!(store.load_terminal_snapshots(ws.uuid).unwrap().is_empty());
        });
    })
    .unwrap();
}

#[gpui::test]
fn persist_state_namespaces_workspace_and_project_files(cx: &mut TestAppContext) {
    // Two workspaces sharing a project root:
    // - W1: projects A + B, persist
    // - W2: project A alone, persist (distinct workspace_uuid)
    // Each workspace mints its own ProjectUuid (via `Project::bootstrap`
    // → `ProjectUuid::new()`). Key invariant: W2's write must not
    // clobber W1's workspace file — loading W1 back still lists its
    // original two projects.
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let root_a = std::env::temp_dir().join("daruda_regression_ns_share_a");
    let root_b = std::env::temp_dir().join("daruda_regression_ns_share_b");
    let _ = std::fs::create_dir_all(&root_a);
    let _ = std::fs::create_dir_all(&root_b);

    let config = daruda_config::Config::default();

    // ---- W1: A + B ----
    let project_a1 = daruda_store::project::Project::from_path(&root_a);
    let w1_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project_a1),
            data_dir.clone(),
            window,
            cx,
        )
    });
    let w1 = w1_handle.root(cx).unwrap();
    cx.update_window(w1_handle.into(), |_, window, cx| {
        w1.update(cx, |ws, cx| {
            ws.add_project(root_b.clone(), window, cx);
        })
    })
    .unwrap();
    w1.read_with(cx, |w, cx| w.persist_state(cx));
    let (w1_uuid, w1_project_ids) = w1.read_with(cx, |w, _| {
        let ids: Vec<_> = w.projects.iter().map(|p| p.uuid).collect();
        (w.uuid, ids)
    });
    assert_eq!(w1_project_ids.len(), 2, "W1 holds two projects");
    let workspaces_dir = daruda_store::project::workspaces_dir_in(&data_dir);
    let workspace_count = std::fs::read_dir(&workspaces_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|s| s == "json").unwrap_or(false))
        .count();
    assert_eq!(
        workspace_count, 1,
        "expected exactly 1 workspace file after W1, found {workspace_count}"
    );

    let projects_dir = daruda_store::project::projects_dir_in(&data_dir);
    let project_count = std::fs::read_dir(&projects_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let stem = e
                .path()
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            daruda_store::project::is_uuid_filename_stem(&stem)
        })
        .count();
    assert_eq!(
        project_count, 2,
        "expected 2 project files after W1, found {project_count}"
    );
    let recent_after_w1 = daruda_store::project::load_recent_in(&data_dir);
    assert_eq!(
        recent_after_w1.len(),
        1,
        "W1 writes exactly one recent entry"
    );
    assert_eq!(recent_after_w1[0].workspace_uuid, w1_uuid);

    // ---- W2: A alone ----
    // Fresh `from_path` so W2 mints its own ProjectUuid for A.
    let project_a2 = daruda_store::project::Project::from_path(&root_a);
    let w2_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project_a2),
            data_dir.clone(),
            window,
            cx,
        )
    });
    let w2 = w2_handle.root(cx).unwrap();
    w2.read_with(cx, |w, cx| w.persist_state(cx));
    let w2_uuid = w2.read_with(cx, |w, _| w.uuid);

    assert_ne!(w1_uuid, w2_uuid, "W1 and W2 are distinct workspaces");

    // Invariant 1: W1's workspace file still on disk with W1's UUID.
    let w1_loaded = daruda_store::project::load_workspace_state_in(&data_dir, w1_uuid)
        .expect("W1 workspace file must survive W2's persist");
    assert_eq!(w1_loaded.uuid, w1_uuid);
    assert_eq!(
        w1_loaded.project_ids.len(),
        2,
        "W1's project list must still reference its original two projects"
    );
    for pid in &w1_project_ids {
        assert!(
            w1_loaded.project_ids.contains(pid),
            "W1's reload must still list project uuid {pid:?}"
        );
    }

    // Invariant 2: W2's workspace file on disk references one project.
    let w2_loaded = daruda_store::project::load_workspace_state_in(&data_dir, w2_uuid)
        .expect("W2 workspace file present");
    assert_eq!(w2_loaded.uuid, w2_uuid);
    assert_eq!(
        w2_loaded.project_ids.len(),
        1,
        "W2 references exactly one project"
    );

    // Invariant 3: recent-workspaces.json contains both UUIDs (most
    // recent first — W2 wrote last).
    let recent = daruda_store::project::load_recent_in(&data_dir);
    assert!(recent.len() >= 2, "recent has both entries");
    assert_eq!(recent[0].workspace_uuid, w2_uuid);
    assert!(recent.iter().any(|e| e.workspace_uuid == w1_uuid));

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// A screenshot scenario seeds fixtures into the live workspace; with
/// persistence suspended neither the layout nor the task list reaches disk.
#[gpui::test]
fn suspended_persistence_writes_nothing(cx: &mut TestAppContext) {
    let data_dir = fresh_test_data_dir();
    let root = tempfile::tempdir().unwrap();
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &daruda_config::Config::default(),
            Some(daruda_store::project::Project::from_path(root.path())),
            data_dir.clone(),
            window,
            cx,
        )
    });
    let ws = window_handle.root(cx).unwrap();
    cx.run_until_parked();
    let listing = || {
        let mut names: Vec<_> = walkdir(&data_dir);
        names.sort();
        names
    };
    let before = listing();
    ws.update(cx, |ws, cx| {
        ws.persistence_suspended = true;
        let project = ws.active_project().unwrap().uuid;
        cx.update_global::<crate::agent::tasks_global::GlobalTasks, _>(|tasks, _| {
            tasks.add(daruda_store::tasks::Task::new(
                project,
                "Fixture".into(),
                String::new(),
                None,
            ));
        });
        ws.save_tasks_dirty(cx);
    });
    ws.read_with(cx, |ws, cx| ws.persist_state(cx));
    cx.run_until_parked();
    assert_eq!(listing(), before);
}

/// Every path under `dir` with its mtime, so a rewrite in place counts too.
fn walkdir(dir: &std::path::Path) -> Vec<(std::path::PathBuf, std::time::SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| {
            let path = entry.path();
            let mut found = walkdir(&path);
            if let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) {
                found.push((path, modified));
            }
            found
        })
        .collect()
}
