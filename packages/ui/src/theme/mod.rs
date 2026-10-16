//! Theme bridge from daruda palettes to `gpui_component::Theme`.
//!
//! Installed once after `gpui_component::init`, retinting upstream widgets at
//! the global theme slot layer. This module re-exports app and terminal palette
//! constants so call sites can use one `crate::ui::theme` path.

pub mod daruda_theme;
pub mod list_metrics;
pub mod metrics;
pub mod palette;
mod sizing;
pub mod syntax;
pub use sizing::UiMetrics;

mod config_mirror;
mod pane_surface;

pub use config_mirror::{
    active_syntax_palette, agent_chat_compact_options_w, agent_chat_embed_row_height,
    agent_chat_font_family, agent_chat_font_size, agent_chat_line_height, agent_chat_reading_width,
    background_alpha, editor_font_family, editor_font_size, editor_line_height,
    set_active_syntax_palette, set_agent_chat_font_family, set_agent_chat_font_size,
    set_agent_chat_line_height, set_agent_chat_reading_width, set_background_alpha,
    set_editor_font_family, set_editor_font_size, set_editor_line_height,
};
pub use daruda_theme::DarudaTheme;
pub use pane_surface::{
    PaneSurfaceTokens, agent_chat_bg, agent_chat_border_tint, agent_chat_fg, agent_chat_fg_muted,
    agent_chat_fg_subtle, agent_chat_hover_tint, agent_chat_link_color, agent_chat_syntax_is_light,
    agent_chat_tint, file_viewer_pane_active_tint, file_viewer_pane_bg,
    file_viewer_pane_border_tint, file_viewer_pane_fg, file_viewer_pane_fg_muted,
    file_viewer_pane_fg_subtle, file_viewer_pane_link_color, file_viewer_pane_syntax_is_light,
    file_viewer_pane_tint, set_agent_chat_bg, set_agent_chat_fg,
};

/// Read the currently installed `DarudaTheme` palette.
pub fn current(cx: &gpui::App) -> &DarudaTheme {
    cx.global::<DarudaTheme>()
}

/// Dim RGB toward gray while preserving alpha; unlike a scrim, this keeps
/// translucent panes translucent.
pub fn dim_toward_gray(color: gpui::Hsla, amount: f32) -> gpui::Hsla {
    if amount <= 0.0 {
        return color;
    }
    let amount = amount.min(1.0);
    let rgba = gpui::Rgba::from(color);
    let g = palette::DIM_GRAY_LEVEL;
    gpui::Rgba {
        r: rgba.r * (1.0 - amount) + g * amount,
        g: rgba.g * (1.0 - amount) + g * amount,
        b: rgba.b * (1.0 - amount) + g * amount,
        a: rgba.a,
    }
    .into()
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
        "daruda_dark" => Some(include_str!("../../../../assets/themes/daruda_dark.json")),
        "daruda_light" => Some(include_str!("../../../../assets/themes/daruda_light.json")),
        _ => None,
    }
}

