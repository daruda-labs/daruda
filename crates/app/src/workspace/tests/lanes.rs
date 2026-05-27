use super::*;

// ---- Lanes (W-2) ----

#[gpui::test]
fn test_workspace_without_project_has_no_lanes(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert!(ws.active_lanes().is_empty());
    });
}

#[gpui::test]
fn test_workspace_with_project_bootstraps_one_default_lane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_bootstrap_wt");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active_lanes().len(), 1);
        assert_eq!(ws.active_lanes()[0].id, 0);
        assert_eq!(ws.active.lane, 0);
        assert_eq!(
            ws.active_lanes()[0].kind,
            daruda_store::project::LaneKind::Default
        );
        assert_eq!(
            ws.active_lanes()[0].path,
            std::path::PathBuf::from("/tmp/test_bootstrap_wt")
        );
    });
}

#[gpui::test]
fn test_activate_lane_requires_existing_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_activate");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
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
            // Nonexistent id → no-op.
            let proj = ws.active_ref().project;
            let mk = |id| daruda_store::project::LaneRef {
                project: proj,
                lane: id,
            };
            ws.activate_lane(mk(99), window, cx);
            assert_eq!(ws.active.lane, 0);
            // Self id → no-op (already active).
            ws.activate_lane(mk(0), window, cx);
            assert_eq!(ws.active.lane, 0);
        });
    })
    .unwrap();
}

#[gpui::test]
fn test_lane_removable_excludes_main_and_default(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |_ws, _| {
        let default_wt =
            crate::lane::Lane::default_for_project(0, std::path::PathBuf::from("/tmp"));
        assert!(!Workspace::lane_removable(&default_wt));

        let main_git = crate::lane::Lane::git(
            0,
            std::path::PathBuf::from("/tmp/repo"),
            Some("main".into()),
            std::path::PathBuf::from("/tmp/repo"),
            std::path::PathBuf::from("/tmp/repo"),
            0,
        );
        assert!(!Workspace::lane_removable(&main_git));

        let linked = crate::lane::Lane::git(
            1,
            std::path::PathBuf::from("/tmp/repo-feat"),
            Some("feat".into()),
            std::path::PathBuf::from("/tmp/repo"),
            std::path::PathBuf::from("/tmp/repo-feat"),
            1,
        );
        assert!(Workspace::lane_removable(&linked));
    });
}

