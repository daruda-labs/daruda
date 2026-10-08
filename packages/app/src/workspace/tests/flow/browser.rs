//! Flow browsing never borrows the active worktree as an action target.

use super::*;
use crate::workspace::{
    flow_browser::{FlowGrouping, FlowScope, FlowTab, RunFilter},
    flow_paths,
    pages::Page,
};
use daruda_store::project::LaneRef;
use gpui::{Modifiers, VisualTestContext};

// GPUI's test selector API requires static strings, including path-based ids.
fn selector(prefix: &str, path: &std::path::Path) -> &'static str {
    Box::leak(format!("{prefix}{}", path.display()).into_boxed_str())
}

fn other_project(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
    flow: &str,
) -> (tempfile::TempDir, LaneRef, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let flows = flow_paths::flows_dir(dir.path());
    std::fs::create_dir_all(&flows).unwrap();
    let path = flows.join("other.yaml");
    std::fs::write(&path, flow).unwrap();
    let target = ws.update(cx, |ws, _| {
        let id = ws.next_project_id;
        ws.next_project_id += 1;
        let project = crate::project::Project::bootstrap_placeholder(id, dir.path().to_path_buf());
        let target = LaneRef {
            project: id,
            lane: project.lanes[0].id,
        };
        ws.projects.push(project);
        ws.main_area.runtimes.entry(target).or_default();
        target
    });
    (dir, target, path)
}

#[gpui::test]
async fn flow_browser_scope_filters_without_switching_the_active_worktree(cx: &mut TestAppContext) {
    let (_dir, ws, original_file, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (_other, target, other_file) = other_project(&ws, cx, ONE_AGENT);
    let active = ws.read_with(cx, |ws, _| ws.active);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.set_flow_scope(FlowScope::Worktree(target), cx);
        let listed = ws.flow_list_for_panel();
        assert!(listed.iter().any(|file| file.path == other_file));
        assert!(!listed.iter().any(|file| file.path == original_file));
        assert_eq!(ws.active, active);
    });
    vcx.run_until_parked();
    let source = vcx
        .debug_bounds("flow-origin-1")
        .expect("repository filter");
    vcx.simulate_click(source.center(), Modifiers::default());
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(
            ws.flows.browser.state.origin,
            Some(flow_paths::FlowOrigin::Repo)
        );
        assert_eq!(ws.active, active);
        assert_eq!(ws.flow_browser_lane(), target);
    });
    let clear = vcx.debug_bounds("flow-clear-filters").unwrap();
    vcx.simulate_click(clear.center(), Modifiers::default());
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.flows.browser.state.origin, None);
        assert_eq!(ws.flow_browser_lane(), target);
        assert_eq!(ws.active, active);
    });
}

#[gpui::test]
async fn flow_browser_run_button_keeps_its_target_through_profile_selection(
    cx: &mut TestAppContext,
) {
    let (_dir, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (_other, target, path) = other_project(&ws, cx, WITH_PROFILES);
    let active = ws.read_with(cx, |ws, _| ws.active);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    vcx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.set_flow_scope(FlowScope::Worktree(target), cx);
    });
    vcx.run_until_parked();
    let button = vcx
        .debug_bounds(selector("flow-run-", &path))
        .expect("run button");
    vcx.simulate_click(button.center(), Modifiers::default());
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.active, active);
        assert_eq!(ws.active_page(), Some(Page::Flows), "Run did not open the graph");
        assert!(matches!(ws.overlays.flow_picker.focused_pick(),
            Some(crate::workspace::command::flow_picker::FlowPick::Profile { lane, .. }) if lane == target));
    });
}

#[gpui::test]
async fn flow_browser_creation_writes_only_to_the_selected_project(cx: &mut TestAppContext) {
    let (_dir, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (_other, target, _path) = other_project(&ws, cx, ONE_AGENT);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        let original_root = ws.active_project().unwrap().root.clone();
        let target_root = ws.project_for(target.project).unwrap().root.clone();
        let wrong = flow_paths::project_flows_dir(&ws.data_dir, &original_root);
        let right = flow_paths::project_flows_dir(&ws.data_dir, &target_root);
        ws.set_flow_scope(FlowScope::Worktree(target), cx);
        ws.create_flow_in(target, "scoped", window, cx);
        assert!(right.is_dir());
        assert!(!wrong.exists());
        assert_eq!(
            ws.active, target,
            "explicit editing enters the target worktree"
        );
        assert!(ws.active_runtime().panes.iter().any(|pane| {
            pane.flow_graph_content()
                .is_some_and(|graph| graph.path.parent() == Some(right.as_path()))
        }));
    });
}

