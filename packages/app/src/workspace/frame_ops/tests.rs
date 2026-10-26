use crate::workspace::layout::DockPosition;
use crate::workspace::tests::build_workspace;
use gpui::{AppContext as _, MouseButton, MouseMoveEvent, TestAppContext, point, px};

#[gpui::test]
fn frame_drag_uses_each_docks_axis_and_direction(cx: &mut TestAppContext) {
    let (handle, workspace) = build_workspace(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            for (position, expected) in [
                (DockPosition::Left, 330.0),
                (DockPosition::Right, 270.0),
                (DockPosition::Bottom, 220.0),
            ] {
                let dock = match position {
                    DockPosition::Left => &ws.docks.left,
                    DockPosition::Right => &ws.docks.right,
                    DockPosition::Bottom => &ws.docks.bottom,
                }
                .clone();
                dock.update(cx, |dock, _| {
                    dock.open();
                    dock.resize(300.0);
                });
                ws.begin_dock_drag(position, 100.0, cx);
                ws.on_frame_mouse_move(
                    &MouseMoveEvent {
                        position: point(px(130.0), px(180.0)),
                        pressed_button: Some(MouseButton::Left),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                assert_eq!(dock.read(cx).size, expected, "{position:?}");
                ws.end_dock_drag(cx);
            }
        });
    })
    .unwrap();
}

#[gpui::test]
fn frame_reentry_without_a_pressed_button_settles_resize_without_moving_it(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = build_workspace(cx);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.docks.right.update(cx, |dock, _| {
                dock.open();
                dock.resize(300.0);
            });
            ws.begin_dock_drag(DockPosition::Right, 100.0, cx);
            ws.on_frame_mouse_move(
                &MouseMoveEvent {
                    position: point(px(180.0), px(130.0)),
                    pressed_button: None,
                    ..Default::default()
                },
                window,
                cx,
            );
            assert!(ws.docks.drag.is_none(), "outside release ends the drag");
            assert_eq!(ws.docks.right.read(cx).size, 300.0);
        });
    })
    .unwrap();
}
