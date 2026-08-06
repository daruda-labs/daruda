//! Tests for [`crate::workspace::screenshot_scenario::drive`] — driving a
//! transient overlay into view for `--screenshot` capture.

use gpui::{AppContext as _, TestAppContext};

use super::build_workspace;
use crate::ui::WindowExt as _;
use crate::workspace::screenshot_scenario::{ScreenshotScenario, drive};

#[gpui::test]
async fn drive_command_palette_opens_palette(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::CommandPalette, &workspace, window, cx);
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.command_palette.is_open,
            "command-palette scenario should open the palette",
        );
    });
}

#[gpui::test]
async fn drive_error_modal_opens_dialog(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::ErrorModal, &workspace, window, cx);
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        assert!(
            window.has_active_dialog(cx),
            "error-modal scenario should open a dialog",
        );
    })
    .unwrap();
}

#[gpui::test]
async fn drive_toast_pushes_a_toast(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::Toast, &workspace, window, cx);
    })
    .unwrap();

    workspace.read_with(cx, |ws, cx| {
        assert!(
            !ws.error_toasts(cx).is_empty(),
            "toast scenario should push a toast",
        );
    });
}

#[gpui::test]
async fn drive_pane_context_menu_deploys_the_menu(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::PaneContextMenu, &workspace, window, cx);
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.main_area.popup_menu_deploy.is_some(),
            "pane-context-menu scenario should deploy the pane menu",
        );
    });
}

#[gpui::test]
async fn drive_mermaid_lightbox_opens_dialog(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::MermaidLightbox, &workspace, window, cx);
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, cx| {
        assert!(
            window.has_active_dialog(cx),
            "mermaid-lightbox scenario should open a dialog",
        );
    })
    .unwrap();
}

#[gpui::test]
async fn drive_settings_opens_settings_window(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(
            ScreenshotScenario::Settings(daruda_config::BuiltinSection::General),
            &workspace,
            window,
            cx,
        );
    })
    .unwrap();

    cx.update(|cx| {
        assert!(
            crate::window_registry::WindowRegistry::settings(cx).is_some(),
            "settings scenario should open the Settings window",
        );
    });
}
