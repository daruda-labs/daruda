//! Theme bridge — map daruda's palette into `gpui_component`'s `Theme`.
//!
//! `gpui_component` widgets read every color through `cx.theme().<slot>`,
//! so overwriting the global `ThemeColor` after `gpui_component::init`
//! retones every Dialog / Input / Checkbox / Notification at one site
//! without touching widget code or the vendored crate.
//!
//! Called once from `main.rs` at app startup. Subsumes the explicit
//! `Theme::change(Dark, ...)` that the migration earlier wired by hand.
//!
//! Dark-only for now; light-mode mapping is a Phase-10+ follow-up.
//!
//! Sibling [`palette`] holds the app-side UI palette constants
//! (workspace chrome, docks, status bar) that are
//! independent of the terminal-side color model in
//! `daruda_terminal::ux::theme`.
//!
//! This module re-exports both palettes so app-side call sites can
//! write `use crate::ui::theme;` and reach every constant through one
//! path — `theme::TAB_BAR_BG`, `theme::MODAL_PANEL_BG`,
//! `theme::TERMINAL_FG`, etc. The split is purely a code-organization
//! concern; consumers should not need to know whether a given
//! constant lives in [`palette`] or in `daruda_terminal::ux::theme`.

pub mod daruda_theme;
pub mod palette;

pub use daruda_theme::DarudaTheme;