#[gpui::test]
fn test_git_repo_root_returns_none_for_non_git_workspace(cx: &mut TestAppContext) {
    // Non-git Default lane → git_repo_root() is the gate the
    // `[+]` button uses to short-circuit before opening the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_modal_non_git");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert!(ws.git_repo_root().is_none());
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_unknown_id(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_unknown");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 999,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("not found"));
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_default_kind(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_default");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        // Bootstrapped default lane at id 0 — not removable.
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 0,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_validate_remove_lane_rejects_default(cx: &mut TestAppContext) {
    // Default-kind lane → not removable. The `×` click handler in
    // dock/lanes/list.rs uses validate_remove_lane() to
    // decide whether to even open the modal.
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_remove_main_noop");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        let proj = ws.active_ref().project;
        let target = daruda_store::project::LaneRef {
            project: proj,
            lane: 0,
        };
        let err = ws.validate_remove_lane(target).unwrap_err();
        assert!(err.contains("cannot be removed"));
    });
}

#[gpui::test]
fn test_activate_lane_swaps_tabs(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_swap");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Seed a second lane with empty runtime so we can swap into it.
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes.push(crate::lane::Lane::default_for_project(
                1,
                std::path::PathBuf::from("/tmp/test_wt_swap_b"),
            ));
        }
    });

    // Swap to lane 1 → the previous runtime lands in the
    // inactive map, and activate_lane lazy-spawns a new pane
    // rooted at the target lane's path (so the viewport isn't
    // empty).
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(ws.main_area.tabs.len(), 1);
            assert_eq!(ws.main_area.panes.len(), 1);
            let proj = ws.active_ref().project;
            ws.activate_lane(
                daruda_store::project::LaneRef {
                    project: proj,
                    lane: 1,
                },
                window,
                cx,
            );
            assert_eq!(ws.active.lane, 1);
            // Lazy seed: exactly one tab/pane materialized.
            assert_eq!(ws.main_area.tabs.len(), 1);
            assert_eq!(ws.main_area.panes.len(), 1);
            assert_eq!(ws.main_area.inactive_lane_runtimes.len(), 1);
            let proj = ws.active_ref().project;
            let stashed = ws
                .main_area
                .inactive_lane_runtimes
                .get(&daruda_store::project::LaneRef {
                    project: proj,
                    lane: 0,
                })
                .unwrap();
            assert_eq!(stashed.tabs.len(), 1);
            assert_eq!(stashed.panes.len(), 1);
        });
    })
    .unwrap();

    // Swap back → the original runtime rehydrates (same PTY tasks,
    // same pane ids). Lane 1's freshly-spawned runtime parks in
    // the inactive map.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let proj = ws.active_ref().project;
            ws.activate_lane(
                daruda_store::project::LaneRef {
                    project: proj,
                    lane: 0,
                },
                window,
                cx,
            );
            assert_eq!(ws.active.lane, 0);
            assert_eq!(ws.main_area.tabs.len(), 1);
            assert_eq!(ws.main_area.panes.len(), 1);
            let stashed = ws
                .main_area
                .inactive_lane_runtimes
                .get(&daruda_store::project::LaneRef {
                    project: proj,
                    lane: 1,
                })
                .unwrap();
            // Lane 1 carries its lazy-spawned pane now.
            assert_eq!(stashed.tabs.len(), 1);
            assert_eq!(stashed.panes.len(), 1);
        });
    })
    .unwrap();
}

#[gpui::test]
fn finalize_remove_active_lane_keeps_main_area_filled(cx: &mut TestAppContext) {
    // Removing the *active* lane must re-point to a sibling and refill
    // the viewport — never leave the main area blank.
    let config = daruda_config::Config::default();
    let repo = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo");
    let feature = std::path::PathBuf::from("/tmp/daruda_remove_active_lane_repo-feature");
    let _ = std::fs::create_dir_all(&repo);
    let project = daruda_store::project::Project::from_path(&repo);
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Replace the bootstrapped Default lane with a git main lane (id 0,
    // path == repo_root → non-removable) plus a removable linked lane
    // (id 1, a distinct path).
    ws.update(cx, |ws, _| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes = vec![
                crate::lane::Lane::git(
                    0,
                    repo.clone(),
                    Some("main".into()),
                    repo.clone(),
                    repo.clone(),
                    0,
                ),
                crate::lane::Lane::git(
                    1,
                    feature.clone(),
                    Some("feature".into()),
                    repo.clone(),
                    feature.clone(),
                    1,
                ),
            ];
        }
    });

    let proj = ws.read_with(cx, |ws, _| ws.active_ref().project);
    let lane0 = daruda_store::project::LaneRef {
        project: proj,
        lane: 0,
    };
    let lane1 = daruda_store::project::LaneRef {
        project: proj,
        lane: 1,
    };

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.activate_lane(lane1, window, cx);
            assert_eq!(ws.active, lane1, "removable lane is active before removal");
            ws.finalize_remove_lane(lane1, window, cx);
        });
    })
    .unwrap();

    ws.read_with(cx, |ws, _| {
        // Fell back to the surviving main lane.
        assert_eq!(ws.active, lane0);
        let p = ws.active_project().unwrap();
        assert_eq!(p.lanes.len(), 1);
        assert_eq!(p.lanes[0].id, 0);
        // The removed active lane must not blank the viewport.
        assert!(
            !ws.main_area.tabs.is_empty(),
            "main area tabs must survive removing the active lane"
        );
        assert!(!ws.main_area.panes.is_empty());
        // The removed lane's frozen runtime is dropped.
        assert!(
            !ws.main_area.inactive_lane_runtimes.contains_key(&lane1),
            "removed lane's runtime must be cleared"
        );
    });

    let _ = std::fs::remove_dir_all(&repo);
}

