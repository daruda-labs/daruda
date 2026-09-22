//! Verify `Workspace::snapshot_for_disk` emits the new-shape
//! `(WorkspaceState, Vec<ProjectState>)` output: each runtime project
//! lands as a separate `ProjectState`, every project carries a
//! policy-B `ProjectOverride` entry, the active focus is projected
//! onto UUIDs, and a workspace with no projects still snapshots.

use super::*;
use daruda_store::project::WORKSPACE_SCHEMA_VERSION;

#[gpui::test]
fn snapshot_for_disk_covers_empty_and_emits_project_schema(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, app_cx| {
        // No projects opened. This is the Landing state, which persists like
        // any other — it used to short-circuit to `None` because the window
        // was destroyed instead of surviving.
        let (workspace, projects) = ws.snapshot_for_disk(app_cx);
        assert!(projects.is_empty());
        assert!(workspace.project_ids.is_empty());
        assert_eq!(workspace.schema_version, WORKSPACE_SCHEMA_VERSION);
    });

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/test_snapshot_for_disk");
    let window_handle = cx.add_window(|window, cx| {
        Workspace::new_with_project_for_test(
            &config,
            Some(project),
            fresh_test_data_dir(),
            window,
            cx,
        )
    });
    let ws = window_handle.root(cx).unwrap();

    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
        ws.set_right_dock_view(daruda_store::project::RightDockView::Tasks, cx);
    });

    ws.read_with(cx, |ws, app_cx| {
        let (workspace, projects) = ws.snapshot_for_disk(app_cx);

        // Schema version stamped on both the envelope and every project.
        assert_eq!(workspace.schema_version, WORKSPACE_SCHEMA_VERSION);
        assert_eq!(projects.len(), 1);
        for p in &projects {
            assert_eq!(p.schema_version, WORKSPACE_SCHEMA_VERSION);
        }

        // Policy-B invariant: every project_id has a matching override.
        assert_eq!(workspace.project_ids.len(), projects.len());
        for p in &projects {
            assert!(
                workspace.project_ids.contains(&p.uuid),
                "project_ids must list every ProjectState's uuid"
            );
            assert!(
                workspace.project_overrides.contains_key(&p.uuid),
                "every project_id must have an override entry (policy B)"
            );
            assert!(
                workspace.project_tabs.contains_key(&p.uuid),
                "every project_id must have a project_tabs entry"
            );
        }

        // Active focus projected onto UUIDs — the single project IS the
        // active one because `new_with_project` makes it so.
        let active_uuid = workspace.active_project.expect("active project");
        assert_eq!(active_uuid, projects[0].uuid);
        assert_eq!(workspace.active_lane, Some(ws.active.lane));
        assert_eq!(
            workspace.active_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
        assert_eq!(
            workspace.active_right_panel_view,
            daruda_store::project::RightDockView::Tasks
        );

        // ProjectState carries intrinsic fields from the runtime Project.
        assert_eq!(projects[0].root, ws.projects[0].root);
        assert_eq!(projects[0].name, Some(ws.projects[0].name.clone()));
        // First-touch lanes: ids start at 0, so next_lane_id is
        // one past the max live id.
        let max_live = ws.projects[0].lanes.iter().map(|w| w.id).max().unwrap();
        assert_eq!(projects[0].next_lane_id, max_live + 1);
    });
}
