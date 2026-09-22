//! Tests for [`crate::workspace::screenshot_scenario::drive`] — driving a
//! transient overlay into view for `--screenshot` capture.

use gpui::{AppContext as _, TestAppContext};

use super::{build_workspace, build_workspace_with};
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

/// The scenario is the only way to see the Windows/Linux title bar from a
/// macOS host, so it has to actually flip the field the render reads.
#[gpui::test]
async fn drive_client_chrome_switches_the_window_chrome(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::ClientChrome, &workspace, window, cx);
    })
    .unwrap();

    cx.update_window(window_handle.into(), |_, window, _| {
        assert!(
            crate::title_bar::chrome_for_window(window).is_client(),
            "the scenario left the window on its host chrome",
        );
    })
    .unwrap();
}

#[gpui::test]
async fn drive_settings_shows_the_settings_view(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(
            ScreenshotScenario::Settings(daruda_config::BuiltinSection::General),
            &workspace,
            window,
            cx,
        );
        assert!(
            workspace.read(cx).settings.is_some(),
            "settings scenario should put the Settings view on screen",
        );
    })
    .unwrap();
}

/// The banner seed is the whole point of this scenario, and the only thing
/// that could quietly drop it is the view not being on screen by the time
/// `drive` reaches for it.
#[gpui::test]
async fn drive_settings_error_raises_the_banner(cx: &mut TestAppContext) {
    let (window_handle, workspace) = build_workspace(cx);

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::SettingsError, &workspace, window, cx);
        let view = workspace
            .read(cx)
            .settings
            .as_ref()
            .expect("settings-error scenario should put the Settings view on screen")
            .view
            .clone();
        assert!(
            view.read(cx).error_for_test().is_some(),
            "the scenario must leave a banner for the capture to show"
        );
    })
    .unwrap();
}

/// The one hazard in the long-label seed: it must swap the *label* of a real
/// candidate, not fabricate a row. A synthetic `lane_ref` would render fine
/// and then activate nothing when the row is picked.
#[gpui::test]
async fn drive_lane_switcher_reuses_a_real_lane_ref(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path("/tmp/daruda_lane_switcher_shot");
    let (window_handle, workspace) = build_workspace_with(cx, &config, Some(project));

    cx.update_window(window_handle.into(), |_, window, cx| {
        drive(ScreenshotScenario::LaneSwitcher, &workspace, window, cx);
    })
    .unwrap();

    workspace.read_with(cx, |ws, _| {
        assert!(
            ws.lane_switcher.is_open,
            "lane-switcher scenario should open the switcher",
        );
        let seeded = ws
            .lane_switcher
            .candidates
            .first()
            .expect("the seeded switcher lists at least one lane");
        let lane_ref = seeded.lane_ref;

        let project = ws
            .project_for(lane_ref.project)
            .expect("the seeded row names a project that exists");
        assert!(
            project.lane(lane_ref.lane).is_some(),
            "the seeded row must reuse a real lane_ref, not a fabricated one",
        );

        // And the label really was swapped — otherwise the capture shows a
        // short row and proves nothing about clipping.
        assert_ne!(
            seeded.label,
            ws.lane_label_for(lane_ref),
            "the seeded row should carry the synthetic long label",
        );
        // Guard against shortening the sample until it no longer exercises
        // popup clipping.
        assert!(
            seeded.label.len() > 80,
            "the synthetic label must overflow the popup: {}",
            seeded.label,
        );
    });
}
