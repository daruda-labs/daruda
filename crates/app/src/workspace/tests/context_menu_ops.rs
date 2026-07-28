//! Tests for the System-B imperative PopupMenu deploy path
//! (`Workspace::open_context_menu` / `close_context_menu` +
//! `PopupMenuDeploy`, `crates/app/src/workspace/layout/ops.rs`).

use gpui::{Point, TestAppContext, px};

use crate::ui::PopupMenu;

use super::build_workspace;

#[gpui::test]
fn open_context_menu_sets_popup_menu_deploy(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    window_handle
        .update(cx, |_, window, cx| {
            let position = Point::new(px(10.), px(20.));
            let menu = PopupMenu::build(window, cx, |menu, _window, _cx| menu);

            workspace.update(cx, |ws, cx| {
                assert!(
                    ws.main_area.popup_menu_deploy.is_none(),
                    "no menu open before the call"
                );
                ws.open_context_menu(position, menu, cx);
                let deploy = ws
                    .main_area
                    .popup_menu_deploy
                    .as_ref()
                    .expect("open_context_menu should populate popup_menu_deploy");
                assert_eq!(deploy.position, position);
            });
        })
        .unwrap();
}

#[gpui::test]
fn close_context_menu_clears_popup_menu_deploy(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    window_handle
        .update(cx, |_, window, cx| {
            let menu = PopupMenu::build(window, cx, |menu, _window, _cx| menu);
            workspace.update(cx, |ws, cx| {
                ws.open_context_menu(Point::new(px(0.), px(0.)), menu, cx);
                assert!(ws.main_area.popup_menu_deploy.is_some());
                ws.close_context_menu(cx);
                assert!(
                    ws.main_area.popup_menu_deploy.is_none(),
                    "close_context_menu should clear the deploy"
                );
            });
        })
        .unwrap();
}

#[gpui::test]
fn close_context_menu_is_a_no_op_when_already_closed(cx: &mut TestAppContext) {
    let (_window, workspace) = build_workspace(cx);

    workspace.update(cx, |ws, cx| {
        assert!(ws.main_area.popup_menu_deploy.is_none());
        // Guarded by `.take().is_some()` — calling close on an already-closed
        // menu must not panic and must stay `None`.
        ws.close_context_menu(cx);
        assert!(ws.main_area.popup_menu_deploy.is_none());
    });
}
