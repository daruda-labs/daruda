use super::super::execution_fixture::{ExecutionShot, seed};
use super::*;
use crate::workspace::tests::build_workspace_with;
use gpui::TestAppContext;

#[gpui::test]
fn stale_details_and_attempt_reads_cannot_replace_the_current_selection(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let first = seed(&root.path().join("first"), ExecutionShot::Completed);
    let second = seed(&root.path().join("second"), ExecutionShot::Retry);
    let history = RunHistory::read(&second).unwrap();
    for (evidence, text) in [(1, "first attempt content"), (2, "second attempt content")] {
        std::fs::write(
            history
                .transcript(&second, &"prepare".into(), evidence)
                .unwrap(),
            text,
        )
        .unwrap();
    }
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, _, cx| {
        ws.install_run_detail(ws.active, first, cx);
        let stale = ws.pages.flows.detail.as_ref().unwrap().id;
        ws.install_run_detail(ws.active, second.clone(), cx);
        let current = ws.pages.flows.detail.as_ref().unwrap().id;
        ws.land_run_execution(stale, Err("stale failure".into()), cx);
        ws.land_run_execution(current, Ok(history), cx);
        ws.select_execution_attempt(current, 1, cx);
        ws.select_execution_attempt(current, 2, cx);
        ws.select_execution_attempt(current, 3, cx);
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, cx| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        assert_eq!(run.dir, second);
        assert_eq!(
            run.execution.inspector.selection().cloned(),
            Some(
                super::super::execution_selection::ExecutionSelection::Attempt("prepare".into(), 2)
            )
        );
        let Evidence::Ready { text, .. } = run.execution.inspector.evidence() else {
            panic!("evidence loaded");
        };
        assert!(text.contains("second attempt content"));
        assert_eq!(
            run.execution
                .graph
                .view()
                .unwrap()
                .canvas
                .read(cx)
                .graph()
                .nodes()
                .len(),
            2
        );
    });
}

#[gpui::test]
fn tab_changes_drop_in_flight_transcript_reads_and_load_the_selected_output(
    cx: &mut TestAppContext,
) {
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Completed);
    let history = RunHistory::read(&dir).unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, _, cx| {
        ws.install_run_detail(ws.active, dir, cx);
        let id = ws.pages.flows.detail.as_ref().unwrap().id;
        ws.land_run_execution(id, Ok(history), cx);
        ws.select_execution_tab(id, EvidenceTab::Output, cx);
        ws.select_execution_tab(id, EvidenceTab::Conversation, cx);
        ws.select_execution_tab(id, EvidenceTab::Output, cx);
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        assert!(run.execution.inspector.tab() == EvidenceTab::Output);
        let Evidence::Ready { text, .. } = run.execution.inspector.evidence() else {
            panic!("output");
        };
        assert!(text.contains("# Release notes"));
        assert!(!text.contains("# Saved transcript"));
    });
}

#[gpui::test]
fn selecting_an_unstarted_node_settles_without_an_attempt(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Active);
    let history = RunHistory::read(&dir).unwrap();
    let node = daruda_flow::NodeId::from("review");
    assert!(history.attempts(&node).is_empty());
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, _, cx| {
        ws.install_run_detail(ws.active, dir, cx);
        let id = ws.pages.flows.detail.as_ref().unwrap().id;
        ws.land_run_execution(id, Ok(history), cx);
        ws.select_execution_node(id, node.clone(), cx);
        ws.select_execution_node(id, node.clone(), cx);
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        assert_eq!(
            run.execution.inspector.selection().cloned(),
            Some(ExecutionSelection::Node(node))
        );
        assert!(matches!(
            run.execution.inspector.evidence(),
            Evidence::Empty
        ));
    });
}

#[gpui::test]
fn refresh_preserves_navigation_and_returning_reads_hidden_changes(cx: &mut TestAppContext) {
    use crate::ui::flow_canvas::{Command, CommandContext};
    use crate::workspace::pages::Page;
    struct PanAndZoom;
    impl Command for PanAndZoom {
        fn name(&self) -> &'static str {
            "test_execution_viewport"
        }
        fn execute(&mut self, ctx: &mut CommandContext) {
            ctx.set_zoom(0.6);
            ctx.set_offset(gpui::point(gpui::px(31.0), gpui::px(47.0)));
        }
        fn undo(&mut self, _: &mut CommandContext) {}
    }
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Completed);
    let history = RunHistory::read(&dir).unwrap();
    let transcript = history.transcript(&dir, &"prepare".into(), 1).unwrap();
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx);
        ws.open_run_detail(ws.active, &dir, window, cx);
    });
    vcx.run_until_parked();
    let (id, canvas) = workspace.update_in(&mut vcx, |ws, _, cx| {
        let detail = ws.pages.flows.detail.as_ref().unwrap();
        let FlowDetailBody::Run(run) = &detail.body else {
            panic!("run");
        };
        let canvas = run.execution.graph.view().unwrap().canvas.clone();
        canvas.update(cx, |canvas, cx| canvas.dispatch_command(PanAndZoom, cx));
        (detail.id, canvas)
    });
    vcx.run_until_parked();
    let added = dir.parent().unwrap().join("new-history-entry");
    std::fs::create_dir(&added).unwrap();
    workspace.update_in(&mut vcx, |ws, _, cx| {
        for _ in 0..10 {
            ws.refresh_execution_progress(id, cx);
        }
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, cx| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        assert_eq!(
            run.execution.graph.view().unwrap().canvas.entity_id(),
            canvas.entity_id()
        );
        assert_eq!(canvas.read(cx).viewport().zoom(), 0.6);
        assert_eq!(
            canvas.read(cx).viewport().offset(),
            gpui::point(gpui::px(31.0), gpui::px(47.0))
        );
        assert!(!run.execution.rail.iter().any(|row| row.entry.dir == added));
    });
    workspace.update_in(&mut vcx, |ws, _, cx| ws.close_page(cx));
    std::fs::write(&transcript, "conversation changed while hidden").unwrap();
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(Page::Flows, window, cx)
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        let Evidence::Ready { text, .. } = run.execution.inspector.evidence() else {
            panic!("evidence");
        };
        assert!(text.contains("conversation changed while hidden"));
        assert!(run.execution.rail.iter().any(|row| row.entry.dir == added));
        assert_eq!(
            run.execution.inspector.selection().cloned(),
            Some(ExecutionSelection::Attempt("prepare".into(), 1))
        );
    });
}

