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
/// CLI token for the mermaid-diagram lightbox scenario.
const NAME_MERMAID_LIGHTBOX: &str = "mermaid-lightbox";

/// A wide sample diagram (two side-by-side subgraphs) — the shape that
/// exercises the lightbox's horizontal scroll/clamp path, not just the
/// common single-column case.
const MERMAID_LIGHTBOX_SAMPLE: &str = concat!(
    "flowchart TD\n",
    "    subgraph ASIS[\"AS-IS\"]\n",
    "        A1[registeredUri] --> A2{\"endsWith('/*')?\"}\n",
    "        A2 -->|no| A3[거부]\n",
    "        A2 -->|yes| A4[\"substring 으로 '/*' 절단\"]\n",
    "        A4 --> A5[parse]\n",
    "        A5 --> A6[\"scheme/host/port/query 일치\"]\n",
    "        A6 --> A7[\"AntPathMatcher.match(pattern, path)\"]\n",
    "        A7 --> A8[허용]\n",
    "    end\n",
    "    subgraph TOBE[\"TO-BE\"]\n",
    "        B1[registeredUri] --> B2[parse]\n",
    "        B2 --> B3{\"path.endsWith('/*')?\"}\n",
    "        B3 -->|no| B4[\"path 정확 일치\"]\n",
    "        B3 -->|yes| B5[\"prefix + 단일 세그먼트 직접 비교\"]\n",
    "        B2 --> B6[\"userInfo/fragment 있으면 거부\"]\n",
    "        B2 --> B7[\"port: 등록에 없으면 any허용\"]\n",
    "        B4 --> B8[허용]\n",
    "        B5 --> B8\n",
    "    end\n",
);

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
    /// Open the mermaid diagram lightbox with a wide (two-subgraph) sample —
    /// the only way to eyeball the clamp/scroll behavior a unit test can
    /// only assert numerically.
    MermaidLightbox,
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
            NAME_MERMAID_LIGHTBOX => Some(Self::MermaidLightbox),
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
        ScreenshotScenario::MermaidLightbox => open_mermaid_lightbox_sample(window, cx),
    }
}

/// Render [`MERMAID_LIGHTBOX_SAMPLE`] and open it in the lightbox — no chat
/// history needed, the same render path the diagram card's zoom button uses.
fn open_mermaid_lightbox_sample(window: &mut Window, cx: &mut App) {
    use super::main_area::agent_chat_pane::render::mermaid_lightbox;
    use super::main_area::file_view_pane::mermaid_theme::MermaidPalette;
    use super::main_area::file_view_pane::render::CachedImage;
    use super::main_area::file_view_pane::visual::render_mermaid_raster;

    let palette = MermaidPalette::default();
    let Some(raster) = render_mermaid_raster(MERMAID_LIGHTBOX_SAMPLE, &palette) else {
        return;
    };
    let Some(image) = CachedImage::from_raster(&raster) else {
        return;
    };
    mermaid_lightbox::open(&image, window, cx);
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
    fn mermaid_lightbox_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("mermaid-lightbox"),
            Some(ScreenshotScenario::MermaidLightbox)
        );
    }

    #[test]
    fn toast_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("toast"),
            Some(ScreenshotScenario::Toast)
        );
    }
}
