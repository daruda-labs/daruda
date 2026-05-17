use super::*;

// ---- Project + persistence ----

#[gpui::test]
fn test_workspace_project_field(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, _| {
        // Default config creates workspace without project.
        assert!(ws.project.is_none());
    });
}

#[gpui::test]
fn test_save_state_returns_none_without_project(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, app_cx| {
        assert!(ws.save_state(app_cx).is_none());
    });
}

#[gpui::test]
fn test_save_state_with_project(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_project");
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, app_cx| {
        let state = ws.save_state(app_cx).unwrap();
        assert_eq!(state.root, std::path::PathBuf::from("/tmp/test_project"));
        // Tabs now live inside the active worktree.
        assert!(state.tabs.is_empty());
        assert_eq!(state.worktrees.len(), 1);
        assert!(!state.worktrees[0].tabs.is_empty());
        assert_eq!(state.font_size, config.font.size);
    });
}

#[gpui::test]
fn test_restore_state_applies_dock_sizes(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore");
    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        let state = daruda_store::project::ProjectState {
            root: std::path::PathBuf::from("/tmp/test_restore"),
            worktrees: Vec::new(),
            active_worktree_id: 0,
            active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
            active_right_panel_view: daruda_store::project::RightSidebarView::default(),
            active_usage_window: daruda_store::project::UsageWindow::default(),
            tabs: vec![],
            active_tab_index: 0,
            focused_pane_id: 0,
            docks: daruda_store::project::DockStates {
                left_open: true,
                left_size: 300.0,
                bottom_open: true,
                bottom_size: 180.0,
                right_open: false,
                right_size: 0.0,
            },
            window: daruda_store::project::WindowState::default(),
            window_user_label: None,
            font_size: 16.0,
            vertical_spacing: 1.2,
            horizontal_spacing: 1.0,
        };
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, cx| {
        assert!(ws.left_dock.read(cx).is_open);
        assert_eq!(ws.left_dock.read(cx).size, 300.0);
        assert!(ws.bottom_dock.read(cx).is_open);
        assert_eq!(ws.bottom_dock.read(cx).size, 180.0);
        assert!(!ws.right_dock.read(cx).is_open);
        assert_eq!(ws.terminal_config.font_size, 16.0);
        assert!((ws.terminal_config.vertical_spacing - 1.2).abs() < f32::EPSILON);
    });
}

#[gpui::test]
fn test_persist_state_creates_file(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project_dir = std::env::temp_dir().join("daruda_test_persist_proj");
    let data_dir = fresh_test_data_dir();
    let _ = std::fs::create_dir_all(&project_dir);
    let project = daruda_store::project::Project::from_path(&project_dir);
    // data_dir injected at construction; new_with_project calls persist_state()
    // automatically so the state file is already on disk after this line.
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), data_dir.clone(), window, cx)
    });
    let ws = window_handle.root(cx).unwrap();

    let state = ws.read_with(cx, |ws, app_cx| ws.save_state(app_cx));
    assert!(state.is_some());

    // Verify the file landed in the isolated data dir (not the real app dir).
    let loaded = daruda_store::project::persistence::load_state_in(&data_dir, &project_dir);
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().root, project_dir);

    let _ = std::fs::remove_dir_all(&data_dir);
    let _ = std::fs::remove_dir_all(&project_dir);
}

#[gpui::test]
fn test_save_state_serializes_leaf_layout(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_layout");
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx)
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, app_cx| {
        let state = ws.save_state(app_cx).unwrap();
        // Tabs moved onto the active worktree in W-2.
        assert_eq!(state.worktrees.len(), 1);
        let wt_tabs = &state.worktrees[0].tabs;
        assert_eq!(wt_tabs.len(), 1);
        match &wt_tabs[0].layout {
            daruda_store::project::SerializedLayout::Leaf { .. } => {}
            _ => panic!("expected Leaf layout for single-pane tab"),
        }
    });
}

// ---- restore_state — layout rebuild ----

#[gpui::test]
fn test_restore_state_rebuilds_horizontal_split(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, ProjectState, SerializedLayout, SerializedTab, SplitDirectionSerde, WindowState,
    };
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_split");
    let split_tab = SerializedTab {
        layout: SerializedLayout::Split {
            direction: SplitDirectionSerde::Horizontal,
            children: vec![
                SerializedLayout::Leaf {
                    pane_id: 10,
                    cwd: None,
                    file: None,
                },
                SerializedLayout::Leaf {
                    pane_id: 11,
                    cwd: None,
                    file: None,
                },
            ],
            ratios: vec![0.3, 0.7],
        },
        last_focused_pane: 11,
        user_label: None,
    };
    let state = ProjectState {
        root: std::path::PathBuf::from("/tmp/test_restore_split"),
        worktrees: vec![daruda_store::project::SerializedWorktree {
            id: 0,
            kind: daruda_store::project::WorktreeKind::Default,
            path: std::path::PathBuf::from("/tmp/test_restore_split"),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: vec![split_tab],
            active_tab_index: 0,
            base_ref: None,
            description: None,
        }],
        active_worktree_id: 0,
        active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
        active_right_panel_view: daruda_store::project::RightSidebarView::default(),
        active_usage_window: daruda_store::project::UsageWindow::default(),
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 11,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };

    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.main_area.tabs.len(), 1);
        assert_eq!(ws.main_area.panes.len(), 2);
        assert_eq!(ws.main_area.tabs[0].layout.leaf_count(), 2);
        // Both panes in the active tab should be registered.
        let ids = ws.main_area.tabs[0].layout.pane_ids();
        assert_eq!(ids.len(), 2);
        // Focused pane must be one of the restored leaves.
        assert!(ids.contains(&ws.main_area.focused_pane_id));
    });
}

