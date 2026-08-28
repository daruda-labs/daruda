//! Tests for the root-deployed PopupMenu path
//! (`Workspace::open_context_menu` / `close_context_menu` +
//! `PopupMenuDeploy`, `crates/app/src/workspace/layout/ops.rs`).

use gpui::{Focusable as _, Point, TestAppContext, px};

use crate::ui::PopupMenu;

use super::{build_workspace, build_workspace_with};

#[gpui::test]
fn context_menu_open_close_and_empty_close_update_popup_deploy(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    window_handle
        .update(cx, |_, window, cx| {
            let position = Point::new(px(10.), px(20.));
            let menu = PopupMenu::build(window, cx, |menu, _window, _cx| menu);

            workspace.update(cx, |ws, cx| {
                let previous_focus = ws.focus_handle.clone();
                previous_focus.focus(window, cx);
                assert!(
                    ws.main_area.popup_menu_deploy.is_none(),
                    "no menu open before the call"
                );
                ws.open_context_menu(position, menu, window, cx);
                let deploy = ws
                    .main_area
                    .popup_menu_deploy
                    .as_ref()
                    .expect("open_context_menu should populate popup_menu_deploy");
                assert_eq!(deploy.position, position);
                assert!(
                    deploy.menu.focus_handle(cx).contains_focused(window, cx),
                    "the open menu owns keyboard focus"
                );

                ws.close_context_menu(window, cx);
                assert!(
                    ws.main_area.popup_menu_deploy.is_none(),
                    "close_context_menu should clear the deploy"
                );
                assert!(
                    previous_focus.is_focused(window),
                    "closing the menu restores the focus it replaced"
                );

                // Calling close on an already-closed menu must return without
                // changing focus or recreating the deploy.
                ws.close_context_menu(window, cx);
                assert!(ws.main_area.popup_menu_deploy.is_none());
            });
        })
        .unwrap();
}

/// A dock row's right-click must reach `Workspace`, not a menu element inside
/// the row's own subtree.
///
/// That is the whole point of `workspace::root_menu`: the left dock clips
/// (`left_panel_body`'s `overflow_hidden`, and the project list scrolls), and
/// a menu deferred from inside that subtree inherits the clip — gpui re-applies
/// the captured `content_mask` when the deferred draw paints, and `hit_test`
/// intersects every hitbox with it, so the overflowing part of the menu is
/// invisible *and* unclickable.
///
/// The press is simulated rather than calling the opener, because calling the
/// opener directly would pass with the wiring absent — which is exactly what
/// was wrong before.
#[gpui::test]
async fn a_dock_row_right_click_deploys_at_the_workspace_root(cx: &mut TestAppContext) {
    let root = std::path::PathBuf::from("/tmp/context_menu_ops_lane");
    let _ = std::fs::create_dir_all(&root);
    let config = daruda_config::Config::default();
    let (window_handle, workspace) = build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(&root)),
    );
    // The press has to land on a real row, and the fixture supplies neither
    // half of that: it boots with the left dock closed, and a lane only exists
    // once one is pushed.
    workspace.update(cx, |ws, cx| {
        if let Some(p) = ws.active_project_mut() {
            p.lanes
                .push(crate::lane::Lane::default_for_project(1, root.clone()));
        }
        ws.left_dock.update(cx, |d, _| d.toggle());
        cx.notify();
    });

    let vcx = &mut gpui::VisualTestContext::from_window(window_handle.into(), cx);
    vcx.run_until_parked();

    let project_id = workspace.read_with(vcx, |ws, _| ws.active_ref().project);
    let selector: &'static str = Box::leak(format!("lane-row-{project_id}-1").into_boxed_str());
    let row = vcx
        .debug_bounds(selector)
        .expect("the lane row renders in the left dock");

    assert!(
        workspace.read_with(vcx, |ws, _| ws.main_area.popup_menu_deploy.is_none()),
        "nothing deployed before the press"
    );

    vcx.simulate_mouse_down(row.center(), gpui::MouseButton::Right, Default::default());
    vcx.run_until_parked();

    let deployed = workspace.read_with(vcx, |ws, _| {
        ws.main_area
            .popup_menu_deploy
            .as_ref()
            .map(|deploy| deploy.position)
    });
    assert_eq!(
        deployed,
        Some(row.center()),
        "the row's menu must deploy at the workspace root, anchored at the press \
         in window coordinates — a subtree-local menu cannot satisfy this"
    );
}
