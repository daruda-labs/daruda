use super::*;

// ---- add_project / close_active_project / window_open_policy ----

#[gpui::test]
fn add_project_mints_next_id_and_activates_first_worktree(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Start with a single bootstrapped project (id 0).
    let project = daruda_store::project::Project::from_path("/tmp/daruda_add_first");
    std::fs::create_dir_all("/tmp/daruda_add_first").unwrap();
    std::fs::create_dir_all("/tmp/daruda_add_second").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    // Pre-state: 1 project with id 0, next counter = 1.
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].id, 0);
        assert_eq!(ws.next_project_id, 1);
        assert_eq!(ws.active.project, 0);
    });

    // Add a second project.
    let target = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_add_second"),
                window,
                cx,
            )
        })
    });
    let target = target.expect("add_project window callback succeeded");
    let target = target.expect("add_project returned a WorktreeRef");

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects.len(), 2);
        assert_eq!(ws.projects[1].id, 1);
        assert_eq!(ws.next_project_id, 2);
        assert_eq!(ws.active, target);
        assert_eq!(ws.active.project, 1);
    });
}

#[gpui::test]
fn close_active_project_returns_false_when_last(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_last");
    std::fs::create_dir_all("/tmp/daruda_close_last").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let keep = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    });
    let keep = keep.expect("close_active_project window callback succeeded");
    assert!(!keep, "last project should signal close-window");
    ws.read_with(cx, |ws, _| {
        assert!(ws.projects.is_empty());
        assert_eq!(ws.active, daruda_store::project::WorktreeRef::default());
    });
}

#[gpui::test]
fn close_active_project_keeps_window_when_other_remain(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_keep_a");
    std::fs::create_dir_all("/tmp/daruda_close_keep_a").unwrap();
    std::fs::create_dir_all("/tmp/daruda_close_keep_b").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_close_keep_b"),
                window,
                cx,
            )
        })
    })
    .ok();
    // After adding, two projects exist; close active project (id 1)
    // and verify project 0 remains active.
    let keep = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    });
    let keep = keep.expect("close_active_project window callback succeeded");
    assert!(keep, "closing one of two projects must keep the window");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].id, 0);
        assert_eq!(ws.active.project, 0);
    });
}

#[gpui::test]
fn window_open_policy_round_trips_through_state(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_policy_round");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.window_open_policy(),
            daruda_store::project::WindowOpenPolicy::Ask
        );
    });
    ws.update(cx, |ws, cx| {
        ws.set_window_open_policy(daruda_store::project::WindowOpenPolicy::NewWindow, cx);
    });
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .expect("save_state");
    assert_eq!(
        state.window_open_policy,
        daruda_store::project::WindowOpenPolicy::NewWindow
    );
    assert_eq!(state.next_project_id, 1);
    assert!(state.projects.iter().any(|p| p.id == 0));
}

#[gpui::test]
fn close_active_project_clears_file_tree_caches_for_project(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_filetree");
    std::fs::create_dir_all("/tmp/daruda_close_filetree").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    // Seed a fake file_tree entry for the active worktree.
    let active = ws.read_with(cx, |ws, _| ws.active);
    ws.update(cx, |ws, _| {
        let tree = crate::files::tree::FileTree::new(std::path::PathBuf::from(
            "/tmp/daruda_close_filetree",
        ));
        ws.file_tree.file_trees.insert(active, tree);
    });
    let _ = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    });
    // After close, no file_tree entries remain for the closed project.
    ws.read_with(cx, |ws, _| {
        assert!(
            !ws.file_tree
                .file_trees
                .keys()
                .any(|k| k.project == active.project),
            "file_trees must drop entries belonging to the closed project"
        );
    });
}

// ---- Group CRUD ----

#[gpui::test]
fn add_group_mints_monotonic_id_and_persists(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_add");
    std::fs::create_dir_all("/tmp/daruda_group_add").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let (id_a, id_b) = ws.update(cx, |ws, cx| {
        let a = ws.add_group("Frontend".to_string(), None, cx);
        let b = ws.add_group("Backend".to_string(), Some("#ff0000".into()), cx);
        (a, b)
    });
    assert_ne!(id_a, id_b, "ids must be unique");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.groups.len(), 2);
        assert_eq!(ws.next_group_id, 2);
    });
    // Round-trip through state preserves the groups.
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .expect("save_state");
    assert_eq!(state.groups.len(), 2);
    assert_eq!(state.next_group_id, 2);
}

#[gpui::test]
fn delete_group_demotes_member_projects_to_ungrouped(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_delete");
    std::fs::create_dir_all("/tmp/daruda_group_delete").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let group_id = ws.update(cx, |ws, cx| {
        let id = ws.add_group("temp".to_string(), None, cx);
        // Move the bootstrapped project into the new group.
        let project_id = ws.projects[0].id;
        ws.move_project_to_group(project_id, Some(id), cx);
        id
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects[0].group_id, Some(group_id));
    });
    ws.update(cx, |ws, cx| {
        ws.delete_group(group_id, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert!(ws.groups.is_empty());
        assert_eq!(
            ws.projects[0].group_id, None,
            "deleted group must demote member projects"
        );
    });
}

#[gpui::test]
fn rename_recolor_collapse_round_trip(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_edit");
    std::fs::create_dir_all("/tmp/daruda_group_edit").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let id = ws.update(cx, |ws, cx| {
        let id = ws.add_group("old".to_string(), None, cx);
        assert!(ws.rename_group(id, "new".to_string(), cx));
        ws.recolor_group(id, Some("#abcdef".into()), cx);
        ws.toggle_group_collapse(id, cx);
        id
    });
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .expect("save_state");
    let group = state.groups.iter().find(|g| g.id == id).unwrap();
    assert_eq!(group.name, "new");
    assert_eq!(group.color.as_deref(), Some("#abcdef"));
    assert!(group.is_collapsed);
}

#[gpui::test]
fn move_project_to_unknown_group_is_noop(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_unknown");
    std::fs::create_dir_all("/tmp/daruda_group_unknown").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    let before = ws.read_with(cx, |ws, _| ws.projects[0].group_id);
    ws.update(cx, |ws, cx| {
        let project_id = ws.projects[0].id;
        ws.move_project_to_group(project_id, Some(9999), cx);
    });
    let after = ws.read_with(cx, |ws, _| ws.projects[0].group_id);
    assert_eq!(before, after, "stale group id must not silently demote");
}

#[gpui::test]
fn has_project_root_matches_existing_project(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_has_root");
    std::fs::create_dir_all("/tmp/daruda_has_root").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.has_project_root(std::path::Path::new("/tmp/daruda_has_root")));
        assert!(!ws.has_project_root(std::path::Path::new("/tmp/daruda_has_root_other")));
    });
}
