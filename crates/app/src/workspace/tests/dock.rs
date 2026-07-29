use super::*;
use daruda_store::project::{LeftDockView, RightDockView};

// ---- Dock integration ----

#[gpui::test]
fn docks_start_closed_and_toggle_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.left_dock.read(cx).is_open);
        assert!(!ws.bottom_dock.read(cx).is_open);
        assert!(!ws.right_dock.read(cx).is_open);

        ws.left_dock.update(cx, |d, _| d.toggle());
        assert!(ws.left_dock.read(cx).is_open);
        ws.left_dock.update(cx, |d, _| d.toggle());
        assert!(!ws.left_dock.read(cx).is_open);

        ws.bottom_dock.update(cx, |d, _| d.toggle());
        assert!(ws.bottom_dock.read(cx).is_open);

        ws.right_dock.update(cx, |d, _| d.toggle());
        assert!(ws.right_dock.read(cx).is_open);
    });
}

/// The status bar's usage chip offers "open the Usage panel". Usage is
/// the right dock's default selected view, so selecting the tab alone is
/// a no-op — the reveal path has to open the closed dock too.
#[gpui::test]
fn right_dock_reveal_opens_switches_and_stays_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Usage);
        ws.reveal_right_dock_view(RightDockView::Usage, cx);
        assert!(ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Usage);

        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Skills);

        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.right_dock.read(cx).is_open);
    });
}

#[gpui::test]
fn dock_panels_register_expected_defaults(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        let left = ws.left_dock.read(cx);
        assert_eq!(left.panels.len(), 3);
        assert_eq!(
            left.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_WORKTREES
        );
        assert_eq!(
            left.panels[1].name(),
            crate::surface::strings::DOCK_PANEL_GIT
        );
        assert_eq!(
            left.panels[2].name(),
            crate::surface::strings::DOCK_PANEL_FILES
        );
        assert_eq!(left.active_panel, 0);
        assert_eq!(
            left.active_panel_name(),
            crate::surface::strings::DOCK_PANEL_WORKTREES
        );

        let bottom = ws.bottom_dock.read(cx);
        assert_eq!(bottom.panels.len(), 1);
        assert_eq!(
            bottom.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_MACROS
        );

        let right = ws.right_dock.read(cx);
        assert_eq!(right.panels.len(), 1);
        assert_eq!(
            right.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_AGENT_TASKS
        );
    });
}

#[gpui::test]
fn left_dock_view_updates_and_same_view_noops(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        for view in [
            LeftDockView::GitChanges,
            LeftDockView::Files,
            LeftDockView::Files,
            LeftDockView::Lanes,
        ] {
            ws.set_left_dock_view(view, cx);
            assert_eq!(ws.left_dock_view, view);
        }
    });
}

#[gpui::test]
fn dock_drag_resizes_clamps_tracks_positions_and_clears_stale(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.is_open = true);
        let start = ws.left_dock.read(cx).size;
        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        ws.left_dock.update(cx, |d, _| d.resize(start + 30.0));
        assert_eq!(ws.left_dock.read(cx).size, start + 30.0);
        ws.left_dock.update(cx, |d, _| d.resize(99999.0));
        assert_eq!(ws.left_dock.read(cx).size, ws.left_dock.read(cx).max_size);
        ws.left_dock.update(cx, |d, _| d.resize(0.0));
        assert_eq!(ws.left_dock.read(cx).size, ws.left_dock.read(cx).min_size);
        ws.end_dock_drag(cx);
        assert!(ws.dock_drag.is_none());

        ws.right_dock.update(cx, |d, _| d.is_open = true);
        ws.bottom_dock.update(cx, |d, _| d.is_open = true);
        ws.begin_dock_drag(layout::DockPosition::Right, 0.0, cx);
        assert!(matches!(
            ws.dock_drag.map(|d| d.position),
            Some(layout::DockPosition::Right)
        ));
        ws.end_dock_drag(cx);
        ws.begin_dock_drag(layout::DockPosition::Bottom, 0.0, cx);
        assert!(matches!(
            ws.dock_drag.map(|d| d.position),
            Some(layout::DockPosition::Bottom)
        ));
        ws.end_dock_drag(cx);

        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        assert!(ws.dock_drag.is_some());
        ws.end_stale_resize_drags(cx);
        assert!(ws.dock_drag.is_none());
        ws.end_stale_resize_drags(cx);
        assert!(ws.dock_drag.is_none());
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
        ws.read_with(cx, |ws, _| (ws.left_dock.clone(), ws.right_dock.clone()));

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
