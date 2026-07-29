//! Scenario hooks for `--screenshot`. The capture path restores only
//! *persisted* workspace state, so transient overlays (command palette,
//! modals) never appear in a vanilla screenshot. A scenario drives one
//! such overlay into view after the settle delay and just before capture,
//! making those states reachable for visual verification.
//!
//! Parsing of the `--screenshot-scenario <name>` CLI flag lives in
//! `crate::screenshot`; this module owns the scenario enum, its CLI-name
//! mapping, and the workspace-driving dispatch ([`drive`]).

use gpui::{App, Entity, Point, Window, px};

use super::{ToggleCommandPalette, Workspace, dialog_helpers};
use daruda_config::BuiltinSection;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

/// CLI token for the command-palette scenario.
const NAME_COMMAND_PALETTE: &str = "command-palette";
/// CLI token for the error-report-modal scenario.
const NAME_ERROR_MODAL: &str = "error-modal";
/// CLI token for the error-toast scenario.
const NAME_TOAST: &str = "toast";
/// CLI token for the Settings-window scenario. Bare opens the default section;
/// `settings:<slug>` opens a specific section (e.g. `settings:font`).
const NAME_SETTINGS: &str = "settings";
/// CLI token for the pane context-menu scenario.
const NAME_PANE_CONTEXT_MENU: &str = "pane-context-menu";

/// Where the pane menu is deployed for the capture, in window coordinates.
/// Near the top-left of the content area so the menu opens downward at its
/// natural length — the shot is meant to show every entry, not the edge flip.
const PANE_MENU_ANCHOR_X: f32 = 320.;
const PANE_MENU_ANCHOR_Y: f32 = 160.;

/// A transient UI state to drive into view before a `--screenshot` capture.
/// One scenario per capture — these overlays are mutually exclusive on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotScenario {
    /// Open the command palette (`CommandPaletteState::open`).
    CommandPalette,
    /// Open the Layer-2 error-report modal with a synthetic report.
    ErrorModal,
    /// Push a synthetic error toast.
    Toast,
    /// Open the Settings window at the given section.
    Settings(BuiltinSection),
    /// Deploy the focused pane's right-click menu. The only way to eyeball
    /// menu length, edge-flip and the keybinding column — none of which any
    /// unit test can see.
    PaneContextMenu,
}

impl ScreenshotScenario {
    /// Map a CLI token to a scenario. Unknown tokens return `None`.
    /// `settings:<slug>` selects a Settings section; bare `settings` uses the
    /// default section.
    pub(crate) fn from_cli_name(name: &str) -> Option<Self> {
        match name {
            NAME_COMMAND_PALETTE => Some(Self::CommandPalette),
            NAME_ERROR_MODAL => Some(Self::ErrorModal),
            NAME_TOAST => Some(Self::Toast),
            NAME_SETTINGS => Some(Self::Settings(BuiltinSection::default())),
            NAME_PANE_CONTEXT_MENU => Some(Self::PaneContextMenu),
            _ => name
                .strip_prefix(concat!("settings", ":"))
                .and_then(BuiltinSection::from_slug)
                .map(Self::Settings),
        }
    }
}

/// Drive `scenario` into view on `workspace`'s `window`. Called from the
/// screenshot capture path with the live workspace window in scope.
pub(crate) fn drive(
    scenario: ScreenshotScenario,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    match scenario {
        ScreenshotScenario::CommandPalette => {
            workspace.update(cx, |ws, cx| {
                ws.on_toggle_command_palette(&ToggleCommandPalette, window, cx);
            });
        }
        ScreenshotScenario::ErrorModal => {
            dialog_helpers::open_error_report_dialog(sample_report(), window, cx);
        }
        ScreenshotScenario::Toast => {
            workspace.update(cx, |ws, cx| ws.report_error(sample_report(), cx));
        }
        ScreenshotScenario::Settings(section) => {
            crate::windows::open_settings_window(section, window, cx);
        }
        ScreenshotScenario::PaneContextMenu => {
            workspace.update(cx, |ws, cx| {
                let pane_id = ws.active_runtime().focused_pane_id;
                let anchor = Point::new(px(PANE_MENU_ANCHOR_X), px(PANE_MENU_ANCHOR_Y));
                ws.open_pane_context_menu_at(pane_id, anchor, window, cx);
            });
        }
    }
}

/// Synthetic report for the error-modal scenario — representative of a real
/// Layer-2 details view (title, message, context table, source location).
fn sample_report() -> ErrorReport {
    ErrorReport::new("Screenshot scenario")
        .severity(ErrorSeverity::Error)
        .message("Synthetic error report for visual verification")
        .at(file!(), line!())
        .with_context("scenario", "error-modal")
        .dedup("screenshot.scenario.error_modal")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_palette_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("command-palette"),
            Some(ScreenshotScenario::CommandPalette)
        );
    }

    #[test]
    fn error_modal_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("error-modal"),
            Some(ScreenshotScenario::ErrorModal)
        );
    }

    #[test]
    fn unknown_name_maps_to_none() {
        assert_eq!(ScreenshotScenario::from_cli_name("nope"), None);
        assert_eq!(ScreenshotScenario::from_cli_name(""), None);
    }

    #[test]
    fn bare_settings_maps_to_default_section() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings"),
            Some(ScreenshotScenario::Settings(BuiltinSection::default()))
        );
    }

    #[test]
    fn settings_slug_maps_to_that_section() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings:font"),
            Some(ScreenshotScenario::Settings(BuiltinSection::Font))
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings:notifications"),
            Some(ScreenshotScenario::Settings(BuiltinSection::Notifications))
        );
    }

    #[test]
    fn unknown_settings_slug_maps_to_none() {
        assert_eq!(ScreenshotScenario::from_cli_name("settings:bogus"), None);
    }

    #[test]
    fn toast_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("toast"),
            Some(ScreenshotScenario::Toast)
        );
    }
}
