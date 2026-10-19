use super::project_initial;
#[test]
fn initials_keep_unicode_graphemes() {
    assert_eq!(project_initial(" daruda"), "D");
    assert_eq!(project_initial("한글"), "한");
    assert_eq!(project_initial("👩‍💻 app"), "👩‍💻");
    assert_eq!(project_initial(" "), "?");
}

#[test]
fn group_labels_are_short_unicode_abbreviations() {
    assert_eq!(super::group_abbreviation("Development"), "DEV");
    assert_eq!(super::group_abbreviation("Open source"), "OS");
    assert_eq!(super::group_abbreviation("개발팀"), "개발");
    assert_eq!(super::group_abbreviation(""), "?");
}

#[gpui::test]
async fn project_and_group_folding_preserve_the_active_pane_and_input(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{AppContext as _, Focusable as _};
    let root = std::env::temp_dir().join("daruda-rail-fold-input");
    std::fs::create_dir_all(&root).unwrap();
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let group = workspace.add_group("Development".into(), None, cx);
            workspace.move_project_to_group(workspace.active.project, Some(group), cx);
            workspace
                .active_project_mut()
                .unwrap()
                .lanes
                .push(crate::lane::Lane::default_for_project(77, root.clone()));
            workspace.input_dock.visible = true;
            workspace.input_dock.input.update(cx, |input, cx| {
                input.set_value("keep this draft", window, cx)
            });
            workspace.docks.left.update(cx, |dock, cx| {
                dock.is_open = false;
                cx.notify();
            });
            workspace
                .input_dock
                .input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.notify();
        });
    })
    .unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    let (active, pane, group) = workspace.read_with(vcx, |workspace, _| {
        (
            workspace.active,
            workspace.active_runtime().focused_pane_id,
            workspace.groups[0].id,
        )
    });
    let project_selector = Box::leak(format!("rail-project-{}", active.project).into_boxed_str());
    let empty_selector = Box::leak(format!("rail-lane-{}-77", active.project).into_boxed_str());
    assert!(bounds(vcx, empty_selector).is_some());
    for _ in 0..2 {
        let project = bounds(vcx, project_selector).unwrap();
        vcx.simulate_click(project.center(), Default::default());
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            workspace
                .read(cx)
                .assert_fold_state(active, pane, window, cx)
        });
    }
    let group_selector = Box::leak(format!("rail-group-{group}").into_boxed_str());
    let line_selector = Box::leak(format!("rail-group-status-{group}").into_boxed_str());
    assert!(bounds(vcx, line_selector).is_none());
    let group_bounds = bounds(vcx, group_selector).unwrap();
    vcx.simulate_click(group_bounds.center(), Default::default());
    vcx.run_until_parked();
    assert!(bounds(vcx, project_selector).is_none());
    vcx.update(|window, cx| {
        workspace
            .read(cx)
            .assert_fold_state(active, pane, window, cx)
    });
    let line = bounds(vcx, line_selector).unwrap();
    assert_eq!(line.left(), group_bounds.left());
    assert_eq!(line.right(), group_bounds.right());
    assert_eq!(line.bottom(), group_bounds.bottom());
    vcx.simulate_click(line.center(), Default::default());
    vcx.run_until_parked();
    assert!(bounds(vcx, project_selector).is_some());
    assert!(bounds(vcx, line_selector).is_none());
    let empty = bounds(vcx, empty_selector).unwrap();
    vcx.simulate_click(empty.center(), Default::default());
    vcx.run_until_parked();
    assert_eq!(
        workspace.read_with(vcx, |workspace, _| workspace.active.lane),
        77
    );
}

impl crate::workspace::Workspace {
    fn assert_fold_state(
        &self,
        active: daruda_store::project::LaneRef,
        pane: u64,
        window: &gpui::Window,
        cx: &gpui::App,
    ) {
        use gpui::Focusable as _;
        assert_eq!(self.active, active);
        assert_eq!(self.active_runtime().focused_pane_id, pane);
        assert!(self.docks.session_chooser.is_none());
        assert_eq!(
            self.input_dock.input.read(cx).value().as_str(),
            "keep this draft"
        );
        assert!(
            self.input_dock
                .input
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
        );
    }
}