#[gpui::test]
fn test_restore_state_rebuilds_multiple_tabs(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, ProjectState, SerializedLayout, SerializedTab, WindowState,
    };
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_tabs");
    let legacy_tabs = vec![
        SerializedTab {
            layout: SerializedLayout::Leaf {
                pane_id: 1,
                cwd: None,
                file: None,
            },
            last_focused_pane: 1,
            user_label: None,
        },
        SerializedTab {
            layout: SerializedLayout::Leaf {
                pane_id: 2,
                cwd: None,
                file: None,
            },
            last_focused_pane: 2,
            user_label: None,
        },
        SerializedTab {
            layout: SerializedLayout::Leaf {
                pane_id: 3,
                cwd: None,
                file: None,
            },
            last_focused_pane: 3,
            user_label: None,
        },
    ];
    let state = ProjectState {
        root: std::path::PathBuf::from("/tmp/test_restore_tabs"),
        worktrees: vec![daruda_store::project::SerializedWorktree {
            id: 0,
            kind: daruda_store::project::WorktreeKind::Default,
            path: std::path::PathBuf::from("/tmp/test_restore_tabs"),
            name: None,
            tab_order: 0,
            is_unread: false,
            last_activity: 0,
            tabs: legacy_tabs,
            active_tab_index: 2,
            base_ref: None,
            description: None,
        }],
        active_worktree_id: 0,
        active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
        active_right_panel_view: daruda_store::project::RightSidebarView::default(),
        active_usage_window: daruda_store::project::UsageWindow::default(),
        tabs: Vec::new(),
        active_tab_index: 0,
        focused_pane_id: 3,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };

    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.main_area.tabs.len(), 3);
        assert_eq!(ws.main_area.panes.len(), 3);
        // Active tab index preserved.
        assert_eq!(ws.main_area.active_tab_index, 2);
        // Focused pane is a valid leaf of the active tab.
        let active_leaves = ws.main_area.tabs[2].layout.pane_ids();
        assert!(active_leaves.contains(&ws.main_area.focused_pane_id));
    });
}

#[gpui::test]
fn test_restore_state_clamps_out_of_range_active_tab(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, ProjectState, SerializedLayout, SerializedTab, WindowState,
    };
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_clamp");
    let state = ProjectState {
        root: std::path::PathBuf::from("/tmp/test_restore_clamp"),
        worktrees: Vec::new(),
        active_worktree_id: 0,
        active_sidebar_view: daruda_store::project::LeftSidebarView::default(),
        active_right_panel_view: daruda_store::project::RightSidebarView::default(),
        active_usage_window: daruda_store::project::UsageWindow::default(),
        tabs: vec![SerializedTab {
            layout: SerializedLayout::Leaf {
                pane_id: 1,
                cwd: None,
                file: None,
            },
            last_focused_pane: 1,
            user_label: None,
        }],
        active_tab_index: 99, // way out of range
        focused_pane_id: 1,
        docks: DockStates::default(),
        window: WindowState::default(),
        window_user_label: None,
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
    };

    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        ws.restore_state(&state, window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();
    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.main_area.active_tab_index, 0);
    });
}

#[gpui::test]
fn test_save_restore_round_trip_preserves_layout(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_restore_roundtrip");
    // Build a workspace with one split and one additional tab, then round-trip.
    let window_handle = cx.add_window(|window, cx| {
        let mut ws = Workspace::new_with_project(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        );
        ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        ws.add_tab(window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();

    // Capture the serialized shape. Tabs now live on the active worktree.
    let original = ws.read_with(cx, |ws, app_cx| ws.save_state(app_cx).unwrap());
    assert_eq!(original.worktrees.len(), 1);
    assert_eq!(original.worktrees[0].tabs.len(), 2);

    // Rebuild into a fresh workspace and verify topology matches.
    let window_handle2 = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project(&config, Some(project), fresh_test_data_dir(), window, cx);
        ws.restore_state(&original, window, cx);
        ws
    });
    let ws2 = window_handle2.root(cx).unwrap();
    ws2.read_with(cx, |ws, _| {
        assert_eq!(ws.main_area.tabs.len(), 2);
        // Tab 0 had a horizontal split → 2 panes.
        assert_eq!(ws.main_area.tabs[0].layout.leaf_count(), 2);
        assert_eq!(ws.main_area.tabs[1].layout.leaf_count(), 1);
        assert_eq!(ws.main_area.panes.len(), 3);
    });
}
