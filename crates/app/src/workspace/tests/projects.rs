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

    // Give project 0 content that must survive closing the later active project.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.add_tab(window, cx));
    })
    .unwrap();

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

    let keep = cx
        .update_window(wh.into(), |_, window, cx| {
            ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
        })
        .expect("close_active_project window callback succeeded");
    assert!(keep, "closing one of two projects must keep the window");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].id, 0);
        assert_eq!(ws.active.project, 0);
        assert!(
            !ws.active_runtime().tabs.is_empty(),
            "surviving project's own tab must remain after closing a project"
        );
        assert!(
            !ws.active_runtime().panes.is_empty(),
            "main area panes must not be empty after closing a project"
        );
    });
}

#[gpui::test]
fn close_active_project_releases_pane_tracking(cx: &mut TestAppContext) {
    // Closing the last project must clear its workspace state and release every
    // pane it owns from PTY/file-tree tracking.
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
            let active = ws.active;
            let tree = crate::files::tree::FileTree::new(std::path::PathBuf::from(
                "/tmp/daruda_close_release",
            ));
            ws.file_tree.file_trees.insert(active, tree);
            let pane_ids: Vec<_> = ws
                .active_runtime()
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
            assert!(ws.projects.is_empty());
            assert_eq!(ws.active, daruda_store::project::LaneRef::default());
            assert!(
                ws.claude.pty_claude_bindings.is_empty(),
                "closed project's pane bindings must be dropped"
            );
            assert!(
                ws.claude.pty_tracker.tracked_pane_ids().is_empty(),
                "closed project's panes must be unregistered from the tracker"
            );
            assert!(
                !ws.file_tree
                    .file_trees
                    .keys()
                    .any(|k| k.project == active.project),
                "file_trees must drop entries belonging to the closed project"
            );
        });
    })
    .unwrap();
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
        assert!(ws.active_runtime().tabs.is_empty());
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
    });
}

#[gpui::test]
fn open_delete_project_modal_by_id_does_not_change_active(cx: &mut TestAppContext) {
    // Root-wrapped window so `open_form_modal`'s `window.open_dialog`
    // can walk the `gpui_component::Root` layer.
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

/// Build a workspace with project A (id 0) and project B (id 1); `add_project`
/// activates B, so A is the *non-active* project. Both bootstrap lanes share
/// id 0 (lane ids restart per project) — the precondition for the
/// cross-project lane-edit bug.
fn workspace_with_background_project_a(
    cx: &mut TestAppContext,
    a_path: &str,
    b_path: &str,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(a_path);
    std::fs::create_dir_all(a_path).unwrap();
    std::fs::create_dir_all(b_path).unwrap();
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.add_project(std::path::PathBuf::from(b_path), window, cx)
        })
    })
    .unwrap()
    .expect("add_project returned a LaneRef");
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active.project, 1, "project B is active")
    });
    (wh, ws)
}

/// The value of `f` (a lane field accessor) for project `pid`'s lane 0.
fn lane0_field<T>(ws: &Workspace, pid: u64, f: impl FnOnce(&crate::lane::Lane) -> T) -> T {
    f(ws.projects
        .iter()
        .find(|p| p.id == pid)
        .expect("project present")
        .lane(0)
        .expect("lane 0 present"))
}

// A lane edit from the left-dock context menu must target the lane in the
// project the menu was opened for, not the like-id'd lane in whichever project
// is active. Every project has a lane id 0, so a bare `LaneId` routed through
// the active project writes the wrong lane. This covers the shared
// `mutate_lane` helper via each setter.

#[gpui::test]
fn lane_field_setters_target_named_project_not_active(cx: &mut TestAppContext) {
    let (wh, ws) = workspace_with_background_project_a(
        cx,
        "/tmp/daruda_lane_field_scope_a",
        "/tmp/daruda_lane_field_scope_b",
    );
    let a_lane = daruda_store::project::LaneRef {
        project: 0,
        lane: 0,
    };
    let host = daruda_store::project::LaneSessionHost::Ssh {
        target: "vm-a".to_string(),
        session_path: "/data/a".to_string(),
        registry_id: None,
    };
    cx.update_window(wh.into(), |_, _window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_lane_session_host(a_lane, host.clone(), cx);
            ws.set_lane_name(a_lane, Some("renamed-A".to_string()), cx);
            ws.set_lane_description(a_lane, Some("desc-A".to_string()), cx);
        })
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            lane0_field(ws, 0, |l| l.session_host.clone()),
            Some(host),
            "target project A's lane must receive the session host edit"
        );
        assert_eq!(
            lane0_field(ws, 1, |l| l.session_host.clone()),
            None,
            "active project B's like-id lane must keep its session host"
        );
        assert_eq!(
            lane0_field(ws, 0, |l| l.name.clone()),
            Some("renamed-A".to_string()),
            "target project A's lane must receive the rename"
        );
        assert_eq!(
            lane0_field(ws, 1, |l| l.name.clone()),
            None,
            "active project B's like-id lane must keep its name"
        );
        assert_eq!(
            lane0_field(ws, 0, |l| l.description.clone()),
            Some("desc-A".to_string()),
            "target project A's lane must receive the description"
        );
        assert_eq!(
            lane0_field(ws, 1, |l| l.description.clone()),
            None,
            "active project B's like-id lane must keep its description"
        );
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
    // `next_project_id` is a runtime-only (per-session) counter, not
    // persisted; the workspace state references exactly one project.
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.next_project_id, 1);
    });
    assert_eq!(workspace_state.project_ids.len(), 1);
    assert_eq!(project_states.len(), 1);
}

// ---- Group CRUD ----

#[gpui::test]
fn group_crud_round_trips_and_demotes_deleted_members(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_group_crud");
    std::fs::create_dir_all("/tmp/daruda_group_crud").unwrap();
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

    ws.update(cx, |ws, cx| {
        let project_id = ws.projects[0].id;
        ws.move_project_to_group(project_id, Some(id_a), cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects[0].group_id, Some(id_a));
    });
    ws.update(cx, |ws, cx| {
        ws.delete_group(id_a, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.groups.len(), 1);
        assert_eq!(
            ws.projects[0].group_id, None,
            "deleted group must demote member projects"
        );
    });

    ws.update(cx, |ws, cx| {
        assert!(ws.rename_group(id_b, "new".to_string(), cx));
        ws.recolor_group(id_b, Some("#abcdef".into()), cx);
        ws.toggle_group_collapse(id_b, cx);
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    let group = workspace_state
        .groups
        .iter()
        .find(|g| g.id == id_b)
        .unwrap();
    assert_eq!(group.name, "new");
    assert_eq!(group.color.as_deref(), Some("#abcdef"));
    assert!(group.is_collapsed);
}
