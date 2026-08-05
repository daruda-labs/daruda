//! Tests for the System-B imperative PopupMenu deploy path
//! (`Workspace::open_context_menu` / `close_context_menu` +
//! `PopupMenuDeploy`, `crates/app/src/workspace/layout/ops.rs`).

use gpui::{Point, TestAppContext, px};

use crate::ui::PopupMenu;

use super::build_workspace;

#[gpui::test]
fn context_menu_open_close_and_empty_close_update_popup_deploy(cx: &mut TestAppContext) {
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

                ws.close_context_menu(cx);
                assert!(
                    ws.main_area.popup_menu_deploy.is_none(),
                    "close_context_menu should clear the deploy"
                );

                // Guarded by `.take().is_some()` — calling close on an
                // already-closed menu must not panic and must stay `None`.
                ws.close_context_menu(cx);
                assert!(ws.main_area.popup_menu_deploy.is_none());
            });
        })
        .unwrap();
}
