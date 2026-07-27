use super::*;
use daruda_store::project::RightDockView;

// ---- Dock integration ----

#[gpui::test]
fn test_left_dock_starts_closed(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.left_dock.read(cx).is_open);
        assert!(!ws.bottom_dock.read(cx).is_open);
        assert!(!ws.right_dock.read(cx).is_open);
    });
}

#[gpui::test]
fn test_toggle_left_dock(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.left_dock.read(cx).is_open);
        ws.left_dock.update(cx, |d, _| d.toggle());
        assert!(ws.left_dock.read(cx).is_open);
        ws.left_dock.update(cx, |d, _| d.toggle());
        assert!(!ws.left_dock.read(cx).is_open);
    });
}

#[gpui::test]
fn test_toggle_bottom_dock(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.bottom_dock.read(cx).is_open);
        ws.bottom_dock.update(cx, |d, _| d.toggle());
        assert!(ws.bottom_dock.read(cx).is_open);
    });
}

#[gpui::test]
fn test_toggle_right_dock(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.right_dock.read(cx).is_open);
        ws.right_dock.update(cx, |d, _| d.toggle());
        assert!(ws.right_dock.read(cx).is_open);
    });
}

/// The status bar's usage chip offers "open the Usage panel". Usage is
/// the right dock's default selected view, so selecting the tab alone is
/// a no-op — the reveal path has to open the closed dock too.
#[gpui::test]
fn test_reveal_right_dock_view_opens_a_closed_dock_on_the_default_view(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        assert!(!ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Usage);
        ws.reveal_right_dock_view(RightDockView::Usage, cx);
        assert!(ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Usage);
    });
}

#[gpui::test]
fn test_reveal_right_dock_view_switches_view_and_keeps_an_open_dock_open(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.right_dock.update(cx, |d, _| d.toggle());
        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.right_dock.read(cx).is_open);
        assert_eq!(ws.right_dock_view, RightDockView::Skills);
        // Idempotent: revealing the already-visible view must not toggle
        // the dock back shut.
        ws.reveal_right_dock_view(RightDockView::Skills, cx);
        assert!(ws.right_dock.read(cx).is_open);
    });
}

#[gpui::test]
fn test_left_dock_registers_three_panels(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        let d = ws.left_dock.read(cx);
        assert_eq!(d.panels.len(), 3);
        assert_eq!(
            d.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_WORKTREES
        );
        assert_eq!(d.panels[1].name(), crate::surface::strings::DOCK_PANEL_GIT);
        assert_eq!(
            d.panels[2].name(),
            crate::surface::strings::DOCK_PANEL_FILES
        );
    });
}

#[gpui::test]
fn test_left_dock_active_panel_matches_default_dock_view(cx: &mut TestAppContext) {
    // Default dock view is Lanes → index 0.
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        let d = ws.left_dock.read(cx);
        assert_eq!(d.active_panel, 0);
        assert_eq!(
            d.active_panel_name(),
            crate::surface::strings::DOCK_PANEL_WORKTREES
        );
    });
}

#[gpui::test]
fn test_set_dock_view_updates_dock_view(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::GitChanges
        );
    });
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
    });
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Lanes, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Lanes
        );
    });
}

#[gpui::test]
fn test_set_dock_view_no_op_when_same(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
    });
    ws.update(cx, |ws, cx| {
        ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.left_dock_view,
            daruda_store::project::LeftDockView::Files
        );
    });
}

#[gpui::test]
fn test_bottom_dock_registers_macros_panel(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        let d = ws.bottom_dock.read(cx);
        assert_eq!(d.panels.len(), 1);
        assert_eq!(
            d.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_MACROS
        );
    });
}

#[gpui::test]
fn test_right_dock_has_agent_chat_panel(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.read_with(cx, |ws, cx| {
        let d = ws.right_dock.read(cx);
        assert_eq!(d.panels.len(), 1);
        assert_eq!(
            d.panels[0].name(),
            crate::surface::strings::DOCK_PANEL_AGENT_TASKS
        );
    });
}

#[gpui::test]
fn test_dock_drag_resizes_left_dock(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.is_open = true);
        let start = ws.left_dock.read(cx).size;
        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        // Mutate size directly; the mousemove path needs a live Window.
        ws.left_dock.update(cx, |d, _| d.resize(start + 30.0));
        assert_eq!(ws.left_dock.read(cx).size, start + 30.0);
        ws.end_dock_drag(cx);
        assert!(ws.dock_drag.is_none());
    });
}

#[gpui::test]
fn test_dock_drag_clamps_to_range(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        ws.left_dock.update(cx, |d, _| d.is_open = true);
        ws.begin_dock_drag(layout::DockPosition::Left, 0.0, cx);
        // Pull way past max — clamp must hold.
        ws.left_dock.update(cx, |d, _| d.resize(99999.0));
        assert_eq!(ws.left_dock.read(cx).size, ws.left_dock.read(cx).max_size);
        ws.left_dock.update(cx, |d, _| d.resize(0.0));
        assert_eq!(ws.left_dock.read(cx).size, ws.left_dock.read(cx).min_size);
        ws.end_dock_drag(cx);
    });
}

#[gpui::test]
fn test_dock_drag_right_and_bottom_track_their_own_sizes(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
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
    });
}

#[gpui::test]
fn test_end_stale_resize_drags_clears_live_drag(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    ws.update(cx, |ws, cx| {
        // Missed release: a drag is live but button-up landed outside the
        // window, so the next in-window move routes here instead of resizing.
        ws.left_dock.update(cx, |d, _| d.is_open = true);
        ws.begin_dock_drag(layout::DockPosition::Left, 100.0, cx);
        assert!(ws.dock_drag.is_some());
        ws.end_stale_resize_drags(cx);
        assert!(ws.dock_drag.is_none());
        // Idempotent — no live drag is a no-op, never panics.
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
fn test_notify_left_dock_safe_while_dock_is_leased(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let dock = ws.read_with(cx, |ws, _| ws.left_dock.clone());
    dock.update(cx, |_, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
        });
    });
}

#[gpui::test]
fn test_notify_right_dock_safe_while_dock_is_leased(cx: &mut TestAppContext) {
    let (_wh, ws) = build_workspace(cx);
    let dock = ws.read_with(cx, |ws, _| ws.right_dock.clone());
    dock.update(cx, |_, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_right_dock_view(daruda_store::project::RightDockView::Skills, cx);
        });
    });
}
