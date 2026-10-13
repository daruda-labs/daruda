//! Fixtures shared by more than one topic module.

use std::collections::BTreeMap;
use std::path::PathBuf;

use daruda_store::project::{
    DockStates, LeftDockView, ProjectOverride, ProjectState, ProjectUuid, RightDockView,
    WORKSPACE_SCHEMA_VERSION, WindowState, WorkspaceState, WorkspaceUuid,
};

pub(crate) fn sample_project() -> ProjectState {
    ProjectState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: ProjectUuid::new(),
        root: PathBuf::from("/Users/test/repo"),
        name: Some("repo".into()),
        lanes: vec![],
        last_active_lane_id: Default::default(),
        next_lane_id: Default::default(),
        default_branch: Some("main".into()),
        base_branch: Some("develop".into()),
    }
}

pub(crate) fn sample_workspace(project: ProjectUuid) -> WorkspaceState {
    WorkspaceState {
        schema_version: WORKSPACE_SCHEMA_VERSION,
        uuid: WorkspaceUuid::new(),
        project_ids: vec![project],
        project_overrides: BTreeMap::from([(project, ProjectOverride::default())]),
        groups: vec![],
        session_titles: vec![],
        active_project: Some(project),
        active_lane: Some(Default::default()),
        docks: DockStates::default(),
        window: WindowState {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            ..Default::default()
        },
        font_size: 13.0,
        vertical_spacing: 1.0,
        horizontal_spacing: 1.0,
        focused_pane_id: Default::default(),
        active_dock_view: LeftDockView::default(),
        active_right_panel_view: RightDockView::default(),
        active_page: None,
        task_detail: None,
        flow_detail: None,
        window_open_policy: Default::default(),
        next_group_id: Default::default(),
        project_tabs: BTreeMap::new(),
    }
}
