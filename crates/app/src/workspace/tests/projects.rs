use super::*;

// ---- add_project / close_active_project / window_open_policy ----

#[gpui::test]
fn add_project_mints_next_id_and_activates_first_lane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    // Start with a single bootstrapped project (id 0).
    let project = daruda_store::project::Project::from_path("/tmp/daruda_add_first");
    std::fs::create_dir_all("/tmp/daruda_add_first").unwrap();
    std::fs::create_dir_all("/tmp/daruda_add_second").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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
    let target = target.expect("add_project returned a LaneRef");

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
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    let keep = cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    });
    let keep = keep.expect("close_active_project window callback succeeded");
    assert!(!keep, "last project should signal close-window");
    ws.read_with(cx, |ws, _| {
        assert!(ws.projects.is_empty());
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
    });
}

#[gpui::test]
fn close_active_project_releases_pane_tracking(cx: &mut TestAppContext) {
    // Closing a project must release every pane it owns — live and
    // frozen runtimes alike — from PTY tracking, or the tracker keeps
    // polling the dead shell PIDs for the window's lifetime.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_release");
    std::fs::create_dir_all("/tmp/daruda_close_release").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_tab(window, cx);
            let pane_ids: Vec<_> = ws
                .main_area
                .tabs
                .iter()
                .flat_map(|t| t.layout.pane_ids())
                .collect();
            assert!(!pane_ids.is_empty(), "tab spawns at least one pane");
            for id in &pane_ids {
                ws.claude.pty_tracker.register(*id, 4242);
                ws.claude.pty_claude_bindings.insert(
                    *id,
                    crate::hooks::pty_tracker::PtyBinding {
                        claude_pid: 4242,
                        session_id: format!("sess-{id}"),
                    },
                );
            }

            let keep = ws.close_active_project(window, cx);

            assert!(!keep, "last project should signal close-window");
            assert!(
                ws.claude.pty_claude_bindings.is_empty(),
                "closed project's pane bindings must be dropped"
            );
            assert!(
                ws.claude.pty_tracker.tracked_pane_ids().is_empty(),
                "closed project's panes must be unregistered from the tracker"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn close_active_project_keeps_window_when_other_remain(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_keep_a");
    std::fs::create_dir_all("/tmp/daruda_close_keep_a").unwrap();
    std::fs::create_dir_all("/tmp/daruda_close_keep_b").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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
        // Regression: main area must not be empty after delete. The
        // surviving project's lane should be re-activated with at
        // least one tab/pane so the user doesn't see a blank viewport.
        assert!(
            !ws.main_area.tabs.is_empty(),
            "main area tabs must not be empty after closing a project"
        );
        assert!(
            !ws.main_area.panes.is_empty(),
            "main area panes must not be empty after closing a project"
        );
    });
}

#[gpui::test]
fn close_active_project_signals_window_close_when_no_survivor_has_a_lane(cx: &mut TestAppContext) {
    // Safety net: if every surviving project is somehow lane-less
    // (runtime corruption), closing the active project must treat the
    // workspace as empty — signal the caller to close the window
    // (→ Welcome) rather than leaving a blank viewport open.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_no_lane_a");
    std::fs::create_dir_all("/tmp/daruda_close_no_lane_a").unwrap();
    std::fs::create_dir_all("/tmp/daruda_close_no_lane_b").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    // Add project B → it becomes the active project (id 1).
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_close_no_lane_b"),
                window,
                cx,
            )
        })
    })
    .ok();
    // Corrupt the surviving project A (id 0): empty its lane list.
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.projects.iter_mut().find(|p| p.id == 0) {
            p.lanes.clear();
        }
    });
    // Close active project B. The only survivor (A) has no usable lane.
    let keep = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
        })
        .unwrap();
    assert!(
        !keep,
        "a workspace with no usable lane must signal window close"
    );
    ws.read_with(cx, |ws, _| {
        // Live runtime is cleared; the window is closing, so the user
        // lands on Welcome instead of a blank viewport.
        assert!(ws.main_area.tabs.is_empty());
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
    });
}

#[gpui::test]
fn open_delete_project_modal_by_id_does_not_change_active(cx: &mut TestAppContext) {
    // Root-wrapped window so `open_form_modal`'s `window.open_dialog`
    // can walk the gpui_component::Root layer.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_delete_byid_a");
    std::fs::create_dir_all("/tmp/daruda_delete_byid_a").unwrap();
    std::fs::create_dir_all("/tmp/daruda_delete_byid_b").unwrap();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));

    // Add project B; `add_project` makes the new project (id 1) active,
    // leaving project 0 as the non-active target for this test.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(
                std::path::PathBuf::from("/tmp/daruda_delete_byid_b"),
                window,
                cx,
            )
        })
    })
    .ok();
    let active_before = ws.read_with(cx, |ws, _| ws.active);
    assert_eq!(active_before.project, 1, "added project becomes active");

    // Opening the delete chooser for the *non-active* project (id 0)
    // must NOT force activation onto it — activation only happens once
    // the user confirms inside the modal, so cancel leaves focus put.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.open_delete_project_modal(0, window, cx));
    })
    .ok();
    cx.run_until_parked();

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.active, active_before,
            "merely opening the delete modal for another project must not \
             change the active focus"
        );
        assert_eq!(ws.projects.len(), 2, "both projects still present");
    });
}

#[gpui::test]
fn window_open_policy_round_trips_through_state(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_policy_round");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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
    let (workspace_state, project_states) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.window_open_policy,
        daruda_store::project::WindowOpenPolicy::NewWindow
    );
    // New schema: `next_project_id` is a runtime-only counter (per-
    // session) rather than persisted. Verify the runtime counter and
    // that the workspace state references exactly one project.
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.next_project_id, 1);
    });
    assert_eq!(workspace_state.project_ids.len(), 1);
    assert_eq!(project_states.len(), 1);
}

#[gpui::test]
fn close_active_project_clears_file_tree_caches_for_project(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_close_filetree");
    std::fs::create_dir_all("/tmp/daruda_close_filetree").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    // Seed a fake file_tree entry for the active lane.
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
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(workspace_state.groups.len(), 2);
    assert_eq!(workspace_state.next_group_id, 2);
}

#[gpui::test]
fn delete_group_demotes_member_projects_to_ungrouped(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_delete");
    std::fs::create_dir_all("/tmp/daruda_group_delete").unwrap();
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
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
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    let id = ws.update(cx, |ws, cx| {
        let id = ws.add_group("old".to_string(), None, cx);
        assert!(ws.rename_group(id, "new".to_string(), cx));
        ws.recolor_group(id, Some("#abcdef".into()), cx);
        ws.toggle_group_collapse(id, cx);
        id
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    let group = workspace_state.groups.iter().find(|g| g.id == id).unwrap();
    assert_eq!(group.name, "new");
    assert_eq!(group.color.as_deref(), Some("#abcdef"));
    assert!(group.is_collapsed);
}