/// WCAG contrast ratio of `fg` over `bg`, compositing `fg`'s alpha. Test-only:
/// the palette's contrast guards are assertions, not a runtime concern.
#[cfg(any(test, feature = "test-support"))]
pub fn contrast_ratio(fg: gpui::Hsla, bg: gpui::Hsla) -> f32 {
    let (fg, bg) = (gpui::Rgba::from(fg), gpui::Rgba::from(bg));
    let channel = |f: f32, b: f32| {
        let v = f * fg.a + b * (1.0 - fg.a);
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let lum = |r: f32, g: f32, b: f32| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let l_fg = lum(
        channel(fg.r, bg.r),
        channel(fg.g, bg.g),
        channel(fg.b, bg.b),
    );
    let l_bg = lum(
        channel(bg.r, bg.r),
        channel(bg.g, bg.g),
        channel(bg.b, bg.b),
    );
    (l_fg.max(l_bg) + 0.05) / (l_fg.min(l_bg) + 0.05)
}

/// Install the named UI theme as the live `DarudaTheme` Global. The raw
/// installer: app code goes through the `UiThemeState` setters below, which
/// call this only when the painted preset moves; tests call it directly.
///
/// - Looks up the bundled JSON via `bundled_theme_json`.
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
/// not bundled or the JSON fails to parse — which leaves `UiThemeState`'s
/// `applied` unset, so the next reconcile tries again.
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

/// The bundled preset `configured` (a `theme.ui_preset` value) paints under
/// `appearance` — `system` follows it, any other name ignores it.
pub fn effective_ui_preset(configured: &str, appearance: gpui::WindowAppearance) -> &str {
    let dark = matches!(
        appearance,
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
    );
    daruda_config::ui_theme_presets::resolve(configured, dark)
}

/// Which UI theme is live, and why — the one owner of that answer.
///
/// Three inputs decide it: the configured `theme.ui_preset`, the OS
/// appearance a `system` preset follows, and a screenshot's capture override.
/// Each setter records its input and reconciles, and the bundled theme is
/// installed only when what they resolve to differs from what is live. So an
/// unrelated settings edit, or one OS flip heard by every window, costs no
/// reparse and no `refresh_windows` (Pitfall 10).
///
/// The appearance is only ever *told* to this state. On Linux the platform
/// stays borrowed while it delivers a flip, so asking it again from inside a
/// window observer panics; zed keeps the same global.
struct UiThemeState {
    configured: String,
    appearance: gpui::WindowAppearance,
    capture_override: Option<&'static str>,
    applied: Option<String>,
}

impl gpui::Global for UiThemeState {}

impl UiThemeState {
    fn painted(&self) -> &str {
        self.capture_override
            .unwrap_or_else(|| effective_ui_preset(&self.configured, self.appearance))
    }
}

/// Seed the state from the platform and the configured preset, and install
/// it. Startup only — the one place the platform is asked.
pub fn init_ui_theme(configured: &str, cx: &mut gpui::App) {
    let appearance = cx.window_appearance();
    cx.set_global(UiThemeState {
        configured: configured.to_owned(),
        appearance,
        capture_override: None,
        applied: None,
    });
    reconcile_ui_theme(cx);
}

/// The configured preset changed — or may have; unchanged is free.
pub fn set_configured_ui_preset(configured: &str, cx: &mut gpui::App) {
    update_ui_theme(cx, |state| {
        if state.configured != configured {
            state.configured = configured.to_owned();
        }
    });
}

/// The OS appearance a window observer was just handed.
pub fn note_system_appearance(appearance: gpui::WindowAppearance, cx: &mut gpui::App) {
    update_ui_theme(cx, |state| state.appearance = appearance);
}

/// Paint `preset` regardless of config and appearance, or stop (`None`).
/// For captures, which must not write the user's config.
pub fn set_capture_ui_preset(preset: Option<&'static str>, cx: &mut gpui::App) {
    update_ui_theme(cx, |state| state.capture_override = preset);
}

/// The bundled preset `configured` paints right now: the capture override,
/// else `configured` resolved against the last appearance told. Before
/// [`init_ui_theme`] — a test fixture — that is a light appearance.
pub fn painted_ui_preset(configured: &str, cx: &gpui::App) -> String {
    match cx.try_global::<UiThemeState>() {
        Some(state) => state
            .capture_override
            .unwrap_or_else(|| effective_ui_preset(configured, state.appearance))
            .to_owned(),
        None => effective_ui_preset(configured, gpui::WindowAppearance::default()).to_owned(),
    }
}

/// Apply `change` and reconcile. A no-op before [`init_ui_theme`]: fixtures
/// that never installed the theme globals must not have one installed.
fn update_ui_theme(cx: &mut gpui::App, change: impl FnOnce(&mut UiThemeState)) {
    if !cx.has_global::<UiThemeState>() {
        return;
    }
    change(cx.global_mut::<UiThemeState>());
    reconcile_ui_theme(cx);
}

fn reconcile_ui_theme(cx: &mut gpui::App) {
    let state = cx.global::<UiThemeState>();
    let next = state.painted().to_owned();
    if state.applied.as_deref() == Some(next.as_str()) {
        return;
    }
    if apply_ui_theme(&next, cx) {
        cx.global_mut::<UiThemeState>().applied = Some(next);
    }
}

pub use crate::theme::palette::*;
pub use crate::theme::syntax::*;
pub use daruda_terminal::ux::theme::*;

// Bridge implementation reads every constant through the unified
// `crate::ui::theme` surface so the underlying split between
// `palette` and `daruda_terminal::ux::theme` is transparent.
use std::sync::Arc;

use crate::theme as p;
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
    // `Theme::popover` is the vendor's one float slot: `popover_style` (the
    // Popover panel, every PopupMenu, the input completion and hover popovers),
    // the tooltip, and — via a patch — the dialog all read it.
    t.popover = d.float_panel_bg;
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
    // Info keeps the primary blue tint; link takes its own pastel so
    // small body text clears contrast (DESIGN §Saturation on dark
    // surfaces). All three link slots share one value — the `Link`
    // widget derives its own hover feedback by opacity, and a hover
    // that jumped back to `accent` would darken the link instead.
    // ---------------------------------------------------------------
    t.info = PRIMARY;
    t.info_hover = ACCENT_HOVER;
    t.info_active = PRIMARY;
    t.info_foreground = TEXT_PRIMARY;
    t.link = d.link_color;
    t.link_hover = d.link_color;
    t.link_active = d.link_color;

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
    // Transparent track: daruda's design draws a thumb only (the app-side
    // custom scrollbar `crate::ui::scrollbar` has no track), so the built-in
    // gpui_component scrollbar's track bar must not paint a background.
    t.scrollbar = gpui::transparent_black();
    // Same slot the app-side custom thumbs read (translucent white) — an
    // opaque surface-ladder gray vanishes on popover/modal surfaces that sit
    // on the same rung (popover bg is byte-identical to `button_widget_bg`).
    t.scrollbar_thumb = d.scrollbar_thumb;
    // Match the app-side custom scrollbar's hover (white-45%) so the built-in
    // scrollbar reads the same on hover as the panes' thumb.
    t.scrollbar_thumb_hover = d.file_viewer_scrollbar_thumb_hover;
    // Always-visible thumb (no idle fade) like the panes' persistent custom
    // scrollbar. The renderer still hides the thumb when content fits (scroll
    // area <= container), so short, non-scrollable lists show nothing.
    t.scrollbar_show = gpui_component::scroll::ScrollbarShow::Always;
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
    // theme — a black gutter stripe beside the light content. Keep the global
    // default on the UI editor surface; hosts with a different surface can
    // override it per input instance.
    highlight.style.editor_background = Some(d.file_viewer_bg);
    // The current-line band is one App-wide slot shared by every editor
    // instance (the File viewer's UI-themed surface *and* the agent-chat
    // diff embed's terminal-preset-derived background — see
    // `theme::agent_chat_bg`), so it can't be tuned to match either
    // surface's exact color. A translucent neutral overlay (the same
    // white-lift/black-recess technique as `agent_chat_tint`) instead of a
    // fixed solid color reads reasonably on any background under it,
    // regardless of which surface a given editor instance is painting on.
    let active_line_overlay = if syntax_is_light {
        p::OVERLAY_BLACK
    } else {
        p::OVERLAY_WHITE
    };
    highlight.style.editor_active_line = Some(p::with_alpha(
        active_line_overlay,
        p::EDITOR_ACTIVE_LINE_ALPHA,
    ));
    t.highlight_theme = Arc::new(highlight);
}

#[cfg(test)]
mod tests;
