//! A past run as the Flows page's detail: its report read in the page.

use super::*;
use crate::workspace::pages::Page;
use crate::workspace::pages::flows::detail::{FlowDetailBody, RunReport};

/// A run directory under `root` holding `report` as its `run.md`, or none.
fn run_dir(root: &std::path::Path, report: Option<&str>) -> std::path::PathBuf {
    let dir = root.join("run-1");
    std::fs::create_dir_all(&dir).expect("run dir");
    if let Some(report) = report {
        std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), report).expect("report");
    }
    dir
}

/// What the page's run detail holds: its directory and report text, `None`
/// for a report that is missing. Panics when the detail is not a run.
fn shown_run(ws: &Workspace) -> (std::path::PathBuf, Option<String>) {
    let detail = ws
        .pages
        .flows
        .detail
        .as_ref()
        .expect("the page holds a detail");
    let FlowDetailBody::Run(run) = &detail.body else {
        panic!("the detail is a run");
    };
    let text = match &run.report {
        RunReport::Loaded(text) => Some(text.to_string()),
        RunReport::Missing => None,
        RunReport::Loading => panic!("the report was read"),
    };
    (run.dir.clone(), text)
}

/// Opening a past run reads its report in the page: the worktree stays, no
/// tab opens, and Back is the list.
#[gpui::test]
async fn a_past_run_opens_its_report_in_the_page(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let dir = run_dir(lane.path(), Some("# Done\nAll passed."));
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let (active, panes) = ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        let before = (ws.active, ws.active_runtime().panes.len());
        ws.open_run_detail(ws.active, &dir, window, cx);
        before
    });
    vcx.run_until_parked();

    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.active_page(), Some(Page::Flows));
        assert_eq!(ws.active, active, "the worktree stays");
        assert_eq!(ws.active_runtime().panes.len(), panes, "no tab opened");
        let (shown, text) = shown_run(ws);
        assert_eq!(shown, dir);
        assert!(text.is_some_and(|t| t.contains("All passed")));
    });

    ws.update_in(&mut vcx, |ws, window, cx| ws.back_to_flow_list(window, cx));
    ws.read_with(&vcx, |ws, _| {
        assert_eq!(ws.active_page(), Some(Page::Flows));
        assert!(ws.pages.flows.detail.is_none(), "back is the list");
    });
}

/// A run that left no report says so rather than showing an empty page.
#[gpui::test]
async fn a_run_without_a_report_says_so(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let dir = run_dir(lane.path(), None);
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    ws.update_in(&mut vcx, |ws, window, cx| {
        ws.open_run_detail(ws.active, &dir, window, cx)
    });
    vcx.run_until_parked();
    ws.read_with(&vcx, |ws, _| assert_eq!(shown_run(ws), (dir.clone(), None)));
}

/// The open run comes back after a restart, its report read again.
#[gpui::test]
async fn the_open_run_reopens_after_a_restart(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let dir = run_dir(lane.path(), Some("# Done"));
    let (saved_workspace, saved_projects) = {
        let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
        ws.update_in(&mut vcx, |ws, window, cx| {
            ws.open_run_detail(ws.active, &dir, window, cx)
        });
        vcx.run_until_parked();
        ws.read_with(&vcx, |ws, app_cx| ws.snapshot_for_disk(app_cx))
    };

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh2, ws2) = build_workspace_with(cx, &config, Some(project));
    let mut vcx2 = gpui::VisualTestContext::from_window(wh2.into(), cx);
    ws2.update_in(&mut vcx2, |ws2, window, cx| {
        ws2.restore_from_disk(&saved_workspace, &saved_projects, window, cx)
    });
    vcx2.run_until_parked();
    ws2.read_with(&vcx2, |ws2, _| {
        assert_eq!(shown_run(ws2), (dir.clone(), Some("# Done".to_string())));
    });
}
