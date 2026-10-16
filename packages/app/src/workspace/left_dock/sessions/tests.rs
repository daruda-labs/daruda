use super::*;
use crate::workspace::tests::build_workspace;
use gpui::{AppContext as _, Focusable as _, TestAppContext, point, px};

fn live_rows(ws: &Workspace, cx: &gpui::App) -> Vec<SessionRow> {
    let acp = ws.agent_chat_statuses(cx);
    let sessions = crate::workspace::claude_status_aggregate::distinct_sessions(
        ws.active_runtime()
            .tabs
            .iter()
            .flat_map(|t| t.layout.pane_ids()),
        &ws.claude_sessions.pty_bindings,
        &ws.claude_sessions.status,
        &acp,
    );
    ws.named_lane_sessions(ws.active, &sessions, cx)
}

#[gpui::test]
async fn chooser_preserves_width_and_selects_the_exact_background_tab(cx: &mut TestAppContext) {
    let (wh, entity) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            let first = ws.active_runtime().focused_pane_id;
            ws.open_agent_chat_pane(window, cx);
            let rows = live_rows(ws, cx);
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].pane, first);
            assert_ne!(rows[0].id, rows[1].id);
            ws.docks.left.update(cx, |d, _| {
                d.is_open = false;
                d.resize(321.0);
            });
            let before = ws.docks.left.read(cx).display_size();
            let snap = ws.prepare_left_dock_snapshot(cx);
            ws.docks.left.update(cx, |d, _| {
                d.stage(DockSnapshot::Left(Box::new(snap)));
            });
            ws.choose_lane_from_rail(
                ws.active,
                gpui::Bounds::new(point(px(0.0), px(80.0)), gpui::size(px(48.0), px(36.0))),
                window,
                cx,
            );
            assert!(ws.docks.session_chooser.is_some());
            assert_eq!(ws.docks.left.read(cx).display_size(), before);
            assert_eq!(ws.docks.left.read(cx).size, 321.0);
            let chooser_focus = ws.docks.session_chooser.as_ref().unwrap().focus.clone();
            ws.close_session_chooser(false, window, cx);
            assert!(
                !chooser_focus.is_focused(window),
                "dismissal must not leave focus in an unmounted chooser"
            );
            ws.choose_lane_from_rail(
                ws.active,
                gpui::Bounds::new(point(px(0.0), px(80.0)), gpui::size(px(48.0), px(36.0))),
                window,
                cx,
            );
            ws.activate_dock_session(ws.active, &rows[0], window, cx);
            assert!(ws.docks.session_chooser.is_none());
            assert_eq!(ws.active_runtime().focused_pane_id, first);
            assert!(
                ws.active_runtime().tabs[ws.active_runtime().active_tab_index]
                    .layout
                    .contains(first)
            );
            assert_eq!(ws.docks.left.read(cx).display_size(), before);
            assert!(!ws.docks.left.read(cx).is_open);
        })
    })
    .unwrap();
}

#[gpui::test]
async fn stale_session_does_not_select_another_pane(cx: &mut TestAppContext) {
    let (wh, entity) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            let mut row = live_rows(ws, cx).remove(0);
            let before = ws.active_runtime().focused_pane_id;
            row.id = "no-longer-bound".into();
            ws.activate_dock_session(ws.active, &row, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, before);
        })
    })
    .unwrap();
}

#[gpui::test]
async fn duplicate_pty_binding_is_counted_once_and_uses_focused_pane(cx: &mut TestAppContext) {
    let (wh, entity) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            use daruda_agent::pty_tracker::PtyBinding;
            let first = ws.active_runtime().focused_pane_id;
            ws.add_tab(window, cx);
            let second = ws.active_runtime().focused_pane_id;
            for pane in [first, second] {
                ws.claude_sessions.pty_bindings.insert(
                    pane,
                    PtyBinding {
                        session_id: "shared".into(),
                        claude_pid: 1,
                    },
                );
            }
            let rows =
                ws.named_lane_sessions(ws.active, &[("shared".into(), SessionStatus::Working)], cx);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].pane, second);
            assert!(
                ws.named_lane_sessions(
                    LaneRef {
                        project: u64::MAX,
                        lane: 0
                    },
                    &[("shared".into(), SessionStatus::Working)],
                    cx
                )
                .is_empty()
            );
        })
    })
    .unwrap();
}

#[gpui::test]
async fn selecting_a_split_session_updates_the_input_owner_and_tab_focus(cx: &mut TestAppContext) {
    use crate::workspace::main_area::{pane_tree::SplitDirection, tab_ops::NewPaneKind};
    let (wh, entity) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            let first = ws.active_runtime().focused_pane_id;
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            let second = ws.active_runtime().focused_pane_id;
            assert_ne!(first, second);
            ws.input_dock.drafts.insert(first, "first draft".into());
            ws.input_dock
                .input
                .update(cx, |input, cx| input.set_value("second draft", window, cx));
            let row = live_rows(ws, cx)
                .into_iter()
                .find(|r| r.pane == first)
                .unwrap();
            ws.activate_dock_session(ws.active, &row, window, cx);
            assert_eq!(ws.active_runtime().focused_pane_id, first);
            assert_eq!(ws.input_dock.owner, Some(first));
            assert_eq!(ws.input_dock.input.read(cx).value().as_str(), "first draft");
            assert_eq!(
                ws.input_dock.drafts.get(&second).map(String::as_str),
                Some("second draft")
            );
            assert_eq!(ws.active_tab_mut().unwrap().last_focused_pane, first);
            assert!(
                ws.input_dock
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        })
    })
    .unwrap();
}