// TODO Task 11: rewrite for new schema (uses deleted `save_state`).
#[cfg(any())]
#[gpui::test]
fn test_save_state_serializes_inactive_lane_runtime(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_save_inactive");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();

    // Add a second lane, push into it, swap back — now inactive
    // lane 1 has meta only (no tabs). Verify save captures both
    // without duplication.
    ws.update(cx, |ws, _cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes.push(crate::lane::Lane::default_for_project(
                1,
                std::path::PathBuf::from("/tmp/test_wt_save_inactive_b"),
            ));
        }
    });

    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    let primary = state.primary_project().unwrap();
    assert_eq!(primary.lanes.len(), 2);
    // Active lane 0 has its tab.
    let wt0 = primary.lanes.iter().find(|w| w.id == 0).unwrap();
    assert!(!wt0.tabs.is_empty());
    // Inactive lane 1 has no runtime → serialized tabs empty.
    let wt1 = primary.lanes.iter().find(|w| w.id == 1).unwrap();
    assert!(wt1.tabs.is_empty());
}

// TODO Task 11: rewrite for new schema (uses deleted `restore_state` /
// legacy `ProjectState`).
#[cfg(any())]
#[gpui::test]
fn test_restore_state_rebuilds_all_lanes(cx: &mut TestAppContext) {
    // Create the lane directories so the path-exists check in
    // restore_state doesn't skip the inactive lane layout rebuild.
    std::fs::create_dir_all("/tmp/test_wt_restore_all/main").unwrap();
    std::fs::create_dir_all("/tmp/test_wt_restore_all/side").unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_wt_restore_all");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let make_tab = |pane_id: u64, cwd: &str| daruda_store::project::SerializedTab {
            layout: daruda_store::project::SerializedLayout::Leaf {
                pane_id,
                cwd: Some(std::path::PathBuf::from(cwd)),
                file: None,
            },
            last_focused_pane: pane_id,
            user_label: None,
        };
        let state = daruda_store::project::legacy::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_wt_restore_all"),
            lanes: vec![
                daruda_store::project::SerializedLane {
                    id: 0,
                    kind: daruda_store::project::LaneKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/main"),
                    name: None,
                    tab_order: 0,
                    is_unread: false,
                    last_activity: 0,
                    tabs: vec![make_tab(100, "/tmp/test_wt_restore_all/main")],
                    active_tab_index: 0,
                    base_ref: None,
                    description: None,
                },
                daruda_store::project::SerializedLane {
                    id: 5,
                    kind: daruda_store::project::LaneKind::Default,
                    path: std::path::PathBuf::from("/tmp/test_wt_restore_all/side"),
                    name: Some("Side".into()),
                    tab_order: 1,
                    is_unread: true,
                    last_activity: 0,
                    tabs: vec![
                        make_tab(200, "/tmp/test_wt_restore_all/side"),
                        make_tab(201, "/tmp/test_wt_restore_all/side"),
                    ],
                    active_tab_index: 1,
                    base_ref: None,
                    description: None,
                },
            ],
            active_lane_id: 5,
            active_dock_view: daruda_store::project::LeftDockView::default(),
            active_right_panel_view: daruda_store::project::RightDockView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: Vec::new(),
            active_tab_index: 0,
            focused_pane_id: 201,
            docks: daruda_store::project::DockStates::default(),
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
        };
        let workspace_state = daruda_store::project::legacy::WorkspaceState::from_legacy(state);
        ws.restore_state(&workspace_state, window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        // Active lane is 5 with 2 tabs → Workspace.tabs mirrors it.
        assert_eq!(ws.active.lane, 5);
        assert_eq!(ws.main_area.tabs.len(), 2);
        assert_eq!(ws.main_area.active_tab_index, 1);
        // Inactive lane 0 rebuilt into the inactive map with 1 tab.
        assert_eq!(ws.main_area.inactive_lane_runtimes.len(), 1);
        let proj = ws.active_ref().project;
        let stashed = ws
            .main_area
            .inactive_lane_runtimes
            .get(&daruda_store::project::LaneRef {
                project: proj,
                lane: 0,
            })
            .unwrap();
        assert_eq!(stashed.tabs.len(), 1);
    });
}

