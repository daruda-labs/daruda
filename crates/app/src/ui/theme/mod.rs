//! Theme bridge — map daruda's palette into `gpui_component`'s `Theme`.
//!
//! `gpui_component` widgets read every color through `cx.theme().<slot>`,
//! so overwriting the global `ThemeColor` after `gpui_component::init`
//! retones every Dialog / Input / Checkbox / Notification at one site
//! without touching widget code or the vendored crate.
//!
//! Called once from `main.rs` at app startup.
//!
//! Sibling [`palette`] holds the app-side UI palette constants
//! (workspace chrome, docks, status bar) that are
//! independent of the terminal-side color model in
//! `daruda_terminal::ux::theme`.
//!
//! This module re-exports both palettes so app-side call sites can
//! write `use crate::ui::theme;` and reach every constant through one
//! path — `theme::SURFACE_1`, `theme::MODAL_PANEL_BG`,
//! `theme::TERMINAL_FG`, etc. The split is purely a code-organization
//! concern; consumers should not need to know whether a given
//! constant lives in [`palette`] or in `daruda_terminal::ux::theme`.

pub mod daruda_theme;
pub mod palette;

pub use daruda_theme::DarudaTheme;

/// Read the currently-installed `DarudaTheme` palette. Wraps
/// `cx.global::<DarudaTheme>()` so call sites read like
/// `theme::current(cx).tab_bar_bg` — visually parallel to the
/// existing `theme::SURFACE_1` const path.
///
/// Phase 3-C migrates the workspace-chrome call sites (tab strip,
/// pane header, dock, status bar, lanes list) to this helper;
/// the long tail of modal / agent / right-panel sites keeps reading
/// the underlying `palette::FOO` const until a follow-up pass moves
/// them too. Both paths return the same colour today — Phase 3-D
/// wires JSON loading so the helper starts returning user-authored
/// overrides while the const path keeps the compiled-in default.
pub fn current(cx: &gpui::App) -> &DarudaTheme {
    cx.global::<DarudaTheme>()
}

/// JSON body for a built-in UI theme preset. Bundled with the binary
/// via `include_str!` so the loader cannot fail with a missing file
/// on a fresh install.
///
/// Returns `None` for names not in `daruda_config::UI_THEME_PRESETS`
/// — the caller (Settings save path, `Workspace::apply_config`) is
/// expected to leave the live theme untouched on an unknown name,
/// the same fall-through behaviour `theme.terminal_preset` already
/// uses for unknown preset names.
fn bundled_theme_json(name: &str) -> Option<&'static str> {
    match name {
        "daruda_dark" => Some(include_str!(
            "../../../../../assets/themes/daruda_dark.json"
        )),
        "daruda_light" => Some(include_str!(
            "../../../../../assets/themes/daruda_light.json"
        )),
        _ => None,
    }
}

/// Install the named UI theme as the live `DarudaTheme` Global.
///
/// - Looks up the bundled JSON via [`bundled_theme_json`].
/// - Parses through `DarudaTheme::from_json`; missing keys in the
///   JSON file fall through to the compile-time dark default
///   thanks to the struct-level `#[serde(default)]`. This is what
///   makes a *partial* `daruda_light.json` valid — it only needs to
///   list the slots that differ from dark.
/// - Replaces the Global wholesale and calls `cx.refresh_windows()`
///   so every visible entity repaints. Without the refresh, a
///   running window keeps painting the previous theme until its
///   next independent invalidation.
///
/// Returns `false` and leaves the live theme untouched if `name` is
/// not bundled or the JSON fails to parse. The caller is responsible
/// for surfacing user feedback (Settings UI logs a warning, the
/// reload path is a no-op).
pub fn apply_ui_theme(name: &str, cx: &mut gpui::App) -> bool {
    let Some(json) = bundled_theme_json(name) else {
        return false;
    };
    let parsed = match DarudaTheme::from_json(json) {
        Ok(t) => t,
        Err(_) => return false,
    };
    cx.set_global(parsed);
    // Re-bridge the new DarudaTheme into `gpui_component::Theme` so
    // every wrapped widget (Input / Select / Button / Dialog / TabBar
    // / Tooltip / …) picks up the light-mode tones. Without this the
    // bespoke daruda surfaces flip but every gpui_component-rendered
    // input box, button, dropdown and modal chrome keeps its dark
    // palette.
    apply_daruda_palette(cx);
    cx.refresh_windows();
    true
}

pub use crate::ui::theme::palette::*;
pub use daruda_terminal::ux::theme::*;

// Bridge implementation reads every constant through the unified
// `crate::ui::theme` surface so the underlying split between
// `palette` and `daruda_terminal::ux::theme` is transparent.
use std::sync::Arc;

use crate::ui::theme as p;
use gpui::{App, px};
use gpui_component::highlighter::HighlightTheme;
use gpui_component::{Theme, ThemeMode};

