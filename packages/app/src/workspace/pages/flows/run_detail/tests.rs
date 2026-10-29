use super::*;
use crate::workspace::pages::flows::execution_fixture::{ExecutionShot, seed};
use crate::workspace::tests::build_workspace_with;

#[gpui::test]
fn older_report_reads_cannot_replace_a_newer_request(cx: &mut gpui::TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Completed);
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, _, cx| {
        ws.install_run_detail(ws.active, dir, cx);
        let id = ws.pages.flows.detail.as_ref().unwrap().id;
        let epoch = |ws: &Workspace| {
            let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
                panic!("run");
            };
            run.execution.report_epoch
        };
        let initial = epoch(ws);
        ws.show_execution_report(id, cx);
        let opened = epoch(ws);
        ws.refresh_execution(id, cx);
        let refreshed = epoch(ws);
        assert!(initial < opened && opened < refreshed);
        ws.land_run_report(id, opened, Some("stale content".into()), cx);
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        assert!(matches!(run.report, RunReport::Loading));
        ws.land_run_report(id, refreshed, Some("latest content".into()), cx);
        ws.land_run_report(id, initial, None, cx);
        ws.land_run_report(id, opened, Some("stale content".into()), cx);
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        let RunReport::Loaded(text) = &run.report else {
            panic!("loaded report");
        };
        assert_eq!(text.as_ref(), "latest content");
    });
}