// TODO Task 11: rewrite for new schema (uses deleted `save_state`).
#[cfg(any())]
#[gpui::test]
fn test_save_state_captures_bootstrapped_lane(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_wt");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    let state = ws
        .read_with(cx, |ws, app_cx| ws.save_state(app_cx))
        .unwrap();
    let primary = state.primary_project().unwrap();
    assert_eq!(primary.lanes.len(), 1);
    assert_eq!(primary.lanes[0].id, 0);
    assert_eq!(
        primary.lanes[0].path,
        std::path::PathBuf::from("/tmp/test_save_wt")
    );
    assert_eq!(state.active.lane, 0);
    // Tabs live inside the active lane from W-2 onward.
    assert!(!primary.lanes[0].tabs.is_empty());
}

#[gpui::test]
fn test_restore_state_reads_tabs_from_active_lane(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        UsageWindow, WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState,
        WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_wt_tabs");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let lane = daruda_store::project::SerializedLane {
            id: 0,
            kind: daruda_store::project::LaneKind::Default,
            path: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: vec![daruda_store::project::SerializedTab {
                layout: daruda_store::project::SerializedLayout::Leaf {
                    pane_id: 1,
                    cwd: Some(std::path::PathBuf::from("/tmp/test_restore_wt_tabs")),
                    file: None,
                },
                last_focused_pane: 1,
                user_label: None,
            }],
            active_tab_index: 0,
            base_ref: None,
            description: None,
        };
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_wt_tabs"),
            name: None,
            lanes: vec![lane],
            last_active_lane_id: 0,
            next_lane_id: 1,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: Some(0),
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 1,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            active_usage_window: UsageWindow::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.main_area.tabs.len(), 1);
        assert_eq!(ws.active_lanes().len(), 1);
        assert_eq!(ws.active.lane, 0);
    });
}

#[gpui::test]
fn test_restore_state_clamps_stale_active_lane_id(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        UsageWindow, WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState,
        WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_clamp_id");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let lane = daruda_store::project::SerializedLane::default_for_path(
            3,
            std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_clamp_id"),
            name: None,
            lanes: vec![lane],
            last_active_lane_id: 3,
            next_lane_id: 4,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: Some(999), // stale — only id 3 exists
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::default(),
            active_usage_window: UsageWindow::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.active.lane, 3);
    });
}

#[gpui::test]
fn test_dock_view_defaults_to_worktrees(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Lanes
        );
    });
}

#[gpui::test]
fn test_set_dock_view_switches_and_notifies(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
        // No-op when already on that view (shouldn't panic).
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_dock_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_dock_view");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.active_dock_view,
        daruda_store::project::LeftDockView::Files
    );
}

#[gpui::test]
fn test_restore_state_applies_active_dock_view(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        UsageWindow, WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState,
        WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_dock_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_dock_view"),
            name: None,
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: None,
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::GitChanges,
            active_right_panel_view: RightDockView::default(),
            active_usage_window: UsageWindow::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_right_panel_view(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_right_dock_view");
    let wh = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = wh.root(cx).unwrap();
    ws.update(cx, |ws, cx| {
        ws.set_right_dock_view(daruda_store::project::RightDockView::Tools, cx);
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.active_right_panel_view,
        daruda_store::project::RightDockView::Tools
    );
}

#[gpui::test]
fn test_restore_state_applies_active_right_panel_view(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        UsageWindow, WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState,
        WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_right_dock_view");
    let wh = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project_for_test_full(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        );
        let project_uuid = ProjectUuid::new();
        let project_state = ProjectState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: project_uuid,
            root: std::path::PathBuf::from("/tmp/test_restore_right_dock_view"),
            name: None,
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
        };
        let mut project_overrides = BTreeMap::new();
        project_overrides.insert(project_uuid, ProjectOverride::default());
        let workspace_state = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: WorkspaceUuid::new(),
            project_ids: vec![project_uuid],
            project_overrides,
            groups: Vec::new(),
            active_project: Some(project_uuid),
            active_lane: None,
            docks: DockStates::default(),
            window: WindowState::default(),
            font_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            focused_pane_id: 0,
            active_dock_view: LeftDockView::default(),
            active_right_panel_view: RightDockView::Tasks,
            active_usage_window: UsageWindow::default(),
            window_open_policy: WindowOpenPolicy::default(),
            next_group_id: 0,
            project_tabs: BTreeMap::new(),
        };
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = wh.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.right_dock_view,
            daruda_store::project::RightDockView::Tasks
        );
    });
}

