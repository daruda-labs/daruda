use super::*;

// ---- empty workspace as a first-class persisted state (Landing) ----

/// A workspace that has lost its last project must still be saveable, and
/// must come back empty. Before Landing this returned `None` and the window
/// was destroyed instead, so the state had nowhere to round-trip through.
#[gpui::test]
fn empty_workspace_snapshots_and_restores(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    std::fs::create_dir_all("/tmp/daruda_empty_round_trip").unwrap();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_empty_round_trip");
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

    // Close the only project — the workspace goes empty but stays alive.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.projects.is_empty(), "the only project is gone");
    });

    let (workspace_state, project_states) =
        ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));
    assert!(project_states.is_empty());
    assert!(workspace_state.project_ids.is_empty());

    // Restore into a fresh workspace: it must accept the empty payload.
    let restored_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx);
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();
    restored.read_with(cx, |ws, _| {
        assert!(ws.projects.is_empty(), "restored workspace stays empty");
        assert_eq!(
            ws.uuid, workspace_state.uuid,
            "the empty workspace keeps its identity, so the next launch finds it"
        );
        assert!(
            ws.has_no_projects(),
            "the accessor the open-folder path reads must agree"
        );
    });
}

/// Closing the last project lands the workspace in the normalized empty
/// state `render` needs to paint Landing.
///
/// Scope note: the window teardown this branch removed lived at the three
/// *call sites* (`bind_keys`'s CloseProject modal callback and the two in
/// `project_ops`), never in this method — so the surviving window handle is
/// asserted here only as a precondition, and those callbacks remain
/// uncovered because reaching them means driving a real dialog.
#[gpui::test]
fn closing_the_only_project_lands_in_the_empty_state(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    std::fs::create_dir_all("/tmp/daruda_last_project_survives").unwrap();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_last_project_survives");
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
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .unwrap();

    assert!(wh.root(cx).is_ok(), "precondition: the window is still up");
    ws.read_with(cx, |ws, _| {
        assert!(ws.has_no_projects());
        // The "active runtime always present" invariant has to survive the
        // teardown, because `render` reads it to paint Landing.
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
        assert!(ws.active_runtime().tabs.is_empty());
    });
}

/// The recent list after a workspace empties out: the row stays (that is
/// how the next launch finds the workspace) but stops naming the project
/// that left, and no second row appears. Covers the `persist_state` branch
/// — the round-trip test above cannot see it, because it never writes.
#[gpui::test]
fn emptying_a_workspace_refreshes_its_recent_row_without_adding_one(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let root = std::env::temp_dir().join("daruda_recent_refresh");
    std::fs::create_dir_all(&root).unwrap();

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, Some(project), data_dir.clone(), window, cx)
    });
    let ws = wh.root(cx).unwrap();

    // Persist while it still holds a project: a normal touch, so this
    // workspace takes the top slot with its project's name.
    ws.read_with(cx, |w, cx| w.persist_state(cx));
    let uuid = ws.read_with(cx, |w, _| w.uuid);

    // Another workspace is then opened, taking the top slot. Ordering is
    // what separates a refresh from a touch: with a plain `touch_recent_in`
    // the step below would promote this workspace back to the front, which
    // is exactly what going empty must not do.
    let other = daruda_store::project::WorkspaceUuid::new();
    daruda_store::project::touch_recent_in(&data_dir, other, "other".into()).unwrap();
    let before = daruda_store::project::load_recent_in(&data_dir);
    assert_eq!(before.len(), 2);
    assert_eq!(before[0].workspace_uuid, other);
    assert_eq!(before[1].workspace_uuid, uuid);
    assert_eq!(before[1].display_name, "daruda_recent_refresh");

    // Close the only project, then persist the now-empty workspace.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .unwrap();
    ws.read_with(cx, |w, cx| w.persist_state(cx));

    let after = daruda_store::project::load_recent_in(&data_dir);
    assert_eq!(after.len(), 2, "an empty workspace must not add a row");
    assert_eq!(
        after[0].workspace_uuid, other,
        "going empty must not promote the row over the workspace last worked in"
    );
    assert_eq!(
        after[1].workspace_uuid, uuid,
        "the row is refreshed in place, not removed"
    );
    assert_eq!(
        after[1].display_name,
        crate::surface::strings::menu_recent_empty_workspace(),
        "the row must stop naming a project the workspace no longer holds"
    );
}

/// A workspace that never held a project earns no recent row at all — the
/// New Empty Window case. Without this, every such window would push a
/// nameless row that `RECENT_MAX` eventually spends on nothing.
#[gpui::test]
fn a_workspace_that_never_held_a_project_earns_no_recent_row(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let config = daruda_config::Config::default();

    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, None, data_dir.clone(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |w, cx| w.persist_state(cx));

    assert!(
        daruda_store::project::load_recent_in(&data_dir).is_empty(),
        "an empty workspace must not appear in the recent list"
    );
    // A state file exists only if something can reach it, and the recent
    // list is the only index there is. The production constructor persists
    // before `restore_from_disk` adopts the saved uuid, so without this a
    // junk file would accumulate on every launch and every Open Recent.
    let count = daruda_store::project::workspaces_dir_in(&data_dir)
        .read_dir()
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(
        count, 0,
        "a workspace nothing can reach must leave no state file"
    );
}