#[gpui::test]
async fn flow_browser_tabs_retain_separate_queries_and_scroll_offsets(cx: &mut TestAppContext) {
    let (_dir, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.flows.browser.searches[0].update(cx, |input, cx| input.set_value("ship", window, cx));
        ws.flows.browser.scrolls[0].set_offset(gpui::point(gpui::px(0.0), gpui::px(-80.0)));
        ws.set_flow_tab(FlowTab::Runs, cx);
        ws.flows.browser.searches[1].update(cx, |input, cx| input.set_value("other", window, cx));
        ws.flows.browser.scrolls[1].set_offset(gpui::point(gpui::px(0.0), gpui::px(-20.0)));
        ws.set_flow_tab(FlowTab::Definitions, cx);
        assert_eq!(ws.flow_browser_snapshot(cx).query, "ship");
        assert_eq!(
            ws.workspace_page.as_ref().unwrap().scroll.offset().y,
            gpui::px(-80.0)
        );
        ws.close_page(cx);
        ws.open_page(Page::Flows, window, cx);
        assert_eq!(
            ws.workspace_page.as_ref().unwrap().scroll.offset().y,
            gpui::px(-80.0)
        );
        ws.set_flow_tab(FlowTab::Runs, cx);
        assert_eq!(ws.flow_browser_snapshot(cx).query, "other");
        assert_eq!(
            ws.workspace_page.as_ref().unwrap().scroll.offset().y,
            gpui::px(-20.0)
        );
    });
}

#[gpui::test]
async fn flow_browser_table_alignment_and_grouping_survive_narrow_windows(cx: &mut TestAppContext) {
    let (_dir, ws, path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.set_flow_grouping(FlowGrouping::Origin, cx);
    });
    vcx.run_until_parked();
    for width in [800.0, 1280.0] {
        vcx.simulate_resize(gpui::size(gpui::px(width), gpui::px(800.0)));
        vcx.run_until_parked();
        let heading = vcx.debug_bounds("flow-source-column").unwrap();
        let source = vcx.debug_bounds(selector("flow-source-", &path)).unwrap();
        let title = vcx.debug_bounds(selector("flow-title-", &path)).unwrap();
        assert_eq!(heading.left(), source.left());
        assert!(title.right() <= source.left());
        assert!(title.size.width >= gpui::px(crate::ui::theme::list_metrics::TITLE_MIN_W));
    }
    for open in [false, true] {
        let group = vcx.debug_bounds("flow-group-Repo").unwrap();
        vcx.simulate_click(group.center(), Modifiers::default());
        vcx.run_until_parked();
        assert_eq!(
            vcx.debug_bounds(selector("flow-file-", &path)).is_some(),
            open
        );
    }
    let file = vcx.debug_bounds(selector("flow-title-", &path)).unwrap();
    vcx.simulate_click(file.center(), Modifiers::default());
    vcx.run_until_parked();
    assert_eq!(ws.read_with(&vcx, |ws, _| ws.active_page()), None);
    let back = vcx
        .debug_bounds("flow-back-to-list")
        .expect("editor return action");
    vcx.simulate_click(back.center(), Modifiers::default());
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.active_page(), Some(Page::Flows));
        assert_eq!(ws.flows.browser.state.grouping, FlowGrouping::Origin);
    });
}

#[gpui::test]
async fn flow_browser_request_recovery_does_not_clear_definition_filters(cx: &mut TestAppContext) {
    let (_dir, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.set_flow_origin(Some(flow_paths::FlowOrigin::Repo), cx);
        ws.flows.browser.searches[0].update(cx, |input, cx| input.set_value("ship", window, cx));
        ws.flows.browser.searches[1].update(cx, |input, cx| input.set_value("hidden", window, cx));
        ws.show_flow_questions(window, cx);
        assert_eq!(ws.flows.browser.state.tab, FlowTab::Runs);
        assert_eq!(ws.flows.browser.state.run_filter, RunFilter::Asking);
        assert!(ws.flow_browser_snapshot(cx).query.is_empty());
        assert_eq!(
            ws.flows.browser.state.origin,
            Some(flow_paths::FlowOrigin::Repo)
        );
        assert_eq!(
            ws.flows.browser.searches[0].read(cx).value().as_ref(),
            "ship"
        );
    });
}

#[gpui::test]
async fn flow_browser_narrow_tables_scroll_to_the_action_column(cx: &mut TestAppContext) {
    use crate::workspace::flow_history::{FlowHistory, FlowRunEntry};
    use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};

    let (_dir, ws, path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut vcx = VisualTestContext::from_window(wh.into(), cx);
    vcx.simulate_resize(gpui::size(px(800.0), px(600.0)));
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.flows.history.put(
            ws.active,
            FlowHistory::seeded(vec![FlowRunEntry {
                dir: path.with_file_name("run-01"),
                started: "now".into(),
                report: None,
                status: daruda_flow::marker::RunStatus::Crashed,
            }]),
        );
    });
    for tab in [FlowTab::Definitions, FlowTab::Runs] {
        ws.update(&mut vcx, |ws, cx| ws.set_flow_tab(tab, cx));
        vcx.run_until_parked();
        let viewport = vcx.debug_bounds("flow-table-scroll").unwrap();
        let before = vcx.debug_bounds("flow-actions-column").unwrap();
        assert!(
            before.right() > viewport.right(),
            "{tab:?}: action={before:?}, viewport={viewport:?}"
        );
        vcx.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta: ScrollDelta::Pixels(point(px(-1000.0), px(0.0))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Started,
        });
        vcx.run_until_parked();
        let after = vcx.debug_bounds("flow-actions-column").unwrap();
        assert!(
            after.right() <= viewport.right(),
            "{tab:?} actions must be reachable"
        );
        assert!(after.left() >= viewport.left());
    }
}
