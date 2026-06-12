//! Round-trip verification: `snapshot_for_disk` → `restore_from_disk`
//! preserves the workspace's UUID-keyed identity (workspace UUID,
//! project UUIDs, project roots, active focus). Tab/pane structure is
//! exercised by the legacy `restore_state` tests; this file focuses on
//! the new-shape projection so a future caller swap can rely on the
//! parallel method staying behaviorally equivalent for the bits the
//! new schema actually carries.

use super::*;

#[gpui::test]
fn snapshot_then_restore_preserves_identity(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project_root = std::path::PathBuf::from("/tmp/test_restore_from_disk");
    let _ = std::fs::create_dir_all(&project_root);
    let project = daruda_store::project::Project::from_path(&project_root);

    // 1) Build a populated workspace and snapshot it.
    let original_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project.clone()),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let original = original_handle.root(cx).unwrap();

    let (workspace_state, project_states, original_uuid, original_project_uuid) = original
        .read_with(cx, |ws, app_cx| {
            let (ws_state, p_states) = ws.snapshot_for_disk(app_cx).expect("snapshot");
            (ws_state, p_states, ws.uuid, ws.projects[0].uuid)
        });

    // 2) Build a fresh empty workspace and restore into it.
    let restored_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx);
        ws.restore_from_disk(&workspace_state, &project_states, window, cx);
        ws
    });
    let restored = restored_handle.root(cx).unwrap();

    restored.read_with(cx, |ws, _| {
        assert_eq!(ws.uuid, original_uuid, "workspace uuid must round-trip");
        assert_eq!(ws.projects.len(), 1, "one project restored");
        assert_eq!(
            ws.projects[0].uuid, original_project_uuid,
            "project uuid must round-trip"
        );
        assert_eq!(ws.projects[0].root, project_root);
        assert_eq!(ws.active.project, ws.projects[0].id);
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
fn restore_rebootstraps_project_with_empty_persisted_lanes(cx: &mut TestAppContext) {
    // Regression: a persisted project whose lane list is empty
    // (`worktrees: []` — a corrupt / interrupted save proven to exist
    // in production) must re-discover its lanes on restore. Restoring
    // it verbatim leaves a 0-lane project, which blanks the left dock
    // and — on project close — the main area.
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project_root = std::env::temp_dir().join("daruda_restore_rebootstrap_empty");
    let _ = std::fs::create_dir_all(&project_root);

    let project_uuid = ProjectUuid::new();
    let workspace_uuid = WorkspaceUuid::new();

    let project_state = ProjectState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: project_uuid,
        root: project_root.clone(),
        name: Some("empty_lanes".to_string()),
        lanes: Vec::new(),
        last_active_lane_id: 0,
        next_lane_id: 0,
        default_branch: None,
        base_branch: None,
    };

    let mut project_overrides = BTreeMap::new();
    project_overrides.insert(project_uuid, ProjectOverride::default());

    let workspace_state = WorkspaceState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: workspace_uuid,
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
        window_open_policy: WindowOpenPolicy::default(),
        next_group_id: 0,
        project_tabs: BTreeMap::new(),
    };

    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx);
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.projects.len(), 1);
        assert!(
            !ws.projects[0].lanes.is_empty(),
            "empty persisted lanes must re-bootstrap on restore"
        );
        // Active focus must land on a real lane of the project.
        assert_eq!(ws.active.project, ws.projects[0].id);
        assert!(
            ws.projects[0].lanes.iter().any(|l| l.id == ws.active.lane),
            "active lane must be a real member of the project"
        );
        // The viewport must not be blank.
        assert!(
            !ws.main_area.tabs.is_empty(),
            "restored workspace must have at least one tab"
        );
    });

    let _ = std::fs::remove_dir_all(&project_root);
}

#[gpui::test]
fn restore_into_empty_workspace_applies_dock_state(cx: &mut TestAppContext) {
    use daruda_store::project::{
        DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
        WORKSPACE_SCHEMA_VERSION, WindowOpenPolicy, WindowState, WorkspaceState, WorkspaceUuid,
    };
    use std::collections::BTreeMap;

    let config = daruda_config::Config::default();
    let project_root = std::path::PathBuf::from("/tmp/test_restore_from_disk_dock");
    let _ = std::fs::create_dir_all(&project_root);

    let project_uuid = ProjectUuid::new();
    let workspace_uuid = WorkspaceUuid::new();

    let project_state = ProjectState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: project_uuid,
        root: project_root.clone(),
        name: Some("test_restore".to_string()),
        lanes: Vec::new(),
        last_active_lane_id: 0,
        next_lane_id: 0,
        default_branch: None,
        base_branch: None,
    };

    let mut project_overrides = BTreeMap::new();
    project_overrides.insert(
        project_uuid,
        ProjectOverride {
            color: None,
            tab_order: 0,
            group_id: None,
            is_collapsed: false,
        },
    );

    let workspace_state = WorkspaceState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: workspace_uuid,
        project_ids: vec![project_uuid],
        project_overrides,
        groups: Vec::new(),
        active_project: Some(project_uuid),
        active_lane: None,
        docks: DockStates {
            left_open: true,
            left_size: 321.0,
            bottom_open: true,
            bottom_size: 199.0,
            right_open: false,
            right_size: 0.0,
        },
        window: WindowState::default(),
        font_size: 17.0,
        vertical_spacing: 1.25,
        horizontal_spacing: 1.0,
        focused_pane_id: 0,
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        window_open_policy: WindowOpenPolicy::default(),
        next_group_id: 0,
        project_tabs: BTreeMap::new(),
    };

    let window_handle = cx.add_window(|window, cx| {
        let mut ws =
            Workspace::new_with_project_for_test(&config, None, fresh_test_data_dir(), window, cx);
        ws.restore_from_disk(&workspace_state, &[project_state], window, cx);
        ws
    });
    let ws = window_handle.root(cx).unwrap();

    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.uuid, workspace_uuid);
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].uuid, project_uuid);
        assert_eq!(ws.projects[0].name, "test_restore");
        assert!(ws.left_dock.read(cx).is_open);
        assert_eq!(ws.left_dock.read(cx).size, 321.0);
        assert!(ws.bottom_dock.read(cx).is_open);
        assert_eq!(ws.bottom_dock.read(cx).size, 199.0);
        assert!(!ws.right_dock.read(cx).is_open);
        assert_eq!(ws.terminal_config.font_size, 17.0);
        assert!((ws.terminal_config.vertical_spacing - 1.25).abs() < f32::EPSILON);
    });

    let _ = std::fs::remove_dir_all(&project_root);
}
