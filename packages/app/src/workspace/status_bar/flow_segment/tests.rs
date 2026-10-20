use super::*;
use daruda_store::project::LaneRef;

struct PanelProbe {
    rows: Vec<FlowRunRow>,
    workspace: WeakEntity<Workspace>,
}

impl gpui::Render for PanelProbe {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        flow_panel(&self.rows, self.workspace.clone(), window, cx)
    }
}

#[gpui::test]
fn a_rendered_stop_does_not_cancel_a_replacement_run(cx: &mut gpui::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let project = daruda_store::project::Project::from_path(directory.path());
    let (_, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(project),
    );
    let lane = workspace.read_with(cx, |ws, _| ws.active);
    let old_run = directory.path().join("old");
    let rows = workspace.update(cx, |ws, cx| {
        ws.seed_flow_run_for_test(lane, old_run, cx);
        ws.flow_status_rows(cx)
    });
    let weak_workspace = workspace.downgrade();
    let window = cx.add_window(move |_, _| PanelProbe {
        rows,
        workspace: weak_workspace,
    });
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    let selector: &'static str =
        Box::leak(format!("status-flow-stop-{}-{}", lane.project, lane.lane).into_boxed_str());
    let stop = vcx
        .debug_bounds(selector)
        .expect("the displayed run has Stop");
    let new_run = directory.path().join("new");
    workspace.update(&mut vcx, |ws, cx| {
        ws.seed_flow_run_for_test(lane, new_run.clone(), cx);
    });
    vcx.run_until_parked();
    vcx.simulate_click(stop.center(), gpui::Modifiers::none());
    workspace.read_with(&vcx, |ws, cx| {
        let run = ws.flow_run_in(lane, cx).unwrap();
        assert_eq!(run.view().run_dir(), new_run);
        assert!(
            !run.is_stopping(),
            "a stale Stop canceled the replacement run"
        );
    });
    let rows = workspace.read_with(&vcx, |ws, cx| ws.flow_status_rows(cx));
    window
        .update(&mut vcx, |probe, _, cx| {
            probe.rows = rows;
            cx.notify();
        })
        .unwrap();
    vcx.run_until_parked();
    let stop = vcx
        .debug_bounds(selector)
        .expect("the replacement has Stop");
    vcx.simulate_click(stop.center(), gpui::Modifiers::none());
    workspace.read_with(&vcx, |ws, cx| {
        assert!(ws.flow_run_in(lane, cx).unwrap().is_stopping());
    });
}

#[gpui::test]
fn scrolling_flow_actions_clear_the_scrollbar_hit_strip(cx: &mut gpui::TestAppContext) {
    let (_, workspace) = crate::workspace::tests::build_workspace(cx);
    let workspace = workspace.downgrade();
    let window = cx.add_window(move |_, _| PanelProbe {
        rows: (0..40)
            .map(|lane| FlowRunRow {
                lane: LaneRef { project: 1, lane },
                ..row("implement")
            })
            .collect(),
        workspace,
    });
    let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    let body = vcx.debug_bounds("status-flow-scroll-wrapper").unwrap();
    let stop = vcx.debug_bounds("status-flow-stop-1-0").unwrap();
    assert!(body.size.height > px(0.));
    assert!(body.size.height <= px(FLOW_PANEL_MAX_HEIGHT));
    assert!(stop.right() <= body.right() - px(theme::POPOVER_ACTION_GUTTER));
}

fn row(doing: &str) -> FlowRunRow {
    FlowRunRow {
        run_dir: std::path::PathBuf::from("run"),
        source: crate::workspace::flow_request::FlowSource::File("flow.yaml".into()),
        lane: LaneRef {
            project: 1,
            lane: 0,
        },
        lane_label: "temp / main".into(),
        doing: doing.to_string().into(),
        asking: Vec::new(),
    }
}

fn question(ask_id: u64) -> crate::workspace::flow_rows::AskRowData {
    crate::workspace::flow_rows::AskRowData {
        ask_id,
        asked: "implement is waiting".into(),
        tool: "Bash".into(),
        detail: None,
        options: Vec::new(),
    }
}

fn waiting_row() -> FlowRunRow {
    FlowRunRow {
        asking: vec![question(1)],
        ..row("implement")
    }
}

/// The header counts runs while none waits, and questions — a run's own
/// plus the ones its parallel nodes hold — in the attention tone once any does.
#[test]
fn the_header_counts_runs_until_someone_waits() {
    let (state, tone) = header_state(&[row("verdict"), row("implement")]);
    assert_eq!(state, crate::surface::strings::flow::menu_running(2));
    assert!(matches!(tone, super::super::menu::Tone::Plain));

    let parallel = FlowRunRow {
        asking: (1..=3).map(question).collect(),
        ..waiting_row()
    };
    let (state, tone) = header_state(&[row("verdict"), parallel]);
    assert_eq!(state, crate::surface::strings::flow::menu_waiting(3));
    assert!(matches!(tone, super::super::menu::Tone::Themed(_)));
}

/// A run waiting on a person outranks whatever the others are doing,
/// at any density. Work finishes on its own; a question does not, and
/// the chip is the only surface a user cannot hide.
#[test]
fn a_waiting_run_takes_the_chip_over_a_working_one() {
    for density in [StatusBarDensity::Full, StatusBarDensity::Compact] {
        let label = trigger_label(&[row("verdict"), waiting_row()], density);
        assert!(
            !label.contains("verdict"),
            "a working node held the chip while someone was waiting: {label}"
        );
        assert!(label.contains('1'), "{label}");
    }
    // And with nobody waiting it goes back to reporting.
    let plain = trigger_label(&[row("verdict")], StatusBarDensity::Full);
    assert!(plain.contains("verdict"), "{plain}");
}

/// One run has room to say what it is doing; several do not, and a
/// count is the honest thing to show instead of one of them.
#[test]
fn the_chip_names_the_node_only_when_there_is_one_run() {
    let one = trigger_label(&[row("verdict")], StatusBarDensity::Full);
    assert!(one.contains("verdict"), "{one}");

    let many = trigger_label(&[row("verdict"), row("gate")], StatusBarDensity::Full);
    assert!(!many.contains("verdict"), "{many}");
    assert!(many.contains('2'), "{many}");
}