/// Read the currently-installed `DarudaTheme` palette. Wraps
/// `cx.global::<DarudaTheme>()` so call sites read like
/// `theme::current(cx).tab_bar_bg` — visually parallel to the
/// existing `theme::TAB_BAR_BG` const path.
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
use crate::ui::theme as p;
use gpui::{App, hsla, px};
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
    // Snapshot the active DarudaTheme as a value so the subsequent
    // `Theme::global_mut(cx)` mutable borrow doesn't conflict.
    let d = cx.global::<DarudaTheme>().clone();
    let t = Theme::global_mut(cx);

    // ---------------------------------------------------------------
    // Surfaces
    // ---------------------------------------------------------------
    t.background = d.tab_bar_bg;
    t.popover = d.modal_panel_bg;
    t.popover_foreground = d.modal_text_primary;
    t.list = d.tab_bar_bg;
    t.list_head = d.tab_inactive_bg;
    t.list_even = d.modal_panel_bg;
    // Transparent accordion bg so the right-bar Skills tab's plugin
    // groups read as nested rows on the same surface instead of a
    // panel-on-panel sandwich. `hsla(_, _, _, 0.0)` = invisible — the
    // parent's background shows through. `accordion_hover` keeps its
    // subtle tint below for affordance.
    t.accordion = hsla(0.0, 0.0, 0.0, 0.0);
    t.muted = d.tab_inactive_bg;
    t.group_box = d.modal_panel_bg;
    t.group_box_foreground = d.muted_text;

    // ---------------------------------------------------------------
    // Foreground (text)
    // ---------------------------------------------------------------
    t.foreground = d.modal_text_primary;
    t.muted_foreground = d.muted_text;
    t.description_list_label_foreground = d.muted_text;

    // ---------------------------------------------------------------
    // Borders
    // ---------------------------------------------------------------
    t.border = d.modal_panel_border;
    t.input = d.modal_input_border;
    t.drag_border = d.modal_primary_bg;

    // ---------------------------------------------------------------
    // Primary / accent / focus ring
    // ---------------------------------------------------------------
    t.primary = d.modal_primary_bg;
    t.primary_hover = d.modal_primary_hover_bg;
    t.primary_active = d.modal_primary_bg;
    t.primary_foreground = d.modal_text_primary;
    t.accent = d.lane_row_hover_bg;
    t.accent_foreground = d.modal_text_primary;
    t.ring = d.input_focus_border;

    // ---------------------------------------------------------------
    // Info / link — share the primary blue tint.
    // ---------------------------------------------------------------
    t.info = d.modal_primary_bg;
    t.info_hover = d.modal_primary_hover_bg;
    t.info_active = d.modal_primary_bg;
    t.info_foreground = d.modal_text_primary;
    t.link = d.modal_primary_bg;
    t.link_hover = d.modal_primary_hover_bg;
    t.link_active = d.modal_primary_hover_bg;

    // ---------------------------------------------------------------
    // Secondary
    // ---------------------------------------------------------------
    t.secondary = d.button_widget_bg;
    t.secondary_hover = d.button_widget_bg_hover;
    t.secondary_active = d.tab_active_bg;
    t.secondary_foreground = d.modal_secondary_text;

    // ---------------------------------------------------------------
    // List rows / accordion (hover + selection)
    // ---------------------------------------------------------------
    t.list_hover = d.lane_row_hover_bg;
    t.list_active = d.tab_active_bg;
    t.list_active_border = d.modal_primary_bg;
    t.accordion_hover = d.lane_row_hover_bg;

    // ---------------------------------------------------------------
    // Danger / Warning / Success — semantic states. Light mode keeps
    // the same hue family at a slightly darker lightness so the icon
    // / state colour remains readable on the light surface.
    // ---------------------------------------------------------------
    t.danger = hsla(0.0, 0.70, 0.55, 1.0);
    t.danger_hover = hsla(0.0, 0.70, 0.60, 1.0);
    t.danger_active = hsla(0.0, 0.70, 0.45, 1.0);
    t.danger_foreground = d.modal_text_primary;

    t.warning = hsla(40.0 / 360.0, 0.80, 0.55, 1.0);
    t.warning_hover = hsla(40.0 / 360.0, 0.80, 0.65, 1.0);
    t.warning_active = hsla(40.0 / 360.0, 0.80, 0.50, 1.0);
    t.warning_foreground = d.modal_text_primary;

    t.success = d.accent_green;
    t.success_hover = hsla(135.0 / 360.0, 0.55, 0.65, 1.0);
    t.success_active = hsla(135.0 / 360.0, 0.55, 0.50, 1.0);
    t.success_foreground = d.modal_text_primary;

    // ---------------------------------------------------------------
    // Caret / selection / drop target
    // ---------------------------------------------------------------
    t.caret = d.modal_primary_bg;
    t.selection = hsla(210.0 / 360.0, 0.70, 0.55, 0.40);
    t.drop_target = d.lane_drop_target_bg;

    // ---------------------------------------------------------------
    // Tab — daruda renders its own tab strip; these cover gpui_component
    // sub-tab usage (left/right dock view tabs, segmented controls).
    // ---------------------------------------------------------------
    t.tab = d.tab_inactive_bg;
    t.tab_active = d.tab_active_bg;
    t.tab_active_foreground = d.tab_active_text;
    t.tab_bar = d.tab_bar_bg;
    t.tab_bar_segmented = d.tab_inactive_bg;
    t.tab_foreground = d.tab_inactive_text;

    // ---------------------------------------------------------------
    // Scrollbar / progress / skeleton
    // ---------------------------------------------------------------
    t.scrollbar = d.modal_panel_bg;
    t.scrollbar_thumb = d.button_widget_bg;
    t.scrollbar_thumb_hover = d.button_widget_bg_hover;
    t.skeleton = d.button_widget_bg;
    t.progress_bar = d.modal_primary_bg;

    // ---------------------------------------------------------------
    // Switch / slider — muted gray track + bright thumb.
    // ---------------------------------------------------------------
    t.switch = d.button_widget_bg;
    t.switch_thumb = d.modal_text_primary;
    t.slider_bar = d.button_widget_bg;
    t.slider_thumb = d.modal_text_primary;

    // ---------------------------------------------------------------
    // Overlay (backdrop behind dialogs)
    // ---------------------------------------------------------------
    t.overlay = hsla(0.0, 0.0, 0.0, 0.50);

    // ---------------------------------------------------------------
    // Radii
    // ---------------------------------------------------------------
    t.radius = px(p::MODAL_BUTTON_RADIUS);
    t.radius_lg = px(p::MODAL_PANEL_RADIUS);
}