#[gpui::test]
fn report_navigation_keeps_the_run_attempt_and_evidence_tab(cx: &mut TestAppContext) {
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Retry);
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(crate::workspace::pages::Page::Flows, window, cx);
        ws.open_run_detail(ws.active, &dir, window, cx);
    });
    vcx.run_until_parked();
    let id = workspace.update_in(&mut vcx, |ws, _, cx| {
        let id = ws.pages.flows.detail.as_ref().unwrap().id;
        ws.select_execution_attempt(id, 1, cx);
        ws.select_execution_tab(id, EvidenceTab::Output, cx);
        ws.show_execution_report(id, cx);
        id
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let detail = ws.pages.flows.detail.as_ref().unwrap();
        let FlowDetailBody::Run(run) = &detail.body else {
            panic!("run");
        };
        assert!(run.execution.content == ExecutionContent::Report);
        assert!(matches!(
            run.report(),
            super::super::detail::RunReport::Loaded(_)
        ));
        assert_eq!(detail.id, id);
        assert_eq!(ws.active_page(), Some(crate::workspace::pages::Page::Flows));
    });
    std::fs::write(
        dir.join(daruda_flow::record::RUN_REPORT_FILE),
        "# Updated report",
    )
    .unwrap();
    workspace.update_in(&mut vcx, |ws, _, cx| ws.refresh_execution(id, cx));
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
            panic!("run");
        };
        let super::super::detail::RunReport::Loaded(text) = run.report() else {
            panic!("report");
        };
        assert!(text.contains("Updated report"));
    });
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.back_from_execution(window, cx)
    });
    vcx.run_until_parked();
    workspace.read_with(&vcx, |ws, _| {
        let detail = ws.pages.flows.detail.as_ref().unwrap();
        let FlowDetailBody::Run(run) = &detail.body else {
            panic!("run");
        };
        assert!(run.execution.content == ExecutionContent::Attempts);
        assert!(run.execution.inspector.tab() == EvidenceTab::Output);
        assert_eq!(
            run.execution.inspector.selection().cloned(),
            Some(ExecutionSelection::Attempt("prepare".into(), 1))
        );
        assert_eq!(detail.id, id);
    });
}

#[gpui::test]
fn long_conversation_and_output_scroll_inside_the_inspector(cx: &mut TestAppContext) {
    use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};
    let root = tempfile::tempdir().unwrap();
    let dir = seed(root.path(), ExecutionShot::Completed);
    let history = RunHistory::read(&dir).unwrap();
    let (node, evidence) = history.latest_selection().unwrap();
    let text = "A saved evidence paragraph.\n\n".repeat(100);
    for path in [
        history.transcript(&dir, &node, evidence).unwrap(),
        history.output(&dir, &node, evidence).unwrap(),
    ] {
        std::fs::write(path, &text).unwrap();
    }
    let project = daruda_store::project::Project::from_path(root.path());
    let (window, workspace) =
        build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    workspace.update_in(&mut vcx, |ws, window, cx| {
        ws.open_page(crate::workspace::pages::Page::Flows, window, cx);
        ws.open_run_detail(ws.active, &dir, window, cx);
    });
    vcx.run_until_parked();
    for width in [1440.0, 1000.0, 700.0] {
        vcx.simulate_resize(gpui::size(px(width), px(800.0)));
        vcx.run_until_parked();
        for tab in [EvidenceTab::Conversation, EvidenceTab::Output] {
            workspace.update_in(&mut vcx, |ws, _, cx| {
                let id = ws.pages.flows.detail.as_ref().unwrap().id;
                ws.select_execution_tab(id, tab, cx);
            });
            vcx.run_until_parked();
            vcx.update(|window, _| window.refresh());
            vcx.run_until_parked();
            let viewport = vcx.debug_bounds("execution-evidence-scroll").unwrap();
            let footer = vcx.debug_bounds("execution-inspector-footer").unwrap();
            assert!(viewport.bottom() <= px(800.0));
            let before = vcx.debug_bounds("execution-evidence-body").unwrap();
            assert!(
                before.size.height > viewport.size.height,
                "evidence must exceed viewport: {before:?}, {viewport:?}"
            );
            vcx.simulate_event(ScrollWheelEvent {
                position: viewport.center(),
                delta: ScrollDelta::Pixels(point(px(0.0), px(-100.0))),
                modifiers: Default::default(),
                touch_phase: TouchPhase::Moved,
            });
            vcx.run_until_parked();
            let after = vcx.debug_bounds("execution-evidence-body").unwrap();
            assert!(
                after.origin.y < before.origin.y,
                "wheel must move evidence: {before:?}, {after:?}"
            );
            assert_eq!(
                vcx.debug_bounds("execution-evidence-scroll").unwrap(),
                viewport
            );
            assert_eq!(
                vcx.debug_bounds("execution-inspector-footer").unwrap(),
                footer
            );
        }
    }
}