/// Idempotent installer — ensures `gpui_component::Theme` Global
/// exists and the daruda palette is layered over it. Production
/// (`main.rs`) and test fixtures (`test_support::init_gpui_component`)
/// register the Globals explicitly; this entry point exists for paths
/// that build a Workspace without those (e.g. tests that drive
/// `Workspace::new_with_project` directly). Whichever runs first wins,
/// the other is a no-op.
pub fn init_if_missing(cx: &mut App) {
    if !cx.has_global::<Theme>() {
        gpui_component::init(cx);
        // DarudaTheme must be registered before apply_daruda_palette;
        // the palette reads `cx.global::<DarudaTheme>()` to bridge
        // slots into `gpui_component::Theme`.
        DarudaTheme::init(cx);
        apply_daruda_palette(cx);
    }
}

/// Apply the daruda palette over `gpui_component`'s default `ThemeColor`.
/// Reads the active `DarudaTheme` Global (set by either the default
/// dark fall-through or a loaded `daruda_light.json`), so calling this
/// after a `cx.set_global::<DarudaTheme>` is what flips every
/// gpui_component-rendered widget (Input / Select / Button / Dialog /
/// TabBar / Tooltip / Scrollbar / Switch / Slider / …) into the new
/// tone. Without this re-bridge the bespoke daruda surfaces respond
/// to theme switches but every wrapped gpui_component widget keeps the
/// palette that was active at app start.
pub fn apply_daruda_palette(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    // Snapshot values before the `Theme::global_mut(cx)` mutable borrow.
    let d = cx.global::<DarudaTheme>().clone();
    let palette = active_syntax_palette(cx);
    // Pick the light or dark variant of the syntax palette to match the
    // active UI theme's editor background, so syntax stays legible on light.
    let syntax_is_light = !d.is_dark();
    // Light-aware surface picks. gpui_component-themed surfaces (base bg,
    // lists, tabs, selected rows) were hard-locked to the dark consts, so
    // they rendered dark on the light theme. Pick the cool-tinted light
    // ladder when the active theme is light.
    let s_canvas = if syntax_is_light {
        LIGHT_CANVAS
    } else {
        CANVAS
    };
    let s_surface_1 = if syntax_is_light {
        LIGHT_SURFACE_1
    } else {
        SURFACE_1
    };
    let t = Theme::global_mut(cx);

    // ---------------------------------------------------------------
    // Surfaces
    // ---------------------------------------------------------------
    t.background = s_surface_1;
    t.popover = d.modal_panel_bg;
    t.popover_foreground = d.text_primary;
    t.list = s_surface_1;
    t.list_head = d.tab_inactive_bg;
    t.list_even = d.modal_panel_bg;
    // Transparent accordion bg so the right-bar Skills tab's plugin
    // groups read as nested rows on the same surface instead of a
    // panel-on-panel sandwich. `hsla(_, _, _, 0.0)` = invisible — the
    // parent's background shows through. `accordion_hover` keeps its
    // subtle tint below for affordance.
    t.accordion = TRANSPARENT;
    t.muted = d.tab_inactive_bg;
    t.group_box = d.modal_panel_bg;
    t.group_box_foreground = d.text_muted;

    // ---------------------------------------------------------------
    // Foreground (text)
    // ---------------------------------------------------------------
    t.foreground = d.text_primary;
    t.muted_foreground = d.text_muted;
    t.description_list_label_foreground = d.text_muted;

    // ---------------------------------------------------------------
    // Borders
    // ---------------------------------------------------------------
    t.border = d.border;
    t.input = d.border;
    t.drag_border = PRIMARY;

    // ---------------------------------------------------------------
    // Primary / accent / focus ring
    // ---------------------------------------------------------------
    t.primary = PRIMARY;
    t.primary_hover = ACCENT_HOVER;
    t.primary_active = PRIMARY;
    t.primary_foreground = TEXT_PRIMARY;
    t.accent = d.lane_row_hover_bg;
    t.accent_foreground = d.text_primary;
    t.ring = PRIMARY;

    // ---------------------------------------------------------------
    // Info / link — share the primary blue tint.
    // ---------------------------------------------------------------
    t.info = PRIMARY;
    t.info_hover = ACCENT_HOVER;
    t.info_active = PRIMARY;
    t.info_foreground = TEXT_PRIMARY;
    t.link = PRIMARY;
    t.link_hover = ACCENT_HOVER;
    t.link_active = ACCENT_HOVER;

    // ---------------------------------------------------------------
    // Secondary
    // ---------------------------------------------------------------
    t.secondary = d.button_widget_bg;
    t.secondary_hover = d.button_widget_bg_hover;
    t.secondary_active = s_canvas;
    t.secondary_foreground = d.text_body;

    // ---------------------------------------------------------------
    // List rows / accordion (hover + selection)
    // ---------------------------------------------------------------
    t.list_hover = d.lane_row_hover_bg;
    t.list_active = s_canvas;
    t.list_active_border = PRIMARY;
    t.accordion_hover = d.lane_row_hover_bg;

    // ---------------------------------------------------------------
    // Danger / Warning / Success — semantic states. Light mode keeps
    // the same hue family at a slightly darker lightness so the icon
    // / state colour remains readable on the light surface.
    // ---------------------------------------------------------------
    t.danger = ERROR;
    t.danger_hover = with_lightness(ERROR, 0.68);
    t.danger_active = with_lightness(ERROR, 0.53);
    t.danger_foreground = TEXT_PRIMARY;

    t.warning = WARNING;
    t.warning_hover = with_lightness(WARNING, 0.68);
    t.warning_active = with_lightness(WARNING, 0.53);
    t.warning_foreground = TEXT_PRIMARY;

    t.success = SUCCESS;
    t.success_hover = with_lightness(SUCCESS, 0.59);
    t.success_active = with_lightness(SUCCESS, 0.44);
    t.success_foreground = TEXT_PRIMARY;

    // ---------------------------------------------------------------
    // Caret / selection / drop target
    // ---------------------------------------------------------------
    t.caret = PRIMARY;
    t.selection = SELECTION_BG;
    t.drop_target = d.lane_drop_target_bg;

    // ---------------------------------------------------------------
    // Tab — daruda renders its own tab strip; these cover gpui_component
    // sub-tab usage (left/right dock view tabs, segmented controls).
    // ---------------------------------------------------------------
    t.tab = d.tab_inactive_bg;
    t.tab_active = s_canvas;
    t.tab_active_foreground = d.text_primary;
    t.tab_bar = s_surface_1;
    t.tab_bar_segmented = d.tab_inactive_bg;
    t.tab_foreground = d.text_muted;

    // ---------------------------------------------------------------
    // Scrollbar / progress / skeleton
    // ---------------------------------------------------------------
    t.scrollbar = d.modal_panel_bg;
    t.scrollbar_thumb = d.button_widget_bg;
    t.scrollbar_thumb_hover = d.button_widget_bg_hover;
    t.skeleton = d.button_widget_bg;
    t.progress_bar = PRIMARY;

    // ---------------------------------------------------------------
    // Switch / slider — muted gray track + bright thumb.
    // ---------------------------------------------------------------
    t.switch = d.button_widget_bg;
    t.switch_thumb = TEXT_PRIMARY;
    t.slider_bar = d.button_widget_bg;
    t.slider_thumb = TEXT_PRIMARY;

    // ---------------------------------------------------------------
    // Overlay (backdrop behind dialogs)
    // ---------------------------------------------------------------
    t.overlay = with_alpha(CANVAS, MODAL_BACKDROP_ALPHA);

    // ---------------------------------------------------------------
    // Radii
    // ---------------------------------------------------------------
    t.radius = px(p::MODAL_BUTTON_RADIUS);
    t.radius_lg = px(p::MODAL_PANEL_RADIUS);

    // ---------------------------------------------------------------
    // Code highlighting — install daruda's `base16-ocean.dark` syntax
    // palette so the raw editor (gpui_component `highlight_theme`) and the
    // diff view (`syntax_color`) share one colour source (`syntax_theme`).
    // Without this the editor keeps gpui_component's default theme and
    // diverges from the diff view.
    // ---------------------------------------------------------------
    let mut highlight = (*HighlightTheme::default_dark()).clone();
    highlight.style.editor_foreground = Some(p::syntax_theme_of(palette, syntax_is_light).default);
    highlight.style.syntax = p::editor_syntax_colors_of(palette, syntax_is_light);
    // `default_dark()` carries a dark `editor_background`, which the editor
    // element paints behind the line-number gutter (and ghost rows) via
    // `cx.theme().editor_background()`. Left unset it stays dark on the light
    // theme — a black gutter stripe beside the light content. Pin it to the
    // file-viewer surface so the gutter matches the content in both modes.
    highlight.style.editor_background = Some(d.file_viewer_bg);
    // The default-dark active-line band is dark; on a light editor it reads
    // as a black stripe. Use a faint cool-light band in light mode.
    if syntax_is_light {
        highlight.style.editor_active_line = Some(LIGHT_SURFACE_1);
    }
    t.highlight_theme = Arc::new(highlight);
}

/// The active syntax palette, mirrored from `config.file_viewer.syntax_theme`
/// into a Global so [`apply_daruda_palette`] (which has no config access — it
/// also runs on light/dark theme switches) can re-seed the editor highlight
/// colours without losing the user's selection. Single update site:
/// [`set_active_syntax_palette`], called from the config-reload path.
#[derive(Clone, Copy, Default)]
struct ActiveSyntaxPalette(p::SyntaxPalette);

impl gpui::Global for ActiveSyntaxPalette {}

/// Read the active syntax palette (defaults to the recommended
/// [`SyntaxPalette::Daruda`](p::SyntaxPalette::Daruda) before any config load).
pub fn active_syntax_palette(cx: &App) -> p::SyntaxPalette {
    cx.try_global::<ActiveSyntaxPalette>()
        .map(|g| g.0)
        .unwrap_or_default()
}

/// Set the active syntax palette and re-bridge the editor highlight theme so
/// the raw editor picks up the new colours. The single source remains the
/// config string; this mirrors the resolved selection for the GPUI side.
pub fn set_active_syntax_palette(cx: &mut App, palette: p::SyntaxPalette) {
    cx.set_global(ActiveSyntaxPalette(palette));
    apply_daruda_palette(cx);
}