#[gpui::test]
async fn expanded_rail_selects_terminal_and_agent_panes_without_a_chooser(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::AppContext as _;
    let root = std::env::temp_dir().join("daruda-rail-direct-panes");
    std::fs::create_dir_all(&root).unwrap();
    let (window, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root)),
    );
    let (terminal, agent) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let terminal = workspace.active_runtime().focused_pane_id;
                workspace.open_agent_chat_pane(window, cx);
                let agent = workspace.active_runtime().focused_pane_id;
                workspace.docks.left.update(cx, |dock, cx| {
                    dock.is_open = false;
                    cx.notify();
                });
                cx.notify();
                (terminal, agent)
            })
        })
        .unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    workspace.read_with(vcx, |workspace, cx| {
        let panes = workspace.rail_chooser_panes(workspace.active, cx);
        assert_eq!(
            panes
                .iter()
                .find(|pane| pane.pane == terminal)
                .unwrap()
                .neutral_icon,
            crate::ui::icons::TERMINAL
        );
        assert_eq!(
            panes
                .iter()
                .find(|pane| pane.pane == agent)
                .unwrap()
                .neutral_icon,
            crate::ui::icons::SESSION
        );
    });
    for pane in [terminal, agent] {
        let selector = Box::leak(format!("rail-pane-{pane}").into_boxed_str());
        let row = bounds(vcx, selector).unwrap();
        vcx.simulate_click(row.center(), Default::default());
        vcx.run_until_parked();
        workspace.read_with(vcx, |workspace, _| {
            assert_eq!(workspace.active_runtime().focused_pane_id, pane);
            assert_eq!(workspace.input_dock.owner, Some(pane));
            assert!(workspace.docks.session_chooser.is_none());
        });
    }
}

#[gpui::test]
fn navigation_preferences_survive_disk_roundtrip(cx: &mut gpui::TestAppContext) {
    let root = std::env::temp_dir().join("daruda-rail-preferences");
    let (_, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root)),
    );
    workspace.update(cx, |workspace, cx| {
        let active = workspace.active;
        let original_name = workspace.active_project().unwrap().name.clone();
        workspace.set_project_display_alias(active.project, Some(" 개발 ".into()), cx);
        workspace.set_lane_rail_grouped(active, true, cx);
        workspace.toggle_project_collapse(active.project, cx);
        let (stored, projects) = workspace.snapshot_for_disk(cx);
        let encoded = serde_json::to_string(&stored).unwrap();
        let decoded: daruda_store::project::WorkspaceState =
            serde_json::from_str(&encoded).unwrap();
        let project = &projects[0];
        let restored = crate::project::Project::from_disk(
            active.project,
            project,
            &decoded.project_overrides[&project.uuid],
        );
        assert_eq!(restored.display_alias.as_deref(), Some("개발"));
        assert!(restored.rail_grouped_lanes.contains(&active.lane));
        assert!(restored.is_collapsed);
        assert_eq!(restored.name, original_name);
        workspace.set_project_display_alias(active.project, Some("three".into()), cx);
        assert_eq!(
            workspace.active_project().unwrap().display_alias.as_deref(),
            Some("개발")
        );
        workspace.set_project_display_alias(active.project, None, cx);
        assert!(workspace.active_project().unwrap().display_alias.is_none());
        workspace.set_lane_rail_grouped(active, false, cx);
        assert!(
            workspace
                .active_project()
                .unwrap()
                .rail_grouped_lanes
                .is_empty()
        );
    });
}

