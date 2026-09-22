//! Settings as a body-level view of the workspace window: it goes up, it comes
//! down, and the pane that had focus gets it back.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::workspace::OpenSettings;

#[gpui::test]
async fn open_settings_puts_the_view_on_screen(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            assert!(ws.settings.is_none(), "a fresh workspace shows no settings");
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::Font),
                window,
                cx,
            );
            let host = ws.settings.as_ref().expect("settings should be on screen");
            assert_eq!(
                host.view.read(cx).active_section(),
                daruda_config::BuiltinSection::Font,
                "the action's section is the one shown",
            );
        });
    })
    .unwrap();
}

/// A second dispatch must move the open view rather than build another — the
/// first one holds the user's in-flight edits.
#[gpui::test]
async fn reopening_moves_the_same_view(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::Font),
                window,
                cx,
            );
            let first = ws
                .settings
                .as_ref()
                .expect("settings on screen")
                .view
                .clone();

            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::About),
                window,
                cx,
            );
            let host = ws.settings.as_ref().expect("settings still on screen");
            assert_eq!(
                host.view.entity_id(),
                first.entity_id(),
                "reopening should reuse the live view",
            );
            assert_eq!(
                host.view.read(cx).active_section(),
                daruda_config::BuiltinSection::About,
            );
        });
    })
    .unwrap();
}

/// The view asks to go away by emitting; nothing else may take it down, so the
/// subscription is what this covers.
#[gpui::test]
async fn the_view_s_close_event_takes_it_down(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    let view = cx
        .update_window(window_handle.into(), |_, window, cx| {
            workspace.update(cx, |ws, cx| {
                ws.on_open_settings(
                    &OpenSettings(daruda_config::BuiltinSection::General),
                    window,
                    cx,
                );
                ws.settings
                    .as_ref()
                    .expect("settings on screen")
                    .view
                    .clone()
            })
        })
        .unwrap();

    cx.update_window(window_handle.into(), |_, _window, cx| {
        view.update(cx, |_, cx| {
            cx.emit(crate::settings::SettingsEvent::Close);
        });
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(window_handle.into(), |_, _window, cx| {
        assert!(
            workspace.read(cx).settings.is_none(),
            "Close should take the view down",
        );
    })
    .unwrap();
}

/// A chord that drives a dock must not land while Settings covers it. The
/// registration is skipped wholesale in that mode, so this is the check that
/// the skip actually reaches the dispatch tree rather than just the source.
#[gpui::test]
async fn settings_mode_does_not_answer_a_dock_action(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);
    let mut vcx = gpui::VisualTestContext::from_window(window_handle.into(), cx);

    // Control: outside Settings the same dispatch flips the dock.
    let before = workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open);
    vcx.dispatch_action(crate::workspace::ToggleLeftDock);
    vcx.run_until_parked();
    let toggled = workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open);
    assert_ne!(before, toggled, "the dock action works outside Settings");

    vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.on_open_settings(
                &OpenSettings(daruda_config::BuiltinSection::General),
                window,
                cx,
            );
        });
    });
    vcx.run_until_parked();

    vcx.dispatch_action(crate::workspace::ToggleLeftDock);
    vcx.run_until_parked();

    assert_eq!(
        workspace.read_with(&vcx, |ws, cx| ws.left_dock.read(cx).is_open),
        toggled,
        "Settings must swallow the dock chord, not pass it to a hidden dock",
    );
}
