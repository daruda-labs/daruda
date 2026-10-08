use super::*;
use daruda_store::project::{LeftDockView, RightDockView};

// ---- Dock integration ----

#[gpui::test]
fn dock_defaults_toggles_and_view_selection(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(ws.docks.left.read(cx).is_open);
        assert!(!ws.docks.bottom.read(cx).is_open);
        assert!(!ws.docks.right.read(cx).is_open);

        // Panels are only ever counted — the tab strip owns the labels.
        assert_eq!(ws.docks.left.read(cx).panels.len(), 3);
        assert_eq!(ws.docks.bottom.read(cx).panels.len(), 1);
        assert_eq!(ws.docks.right.read(cx).panels.len(), 1);

        ws.docks.left.update(cx, |d, _| d.toggle());
        assert!(!ws.docks.left.read(cx).is_open);
        ws.docks.left.update(cx, |d, _| d.toggle());
        assert!(ws.docks.left.read(cx).is_open);

        ws.docks.bottom.update(cx, |d, _| d.toggle());
        assert!(ws.docks.bottom.read(cx).is_open);

        assert!(!ws.docks.right.read(cx).is_open);
        assert_eq!(ws.docks.right_view, RightDockView::Usage);
        ws.reveal_right_dock_view(RightDockView::Usage, cx);
        assert!(ws.docks.right.read(cx).is_open);
        assert_eq!(ws.docks.right_view, RightDockView::Usage);

        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.docks.right.read(cx).is_open);
        assert_eq!(ws.docks.right_view, RightDockView::Skills);

        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.docks.right.read(cx).is_open);
        ws.docks.right.update(cx, |d, _| d.toggle());
        assert!(!ws.docks.right.read(cx).is_open);
        ws.docks.right.update(cx, |d, _| d.toggle());
        assert!(ws.docks.right.read(cx).is_open);

        for view in [
            LeftDockView::GitChanges,
            LeftDockView::Files,
            LeftDockView::Files,
            LeftDockView::Lanes,
        ] {
            ws.set_left_dock_view(view, cx);
            assert_eq!(ws.docks.left_view, view);
        }
    });
}

/// The Input panel's stacked chrome is taller than the single-row macro
/// preset; showing it must lift the dock to fit, and the macro panel must be
/// free to shrink back afterwards.
#[gpui::test]
fn input_panel_lifts_the_bottom_dock_floor_to_its_one_row_height(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        let one_row = layout::ops::bottom_dock_height_for_rows(1);
        let macro_floor = crate::ui::theme::DOCK_BOTTOM_MIN_H;
        assert!(macro_floor < one_row);

        let tab_id = ws.panels.tabs[0].id.clone();
        ws.set_active_panel_tab(tab_id, cx);
        ws.docks.bottom.update(cx, |d, _| d.resize(0.0));
        assert_eq!(ws.docks.bottom.read(cx).size, macro_floor);

        ws.activate_bottom_input(cx);
        assert_eq!(ws.docks.bottom.read(cx).size, one_row);
        ws.docks.bottom.update(cx, |d, _| d.resize(0.0));
        assert_eq!(ws.docks.bottom.read(cx).size, one_row);
    });
}

#[gpui::test]
fn dock_drag_resizes_clamps_tracks_positions_and_clears_stale(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.docks.left.update(cx, |d, _| d.is_open = true);
        let start = ws.docks.left.read(cx).size;
        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        ws.docks.left.update(cx, |d, _| d.resize(start + 30.0));
        assert_eq!(ws.docks.left.read(cx).size, start + 30.0);
        ws.docks.left.update(cx, |d, _| d.resize(99999.0));
        assert_eq!(ws.docks.left.read(cx).size, ws.docks.left.read(cx).max_size);
        ws.docks.left.update(cx, |d, _| d.resize(0.0));
        assert_eq!(ws.docks.left.read(cx).size, ws.docks.left.read(cx).min_size);
        ws.end_dock_drag(cx);
        assert!(ws.docks.drag.is_none());

        ws.docks.right.update(cx, |d, _| d.is_open = true);
        ws.docks.bottom.update(cx, |d, _| d.is_open = true);
        ws.begin_dock_drag(layout::DockPosition::Right, 0.0, cx);
        assert!(matches!(
            ws.docks.drag.map(|d| d.position),
            Some(layout::DockPosition::Right)
        ));
        ws.end_dock_drag(cx);
        ws.begin_dock_drag(layout::DockPosition::Bottom, 0.0, cx);
        assert!(matches!(
            ws.docks.drag.map(|d| d.position),
            Some(layout::DockPosition::Bottom)
        ));
        ws.end_dock_drag(cx);

        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        assert!(ws.docks.drag.is_some());
        ws.end_stale_resize_drags(cx);
        assert!(ws.docks.drag.is_none());
        ws.end_stale_resize_drags(cx);
        assert!(ws.docks.drag.is_none());
    });
}