#[gpui::test]
fn test_save_state_captures_active_usage_window(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_save_usage_window");
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    let _ = wh.update(cx, |_root, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_usage_window(daruda_store::project::UsageWindow::Last5h, window, cx);
        });
    });
    let (workspace_state, _) = ws
        .read_with(cx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
        .expect("snapshot_for_disk");
    assert_eq!(
        workspace_state.active_usage_window,
        daruda_store::project::UsageWindow::Last5h
    );
    // `set_usage_window` must also keep the dropdown in sync — any
    // future caller that bypasses the Confirm event path (action
    // handler, keyboard shortcut, programmatic restore, …) needs the
    // visible UI to follow `Workspace::usage_window`.
    ws.read_with(cx, |ws, cx_inner| {
        let v = ws.claude.usage_select.read(cx_inner).selected_value();
        assert_eq!(
            v.map(|s| s.as_ref()),
            Some(daruda_store::project::UsageWindow::Last5h.slug())
        );
    });
}

#[gpui::test]
fn test_restore_state_applies_active_usage_window(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        UsageWindow, WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState,
        WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_usage_window");
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    let _ = wh.update(cx, |_root, window, cx| {
        ws.update(cx, |ws, cx| {
            let project_uuid = ProjectUuid::new();
            let project_state = ProjectState {
                schema_version: WORKSPACE_SCHEMA_VERSION,
                uuid: project_uuid,
                root: std::path::PathBuf::from("/tmp/test_restore_usage_window"),
                name: None,
                lanes: Vec::new(),
                last_active_lane_id: 0,
                next_lane_id: 0,
            };
            let mut project_overrides = BTreeMap::new();
            project_overrides.insert(project_uuid, ProjectOverride::default());
            let workspace_state = WorkspaceState {
                schema_version: WORKSPACE_SCHEMA_VERSION,
                uuid: WorkspaceUuid::new(),
                project_ids: vec![project_uuid],
                project_overrides,
                groups: Vec::new(),
                active_project: Some(project_uuid),
                active_lane: None,
                docks: DockStates::default(),
                window: WindowState::default(),
                font_size: 13.0,
                vertical_spacing: 1.0,
                horizontal_spacing: 1.0,
                focused_pane_id: 0,
                active_dock_view: LeftDockView::default(),
                active_right_panel_view: RightDockView::default(),
                active_usage_window: UsageWindow::Last24h,
                window_open_policy: WindowOpenPolicy::default(),
                next_group_id: 0,
                project_tabs: BTreeMap::new(),
            };
            ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        });
    });
    ws.read_with(cx, |ws, cx_inner| {
        assert_eq!(
            ws.claude.usage_window,
            daruda_store::project::UsageWindow::Last24h
        );
        // Picker selection follows the restored state.
        let v = ws.claude.usage_select.read(cx_inner).selected_value();
        assert_eq!(
            v.map(|s| s.as_ref()),
            Some(daruda_store::project::UsageWindow::Last24h.slug())
        );
    });
}

#[gpui::test]
fn test_workspace_renders_with_docks_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.toggle());
        ws.bottom_dock.update(cx, |d, _| d.toggle());
        ws.right_dock.update(cx, |d, _| d.toggle());
        assert!(ws.left_dock.read(cx).is_open);
        assert!(ws.bottom_dock.read(cx).is_open);
        assert!(ws.right_dock.read(cx).is_open);
    });
}