#[gpui::test]
async fn selecting_a_hidden_split_session_reveals_it_and_preserves_its_own_zoom(
    cx: &mut TestAppContext,
) {
    use crate::workspace::main_area::{pane_tree::SplitDirection, tab_ops::NewPaneKind};
    let (wh, entity) = build_workspace(cx);
    cx.run_until_parked();
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            let first = ws.active_runtime().focused_pane_id;
            ws.split_focused_pane_kind(
                NewPaneKind::AgentChat,
                SplitDirection::Horizontal,
                window,
                cx,
            );
            let second = ws.active_runtime().focused_pane_id;
            let row = live_rows(ws, cx)
                .into_iter()
                .find(|r| r.pane == first)
                .unwrap();
            ws.toggle_zoom_pane(second, cx);
            assert!(!ws.pane_on_screen(first));
            ws.activate_dock_session(ws.active, &row, window, cx);
            assert_eq!(ws.main_area.zoomed_pane_id, None);
            assert!(ws.pane_on_screen(first));
            ws.toggle_zoom_pane(first, cx);
            ws.activate_dock_session(ws.active, &row, window, cx);
            assert_eq!(ws.main_area.zoomed_pane_id, Some(first));
            assert!(ws.pane_on_screen(first));
        })
    })
    .unwrap();
}

#[gpui::test]
async fn rail_trigger_click_toggles_the_chooser_and_outside_click_dismisses(
    cx: &mut TestAppContext,
) {
    let config = daruda_config::Config::default();
    let root = std::env::temp_dir().join("daruda-session-chooser-trigger");
    std::fs::create_dir_all(&root).unwrap();
    let (wh, entity) = crate::workspace::tests::build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(&root)),
    );
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            ws.open_agent_chat_pane(window, cx);
            ws.docks.left.update(cx, |dock, cx| {
                dock.is_open = false;
                cx.notify();
            });
            cx.notify();
        });
    })
    .unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    let lane = entity.read_with(vcx, |ws, _| ws.active);
    let selector: &'static str =
        Box::leak(format!("rail-lane-{}-{}", lane.project, lane.lane).into_boxed_str());
    let row = vcx
        .debug_bounds(selector)
        .expect("the active worktree renders in the rail");
    vcx.simulate_click(row.center(), Default::default());
    vcx.run_until_parked();
    assert!(entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_some()));
    vcx.simulate_click(row.center(), Default::default());
    vcx.run_until_parked();
    assert!(
        entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_none()),
        "a second trigger click closes the popup"
    );
    vcx.simulate_click(row.center(), Default::default());
    vcx.run_until_parked();
    assert!(entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_some()));
    vcx.simulate_click(point(px(700.0), px(60.0)), Default::default());
    vcx.run_until_parked();
    assert!(
        entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_none()),
        "a press outside the trigger and popup dismisses it"
    );
}

#[gpui::test]
async fn project_choices_drill_into_sessions_and_back_without_resizing(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join("daruda-project-session-drill");
    std::fs::create_dir_all(&root).unwrap();
    let (wh, entity) = crate::workspace::tests::build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(root)),
    );
    cx.update_window(wh.into(), |_, window, cx| {
        entity.update(cx, |ws, cx| {
            ws.open_agent_chat_pane(window, cx);
            ws.open_agent_chat_pane(window, cx);
            ws.docks.left.update(cx, |d, cx| {
                d.is_open = false;
                cx.notify();
            });
            cx.notify();
        });
    })
    .unwrap();
    let vcx = &mut gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    let lane = entity.read_with(vcx, |ws, _| ws.active);
    let project_selector: &'static str =
        Box::leak(format!("rail-project-{}", lane.project).into_boxed_str());
    let row_selector: &'static str =
        Box::leak(format!("dock-worktree-{}-{}", lane.project, lane.lane).into_boxed_str());
    let project_bounds = vcx.debug_bounds(project_selector).unwrap();
    vcx.simulate_click(project_bounds.center(), Default::default());
    vcx.run_until_parked();
    let worktree = vcx.debug_bounds(row_selector).unwrap();
    vcx.simulate_click(worktree.center(), Default::default());
    vcx.run_until_parked();
    assert!(entity.read_with(vcx, |ws, cx| matches!(
        ws.docks.session_chooser.as_ref().unwrap().target,
        Target::Sessions { .. }
    ) && !ws.docks.left.read(cx).is_open));
    let back = vcx.debug_bounds("dock-worktrees-back").unwrap();
    vcx.simulate_click(back.center(), Default::default());
    vcx.run_until_parked();
    assert!(entity.read_with(vcx, |ws, _| matches!(
        ws.docks.session_chooser.as_ref().unwrap().target,
        Target::Worktrees { .. }
    )));
    let worktree = vcx.debug_bounds(row_selector).unwrap();
    vcx.simulate_click(worktree.center(), Default::default());
    vcx.run_until_parked();
    vcx.simulate_keystrokes("down enter");
    vcx.run_until_parked();
    assert!(entity.read_with(vcx, |ws, _| ws.docks.session_chooser.is_none()));
}