// ---- Dock notify reentrancy ----
//
// Dock event listeners run while the Dock entity is leased, so any Workspace op
// they dispatch that reaches `notify_left_dock` / `notify_right_dock` must be
// lease-free — otherwise it double-leases the dock and aborts the app (a panic
// across the objc event boundary cannot unwind).

#[gpui::test]
fn notify_docks_safe_while_dock_is_leased(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let (left_dock, right_dock) =
        ws.read_with(cx, |ws, _| (ws.docks.left.clone(), ws.docks.right.clone()));

    left_dock.update(cx, |_, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_left_dock_view(LeftDockView::Files, cx);
        });
    });

    right_dock.update(cx, |_, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_right_dock_view(RightDockView::Skills, cx);
        });
    });
}

/// Every "show me this right panel" action opens the dock, not just the tab.
///
/// `set_right_dock_view` moves the tab selection and returns early when that
/// view is already selected, so an affordance built on it alone shows nothing
/// while the dock is closed — and closed is how it starts. Each action below
/// was written that way; the assertion that catches it is `is_open`, not the
/// view, which is why asserting the view is not enough on its own.
#[gpui::test]
fn every_right_panel_action_opens_the_dock_it_selects_in(cx: &mut TestAppContext) {
    use crate::workspace::pages::Page;
    use crate::workspace::{
        FocusSkillSearch, SwitchRightPanelFlows, SwitchRightPanelSkills, SwitchRightPanelTasks,
        SwitchRightPanelTools, SwitchRightPanelUsage,
    };

    let (wh, ws) = build_workspace(cx);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Each action is checked from a closed dock, or the assertion
            // below would hold for a handler that only ever moves the tab.
            let mut check =
                |view: RightDockView,
                 act: &mut dyn FnMut(&mut Workspace, &mut Context<Workspace>)| {
                    if ws_is_open(ws, cx) {
                        ws.docks.right.update(cx, |d, _| d.toggle());
                    }
                    assert!(!ws_is_open(ws, cx), "the fixture left the dock open");
                    act(ws, cx);
                    assert!(ws_is_open(ws, cx), "{view:?} is behind a closed dock");
                    assert_eq!(ws.docks.right_view, view);
                };

            check(RightDockView::Usage, &mut |ws, cx| {
                ws.on_switch_right_panel_usage(&SwitchRightPanelUsage, window, cx)
            });
            check(RightDockView::Skills, &mut |ws, cx| {
                ws.on_switch_right_panel_skills(&SwitchRightPanelSkills, window, cx)
            });
            check(RightDockView::Tools, &mut |ws, cx| {
                ws.on_switch_right_panel_tools(&SwitchRightPanelTools, window, cx)
            });
            // `Cmd+/` promises the query box, which is not rendered at all
            // while the dock is shut.
            check(RightDockView::Skills, &mut |ws, cx| {
                ws.on_focus_skill_search(&FocusSkillSearch, window, cx)
            });

            // Tasks and Flows are pages: they must leave the utility dock shut.
            ws.docks.right.update(cx, |d, _| d.toggle());
            assert!(!ws_is_open(ws, cx), "the fixture left the dock open");
            ws.on_switch_right_panel_tasks(&SwitchRightPanelTasks, window, cx);
            assert_eq!(ws.active_page(), Some(Page::Tasks));
            ws.on_switch_right_panel_flows(&SwitchRightPanelFlows, window, cx);
            assert_eq!(ws.active_page(), Some(Page::Flows));
            assert!(!ws_is_open(ws, cx), "pages do not open the utility dock");
        });
    })
    .expect("the test window is live");
}

fn ws_is_open(ws: &Workspace, cx: &gpui::App) -> bool {
    ws.docks.right.read(cx).is_open
}