/// One persisted workspace, one window. A `WorkspaceUuid` keys a single
/// `workspaces/<uuid>.json`, so a second window onto the same record would
/// let the last save win — an emptied window erasing the projects the other
/// still holds. The open paths resolve the uuid to the live window first.
#[gpui::test]
fn a_workspace_uuid_resolves_to_the_window_already_holding_it(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root = std::env::temp_dir().join("daruda_uuid_dedup");
    std::fs::create_dir_all(&root).unwrap();
    let project = daruda_store::project::Project::from_path(&root);

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
    // The test constructor skips registration; the lookup reads the registry.
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    let uuid = ws.read_with(cx, |w, _| w.uuid);

    cx.update(|cx| {
        assert_eq!(
            crate::window_registry::WindowRegistry::workspace_window_for_uuid(uuid, cx),
            Some(wh.into()),
            "an open workspace must be found by its persisted identity"
        );
        assert_eq!(
            crate::window_registry::WindowRegistry::workspace_window_for_uuid(
                daruda_store::project::WorkspaceUuid::new(),
                cx
            ),
            None,
            "a uuid no window holds must not resolve to some other window"
        );
    });
}

/// The reachable half of the same invariant: an emptied workspace that
/// still owns a recent row keeps its file, because that is how the next
/// launch finds it.
#[gpui::test]
fn an_emptied_workspace_with_a_recent_row_keeps_its_state_file(cx: &mut TestAppContext) {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let root = std::env::temp_dir().join("daruda_reachable_keeps_file");
    std::fs::create_dir_all(&root).unwrap();

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(&root);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(&config, Some(project), data_dir.clone(), window, cx)
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |w, cx| w.persist_state(cx));
    let uuid = ws.read_with(cx, |w, _| w.uuid);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .unwrap();
    ws.read_with(cx, |w, cx| w.persist_state(cx));

    assert!(
        daruda_store::project::load_workspace_state_in(&data_dir, uuid).is_some(),
        "an emptied workspace the recent list still names must keep its file"
    );
}

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
        assert_eq!(
            ws.window_open_policy(),
            daruda_store::project::WindowOpenPolicy::Ask
        );
    });
    ws.update(cx, |ws, cx| {
        ws.set_window_open_policy(daruda_store::project::WindowOpenPolicy::NewWindow, cx);
    });
    let (workspace_state, project_states) =
        ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));
    assert_eq!(
        workspace_state.window_open_policy,
        daruda_store::project::WindowOpenPolicy::NewWindow
    );
    // `next_project_id` is a runtime-only (per-session) counter, not persisted;
    // the workspace state references exactly one project before the add path.
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.next_project_id, 1);
    });
    assert_eq!(workspace_state.project_ids.len(), 1);
    assert_eq!(project_states.len(), 1);

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

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .expect("close_active_project window callback succeeded");
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
            ws.lane_scoped_mut(active).files.tree = Some(tree);
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

            ws.close_active_project(window, cx);

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
                !ws.lane_file_tree_refs()
                    .any(|k| k.project == active.project),
                "file trees must drop entries belonging to the closed project"
            );
        });
    })
    .unwrap();
}

/// `HistoryBuffer` has no cap and `finalize_remove_lane` is the only other
/// place anything prunes it, so a project closed with history behind it would
/// hold every line for the rest of the session.
#[gpui::test]
fn close_active_project_drops_the_input_history_of_its_lanes(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let root = "/tmp/daruda_close_input_history";
    std::fs::create_dir_all(root).unwrap();
    let project = daruda_store::project::Project::from_path(root);
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
            let active = ws.active;
            ws.lane_scoped_mut(active).input_history.push("cargo test");
            assert!(ws.lane_scoped[&active].input_history.has_entries());

            ws.close_active_project(window, cx);

            assert!(
                !ws.lane_scoped.keys().any(|k| k.project == active.project),
                "scoped state, including input history, must drop with the closed project"
            );
        });
    })
    .unwrap();
}

#[gpui::test]
fn close_active_project_empties_the_workspace_when_no_survivor_has_a_lane(cx: &mut TestAppContext) {
    // Safety net: if every surviving project is somehow lane-less
    // (runtime corruption), closing the active project must treat the
    // workspace as empty — reset to the Landing state rather than leaving
    // a blank viewport open.
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
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.close_active_project(window, cx))
    })
    .unwrap();
    assert!(
        wh.root(cx).is_ok(),
        "the window survives a workspace with no usable lane"
    );
    ws.read_with(cx, |ws, _| {
        // Live runtime is cleared and the active ref is back at the default
        // seed, which is what `render` needs to paint Landing.
        assert!(ws.active_runtime().tabs.is_empty());
        assert_eq!(ws.active, daruda_store::project::LaneRef::default());
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
    let active_before = ws.read_with(cx, |ws, _| ws.active);

    // Opening the delete chooser for the non-active project must not force
    // activation onto it; activation happens only after confirmation.
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
    let (workspace_state, _) = ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));
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
    let (workspace_state, _) = ws.read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx));
    let group = workspace_state
        .groups
        .iter()
        .find(|g| g.id == id_b)
        .unwrap();
    assert_eq!(group.name, "new");
    assert_eq!(group.color.as_deref(), Some("#abcdef"));
    assert!(group.is_collapsed);
}