#[gpui::test]
fn collapsed_group_indicator_uses_only_the_focused_pane(cx: &mut gpui::TestAppContext) {
    use crate::workspace::tab_indicator::TabIndicator;
    use daruda_agent::SessionStatus;
    let root = std::env::temp_dir().join("daruda-rail-group-indicator");
    let (_, workspace) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root)),
    );
    workspace.update(cx, |workspace, cx| {
        let group = workspace.add_group("Development".into(), None, cx);
        workspace.move_project_to_group(workspace.active.project, Some(group), cx);
        workspace.toggle_group_collapse(group, cx);
        let mut snap = workspace.prepare_left_dock_snapshot(cx);
        let focused = snap.focused_pane;
        snap.rail_panes.insert(
            snap.active,
            vec![
                super::PaneSnapshot {
                    pane: focused,
                    title: "focused".into(),
                    neutral_icon: crate::ui::icons::SESSION,
                    status: Some(SessionStatus::Working),
                    outcome: None,
                },
                super::PaneSnapshot {
                    pane: focused + 1,
                    title: "sibling".into(),
                    neutral_icon: crate::ui::icons::SESSION,
                    status: Some(SessionStatus::Failed),
                    outcome: None,
                },
            ],
        );
        assert_eq!(
            super::groups::active_indicator(&snap.groups[0], &snap),
            Some(TabIndicator::Working)
        );
        snap.focused_pane = focused + 1;
        assert_eq!(
            super::groups::active_indicator(&snap.groups[0], &snap),
            Some(TabIndicator::Failed)
        );
        snap.projects[0].group_id = None;
        assert_eq!(
            super::groups::active_indicator(&snap.groups[0], &snap),
            None
        );
    });
}

#[test]
fn pane_kind_icons_preserve_connected_idle_and_other_session_states() {
    use daruda_agent::{AgentOutcome, SessionStatus};

    for icon in [crate::ui::icons::TERMINAL, crate::ui::icons::SESSION] {
        let mut pane = super::PaneSnapshot {
            pane: 1,
            title: "pane".into(),
            neutral_icon: icon,
            status: None,
            outcome: None,
        };
        assert_eq!(pane.unconnected_icon(), Some(icon));
        for outcome in [AgentOutcome::Completed, AgentOutcome::Errored] {
            pane.outcome = Some(outcome);
            assert_eq!(pane.unconnected_icon(), None);
        }
        pane.outcome = None;
        for status in [
            SessionStatus::Idle,
            SessionStatus::Working,
            SessionStatus::ExecutingTool,
            SessionStatus::NeedsAttention,
            SessionStatus::Failed,
            SessionStatus::Connecting,
        ] {
            pane.status = Some(status);
            assert_eq!(pane.unconnected_icon(), None, "{status:?}");
        }
    }
}

fn bounds(
    vcx: &mut gpui::VisualTestContext,
    selector: &'static str,
) -> Option<gpui::Bounds<gpui::Pixels>> {
    // WORKAROUND: GPUI cached paint omits debug_bounds replay; refresh only
    // this test instead of changing vendored instrumentation for a feature test.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.debug_bounds(selector)
}

#[gpui::test]
async fn live_size_reload_reflows_the_cached_rail_without_changing_width(
    cx: &mut gpui::TestAppContext,
) {
    let root = std::env::temp_dir().join("daruda-rail-size-test");
    std::fs::create_dir_all(&root).unwrap();
    let (wh, entity) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(&root)),
    );
    entity.update(cx, |ws, cx| {
        ws.docks.left.update(cx, |dock, cx| {
            dock.is_open = false;
            cx.notify();
        });
        cx.notify();
    });
    let vcx = &mut gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    let dock = entity.read_with(vcx, |ws, _| ws.docks.left.clone());
    let original_width = dock.read_with(vcx, |d, _| d.display_size());
    for size in daruda_config::UiSize::ALL {
        let before = dock.read_with(vcx, |d, _| d.render_count.get());
        entity.update(vcx, |ws, cx| {
            let mut config = daruda_config::Config::default();
            config.left_dock.size = size;
            ws.apply_config(&config, cx);
        });
        vcx.run_until_parked();
        dock.read_with(vcx, |d, _| {
            let crate::workspace::layout::DockSnapshot::Left(snap) = &d.snap else {
                panic!("the rail must retain its left dock snapshot");
            };
            assert_eq!(
                snap.metrics,
                crate::ui::theme::UiMetrics(size),
                "snapshot must advance before painting"
            );
            assert!(
                d.render_count.get() > before,
                "{size:?}: cached dock renders {before} -> {}, snapshot {:?}",
                d.render_count.get(),
                snap.metrics
            );
            assert_eq!(d.display_size(), original_width);
        });
    }
    let before = dock.read_with(vcx, |d, _| d.render_count.get());
    entity.update(vcx, |_, cx| cx.notify());
    vcx.run_until_parked();
    assert_eq!(
        dock.read_with(vcx, |d, _| d.render_count.get()),
        before,
        "an unchanged snapshot must retain its cached paint"
    );
}
