//! Theme bridge from daruda palettes to `gpui_component::Theme`.
//!
//! Installed once after `gpui_component::init`, retinting upstream widgets at
//! the global theme slot layer. This module re-exports app and terminal palette
//! constants so call sites can use one `crate::ui::theme` path.

pub mod daruda_theme;
pub mod palette;

pub use daruda_theme::DarudaTheme;

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

/// File-viewer / editor font size in points, mirrored from config
/// `font.editor_size`. Drives the raw + diff editor body, the markdown
/// raw rows, and the line-number gutter so they share one size that is
/// independent of the terminal font. Single update site:
/// [`set_editor_font_size`], called from the config-reload path.
#[derive(Clone, Copy)]
struct EditorFontSize(f32);

impl gpui::Global for EditorFontSize {}

/// Read the file-viewer editor font size (points). Defaults to
/// [`FILE_VIEWER_FONT_SIZE`](p::FILE_VIEWER_FONT_SIZE) before any config load.
pub fn editor_font_size(cx: &App) -> f32 {
    cx.try_global::<EditorFontSize>()
        .map(|g| g.0)
        .unwrap_or(p::FILE_VIEWER_FONT_SIZE)
}

/// Mirror the resolved config `font.editor_size` for the GPUI side. The
/// config string stays the single source; this caches the value for the
/// file-viewer render path.
pub fn set_editor_font_size(cx: &mut App, size: f32) {
    cx.set_global(EditorFontSize(size));
}

/// Agent-chat font size in points, mirrored from config `font.agent_chat_size`.
/// Drives the entire conversation pane — message bodies, headers, tool titles,
/// and chrome share one size (body and label unified) that is independent of
/// the terminal and editor fonts. Single update site:
/// [`set_agent_chat_font_size`], called from the config-reload path.
#[derive(Clone, Copy)]
struct AgentChatFontSize(f32);

impl gpui::Global for AgentChatFontSize {}

/// Read the agent-chat font size (points). Defaults to
/// [`AGENT_CHAT_MSG_FONT_SIZE`](p::AGENT_CHAT_MSG_FONT_SIZE) before any config
/// load. Every agent-chat text size resolves through this so the whole pane
/// scales as one.
pub fn agent_chat_font_size(cx: &App) -> f32 {
    cx.try_global::<AgentChatFontSize>()
        .map(|g| g.0)
        .unwrap_or(p::AGENT_CHAT_MSG_FONT_SIZE)
}

/// Mirror the resolved config `font.agent_chat_size` for the GPUI side. The
/// config string stays the single source; this caches the value for the
/// agent-chat render path.
pub fn set_agent_chat_font_size(cx: &mut App, size: f32) {
    cx.set_global(AgentChatFontSize(size));
}

/// Pane width at or below which the Activity Bar's three transcript chips
/// collapse into one view-options gear.
///
/// The two parts are text widths measured at
/// [`AGENT_CHAT_MSG_FONT_SIZE`](p::AGENT_CHAT_MSG_FONT_SIZE), and the pane's
/// size is user-configurable (`font.agent_chat_size`, clamped 6–72), so the
/// threshold has to scale with it. A fixed breakpoint reads as derived while
/// silently assuming one font: at 20px the spelled-out chips outgrow the budget,
/// the bar stays wide, and the cluster ellipsizes them instead of collapsing —
/// exactly the state the split exists to avoid.
///
/// Padding is a fixed metric and does not scale.
pub fn agent_chat_compact_options_w(cx: &App) -> f32 {
    let scale = agent_chat_font_size(cx) / p::AGENT_CHAT_MSG_FONT_SIZE;
    (p::AGENT_CHAT_TITLE_MIN_W + p::AGENT_CHAT_OPTIONS_CLUSTER_W) * scale
        + 2.0 * p::AGENT_CHAT_PAD_X
}

/// Agent-chat reading-mode content width in pixels, mirrored from config
/// `agent.reading_width`. Single update site:
/// [`set_agent_chat_reading_width`], called from startup and config reload.
#[derive(Clone, Copy)]
struct AgentChatReadingWidth(f32);

impl gpui::Global for AgentChatReadingWidth {}

/// Read the AgentChat reading-mode content width. Defaults to the config
/// default before startup mirrors are seeded.
pub fn agent_chat_reading_width(cx: &App) -> f32 {
    cx.try_global::<AgentChatReadingWidth>()
        .map(|g| g.0)
        .unwrap_or(daruda_config::READING_WIDTH_DEFAULT)
}

/// Mirror the resolved config `agent.reading_width` for AgentChat render.
pub fn set_agent_chat_reading_width(cx: &mut App, width: f32) {
    cx.set_global(AgentChatReadingWidth(width));
}

/// Background opacity (0.1–1.0), mirrored from config `window.opacity` — the
/// same value that drives terminal-pane background translucency. Lets the
/// agent-chat pane render its background at the window opacity so it matches
/// the terminal when the window is transparent/blurred. Single update site:
/// [`set_background_alpha`], called from the config-reload path.
#[derive(Clone, Copy)]
struct BackgroundAlpha(f32);

impl gpui::Global for BackgroundAlpha {}

/// Read the background opacity. Defaults to `1.0` (fully opaque) before any
/// config load, so panes render solid until the config mirrors a lower value.
pub fn background_alpha(cx: &App) -> f32 {
    cx.try_global::<BackgroundAlpha>()
        .map(|g| g.0)
        .unwrap_or(1.0)
}

/// Mirror the resolved config `window.opacity` for the GPUI side. The config
/// value stays the single source; this caches it for the agent-chat render
/// path (which reads the global directly, like [`editor_font_size`]).
pub fn set_background_alpha(cx: &mut App, alpha: f32) {
    cx.set_global(BackgroundAlpha(alpha));
}

/// Terminal-theme background color, mirrored from config
/// `effective_colors().background` (i.e. `[colors]` + `[theme].terminal_preset`)
/// — the same value that fills the terminal pane. Lets the agent-chat pane
/// render on the terminal color theme rather than the UI theme's editor
/// surface, so the two panes match. Stored as raw RGB (the channel the
/// terminal fill uses) and converted to `Hsla` on read. Single update sites:
/// the construction seed in `globals` + the config-reload path.
#[derive(Clone, Copy, PartialEq)]
struct AgentChatBg {
    r: u8,
    g: u8,
    b: u8,
}

impl gpui::Global for AgentChatBg {}

/// Cohesive color set for pane-local surfaces that follow the terminal
/// color theme rather than the workspace chrome theme.
#[derive(Clone, Copy)]
pub struct PaneSurfaceTokens {
    pub background: gpui::Hsla,
    pub foreground: gpui::Hsla,
    pub foreground_muted: gpui::Hsla,
    pub foreground_subtle: gpui::Hsla,
    pub tint: gpui::Hsla,
    pub active_tint: gpui::Hsla,
    pub border_tint: gpui::Hsla,
    /// Resting edge for an interactive control on this surface — heavier than
    /// [`Self::border_tint`], which edges cards. See
    /// [`AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK`](p::AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK).
    pub control_border: gpui::Hsla,
    pub syntax_is_light: bool,
}

impl PaneSurfaceTokens {
    pub fn agent_chat(cx: &App) -> Self {
        Self::from_background_and_foreground(agent_chat_bg(cx), agent_chat_fg(cx))
    }

    pub fn file_viewer(cx: &App) -> Self {
        Self::from_background_and_foreground(file_viewer_pane_bg(cx), agent_chat_fg(cx))
    }

    /// The flow graph's canvas. Same source as the two above — a graph opened
    /// from a transcript should not land on a surface unrelated to the pane it
    /// came from.
    pub fn flow_graph(cx: &App) -> Self {
        Self::from_background_and_foreground(agent_chat_bg(cx), agent_chat_fg(cx))
    }

    /// The same surface dimmed for an inactive pane. Every token moves
    /// together, so a control built from these dims in step with the text
    /// beside it instead of staying at full strength.
    pub fn dimmed(self, amount: f32) -> Self {
        Self {
            background: dim_toward_gray(self.background, amount),
            foreground: dim_toward_gray(self.foreground, amount),
            foreground_muted: dim_toward_gray(self.foreground_muted, amount),
            foreground_subtle: dim_toward_gray(self.foreground_subtle, amount),
            tint: dim_toward_gray(self.tint, amount),
            active_tint: dim_toward_gray(self.active_tint, amount),
            border_tint: dim_toward_gray(self.border_tint, amount),
            control_border: dim_toward_gray(self.control_border, amount),
            syntax_is_light: self.syntax_is_light,
        }
    }

    fn from_background_and_foreground(background: gpui::Hsla, foreground: gpui::Hsla) -> Self {
        let overlay = neutral_overlay_for(background);
        // Darkening a near-white surface buys less contrast than lightening a
        // near-black one, so the control edge takes its alpha per direction.
        let control_alpha = if background.l < 0.5 {
            p::AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK
        } else {
            p::AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_LIGHT
        };
        Self {
            background,
            foreground,
            foreground_muted: foreground.opacity(p::AGENT_CHAT_FG_MUTED_ALPHA),
            foreground_subtle: foreground.opacity(p::AGENT_CHAT_FG_SUBTLE_ALPHA),
            tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_TINT_ALPHA),
            active_tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_BORDER_ALPHA),
            border_tint: p::with_alpha(overlay, p::AGENT_CHAT_CARD_BORDER_ALPHA),
            control_border: p::with_alpha(overlay, control_alpha),
            syntax_is_light: background.l >= 0.5,
        }
    }
}

/// Read the agent-chat background color. Defaults to [`BG_EDITOR`](p::BG_EDITOR)
/// before any config load, so the pane renders on the editor surface until the
/// terminal color is mirrored in.
pub fn agent_chat_bg(cx: &App) -> gpui::Hsla {
    match cx.try_global::<AgentChatBg>() {
        Some(c) => gpui::Rgba {
            r: f32::from(c.r) / 255.0,
            g: f32::from(c.g) / 255.0,
            b: f32::from(c.b) / 255.0,
            a: 1.0,
        }
        .into(),
        None => p::BG_EDITOR,
    }
}

/// File-viewer pane background. This is intentionally scoped to the file
/// viewer's own body/editor/toolbar/search-panel surfaces, not workspace chrome
/// such as the tab strip. It follows the same terminal-mirrored colour source
/// as Agent Chat so file links opened from a transcript do not land on a
/// visually unrelated editor surface. Kept opaque because editor internals
/// repaint sub-areas such as the gutter on top of the pane surface.
pub fn file_viewer_pane_bg(cx: &App) -> gpui::Hsla {
    agent_chat_bg(cx)
}

/// Mirror the resolved terminal background color for the agent-chat render
/// path. Returns `true` when the value changed, so the reload path can decide
/// whether to repaint the cached agent-chat views.
pub fn set_agent_chat_bg(cx: &mut App, r: u8, g: u8, b: u8) -> bool {
    let next = AgentChatBg { r, g, b };
    let changed = cx.try_global::<AgentChatBg>() != Some(&next);
    cx.set_global(next);
    changed
}

/// Agent-chat foreground color, mirrored from the terminal color theme's
/// `effective_colors().foreground` — the counterpart to [`AgentChatBg`]. The
/// pane already renders its background on the terminal color theme; mirroring
/// the terminal foreground for the pane's text (prose, headers, summaries)
/// keeps glyph color consistent with the background instead of pulling from the
/// UI theme's text ramp. Code-block syntax highlighting is intentionally
/// unaffected — that follows the selectable syntax palette. Stored as raw RGB;
/// converted to `Hsla` on read. Single update sites: the `globals` seed + the
/// config-reload path.
#[derive(Clone, Copy, PartialEq)]
struct AgentChatFg {
    r: u8,
    g: u8,
    b: u8,
}

impl gpui::Global for AgentChatFg {}

/// Read the agent-chat foreground color. Mirrors the terminal color theme's
/// foreground, then lifts its lightness by [`AGENT_CHAT_FG_BRIGHTEN`](p::AGENT_CHAT_FG_BRIGHTEN)
/// so chat text reads a touch brighter than the raw terminal glyph **without
/// touching the terminal itself** (the terminal renders from `terminal_config`,
/// not this global). The lift is scaled by how dark the pane background is
/// (`1 - bg.l`), so it brightens light-on-dark text (the common case) but
/// barely nudges dark-on-light text — where "brighter" would instead cut
/// contrast. Defaults to [`TEXT_PRIMARY`](p::TEXT_PRIMARY) (already bright)
/// before any config load.
pub fn agent_chat_fg(cx: &App) -> gpui::Hsla {
    match cx.try_global::<AgentChatFg>() {
        Some(c) => {
            let mut hsla: gpui::Hsla = gpui::Rgba {
                r: f32::from(c.r) / 255.0,
                g: f32::from(c.g) / 255.0,
                b: f32::from(c.b) / 255.0,
                a: 1.0,
            }
            .into();
            let bg_l = agent_chat_bg(cx).l;
            hsla.l = (hsla.l + p::AGENT_CHAT_FG_BRIGHTEN * (1.0 - bg_l)).min(1.0);
            hsla
        }
        None => p::TEXT_PRIMARY,
    }
}

/// Dimmed agent-chat foreground for secondary text (muted labels, status).
/// Derived by blending the terminal foreground toward the pane background via
/// alpha, so it dims relative to the *actual* terminal background on any theme
/// (a fixed UI-theme grey would be wrong on a light or strongly tinted
/// terminal). Replaces `DarudaTheme::text_muted` in the agent-chat pane.
pub fn agent_chat_fg_muted(cx: &App) -> gpui::Hsla {
    agent_chat_fg(cx).opacity(p::AGENT_CHAT_FG_MUTED_ALPHA)
}

/// Most-dimmed agent-chat foreground for tertiary text (collapsed summaries,
/// disclosure chevrons). The subtle step of the terminal-foreground ramp;
/// replaces `DarudaTheme::text_subtle` in the agent-chat pane. See
/// [`agent_chat_fg_muted`] for why this blends toward the background.
pub fn agent_chat_fg_subtle(cx: &App) -> gpui::Hsla {
    agent_chat_fg(cx).opacity(p::AGENT_CHAT_FG_SUBTLE_ALPHA)
}

/// File-viewer pane foreground. Mirrors Agent Chat's terminal-derived text
/// ramp so file-viewer chrome tracks live terminal theme changes.
pub fn file_viewer_pane_fg(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground
}

/// Secondary file-viewer pane foreground for counters and inactive controls.
pub fn file_viewer_pane_fg_muted(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground_muted
}

/// Tertiary file-viewer pane foreground for low-emphasis labels.
pub fn file_viewer_pane_fg_subtle(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).foreground_subtle
}

/// Mirror the resolved terminal foreground color for the agent-chat render
/// path. Returns `true` when the value changed, so the reload path can decide
/// whether to repaint the cached agent-chat views.
pub fn set_agent_chat_fg(cx: &mut App, r: u8, g: u8, b: u8) -> bool {
    let next = AgentChatFg { r, g, b };
    let changed = cx.try_global::<AgentChatFg>() != Some(&next);
    cx.set_global(next);
    changed
}

fn neutral_overlay_for(bg: gpui::Hsla) -> gpui::Hsla {
    if bg.l < 0.5 {
        p::OVERLAY_WHITE
    } else {
        p::OVERLAY_BLACK
    }
}

/// Background-derived elevation tint for the agent-chat tool cards. A
/// translucent neutral overlay picked by the pane background's *lightness*
/// (white over a dark background, black over a light one), so a tool card
/// reads one step above the pane on any background color — and, being
/// translucent, keeps the pane's window opacity showing through. Mirrors the
/// inline-code tint in the vendored `text/node.rs`.
pub fn agent_chat_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::agent_chat(cx).tint
}

/// Background-derived border for the agent-chat tool cards — the same neutral
/// overlay as [`agent_chat_tint`] but one step stronger, so the hairline edge
/// tracks the pane background instead of a fixed line color. Pairs with the
/// fill tint on the same card.
pub fn agent_chat_border_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::agent_chat(cx).border_tint
}

/// Background-derived tint for file-viewer pane chrome, including the toolbar,
/// mode chips, and floating search panel. Uses the same neutral-overlay rule as
/// Agent Chat so the two pane types move together under live terminal theme
/// changes without leaking this colour into the workspace tab strip.
pub fn file_viewer_pane_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).tint
}

/// Stronger background-derived tint for active controls inside the file-viewer
/// pane, such as the Raw/Preview/Changes mode chips.
pub fn file_viewer_pane_active_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).active_tint
}

/// Hairline tint for file-viewer pane chrome.
pub fn file_viewer_pane_border_tint(cx: &App) -> gpui::Hsla {
    PaneSurfaceTokens::file_viewer(cx).border_tint
}

/// Whether agent-chat content should pick the *light* variant of a
/// light/dark-aware palette (diff/markdown syntax highlighting, mermaid
/// diagrams, the diff embed's own fallback text colour) — judged by
/// [`agent_chat_bg`]'s own lightness, not the UI theme's `DarudaTheme::is_dark`.
/// The pane's actual paint surface is the terminal-preset background mirrored
/// into `agent_chat_bg`, which can disagree with the UI theme on light vs
/// dark; content painted on it needs to match *that* background. Single
/// source shared by `Workspace::agent_chat_theme_params` (feeds the diff/
/// mermaid reconcilers) and `ui::code_editor`'s agent-chat diff viewer.
pub fn agent_chat_syntax_is_light(cx: &App) -> bool {
    PaneSurfaceTokens::agent_chat(cx).syntax_is_light
}

pub fn file_viewer_pane_syntax_is_light(cx: &App) -> bool {
    PaneSurfaceTokens::file_viewer(cx).syntax_is_light
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn agent_chat_syntax_is_light_switches_at_the_midpoint(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(!agent_chat_syntax_is_light(cx), "unset default is dark");

            set_agent_chat_bg(cx, 0, 0, 0);
            assert!(!agent_chat_syntax_is_light(cx), "black background is dark");

            set_agent_chat_bg(cx, 255, 255, 255);
            assert!(agent_chat_syntax_is_light(cx), "white background is light");

            // The finest boundary an 8-bit RGB mirror can actually produce:
            // grayscale lightness is `v / 255`, so 127 sits just under the
            // `l >= 0.5` cutoff and 128 sits just over it. Matches
            // `agent_chat_tint`'s `l < 0.5` dark-branch cutoff — both treat
            // `l == 0.5` as the light side.
            set_agent_chat_bg(cx, 127, 127, 127);
            assert!(!agent_chat_syntax_is_light(cx), "l=127/255 is still dark");

            set_agent_chat_bg(cx, 128, 128, 128);
            assert!(agent_chat_syntax_is_light(cx), "l=128/255 crosses to light");
        });
    }

    /// Composite `fg` over `bg` and return the WCAG contrast ratio. The
    /// agent-chat foreground ramp is alpha-based, so an uncomposited pair
    /// would measure the wrong thing.
    fn contrast_over(fg: gpui::Hsla, bg: gpui::Hsla) -> f32 {
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

    /// Controls on the agent-chat bar take their colour from this surface, and
    /// the surface mirrors the *terminal* palette — which the UI theme knows
    /// nothing about. A control coloured from the UI theme instead landed at
    /// ~1.1:1 here under a light UI over a dark pane.
    #[gpui::test]
    fn the_pane_surface_ramp_stays_readable_on_its_own_background(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for (bg, fg) in [
                ((17u8, 17u8, 17u8), (216u8, 216u8, 216u8)),
                ((250, 250, 250), (40, 40, 40)),
            ] {
                set_agent_chat_bg(cx, bg.0, bg.1, bg.2);
                set_agent_chat_fg(cx, fg.0, fg.1, fg.2);
                let s = PaneSurfaceTokens::agent_chat(cx);
                // Body text at AA (4.5:1); the muted tier carries chips, icon
                // glyphs and secondary labels, so it is held to the 3:1 floor
                // WCAG sets for UI components.
                for (name, color, floor) in [
                    ("foreground", s.foreground, 4.5),
                    ("muted", s.foreground_muted, 3.0),
                ] {
                    let ratio = contrast_over(color, s.background);
                    assert!(ratio >= floor, "{name} on {bg:?} measures {ratio:.2}:1");
                }
            }
        });
    }

    /// The vendored `selected_foreground` slot exists *only* because no single
    /// foreground clears both of a segmented strip's contrast pairs. Assert both,
    /// or a palette move silently invalidates the patch's whole justification.
    #[gpui::test]
    fn a_segmented_strip_clears_both_of_its_contrast_pairs(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            init_if_missing(cx);
            // Read the popover fill through the *bridged* theme, so this tracks
            // whatever `apply_daruda_palette` actually assigns rather than a
            // second guess at it. Today that is `surface-2`, not the `surface-4`
            // DESIGN.md's elevation table nominally gives popovers.
            let popover = Theme::global(cx).popover;
            let t = current(cx);
            let resting = contrast_over(t.text_muted, popover);
            assert!(resting >= 4.5, "resting label measures {resting:.2}:1");
            // Selected label on the accent fill.
            let selected = contrast_over(p::ACCENT_FG, p::PRIMARY);
            assert!(selected >= 4.5, "selected label measures {selected:.2}:1");
            // The pair the patch rules out: one shared foreground cannot do both.
            let shared = contrast_over(t.text_muted, p::PRIMARY);
            assert!(
                shared < 4.5,
                "if the resting tone also worked on the fill ({shared:.2}:1) the \
                 patch would be unnecessary"
            );
            // And the reason accent is not the resting label colour.
            let accent_as_text = contrast_over(p::PRIMARY, popover);
            assert!(
                accent_as_text < 4.5,
                "accent as a segment label measures {accent_as_text:.2}:1"
            );
        });
    }

    /// The two parts of the breakpoint are text widths, so the threshold has to
    /// track the pane's configured font size. A fixed number would read as
    /// derived while assuming one font.
    #[gpui::test]
    fn the_compact_breakpoint_scales_with_the_pane_font(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            // Unset: the compile-time default, so the documented value holds.
            let default = agent_chat_compact_options_w(cx);
            assert!(
                (default - p::AGENT_CHAT_COMPACT_OPTIONS_W).abs() < 0.01,
                "at the default size the constant is the threshold, got {default}"
            );

            set_agent_chat_font_size(cx, p::AGENT_CHAT_MSG_FONT_SIZE * 2.0);
            let doubled = agent_chat_compact_options_w(cx);
            assert!(
                doubled > default,
                "a larger pane font needs a wider pane before the chips fit"
            );
            // Only the text parts scale; the fixed padding stays put.
            let text_part = default - 2.0 * p::AGENT_CHAT_PAD_X;
            assert!(
                (doubled - (text_part * 2.0 + 2.0 * p::AGENT_CHAT_PAD_X)).abs() < 0.01,
                "padding must not scale with the font, got {doubled}"
            );

            set_agent_chat_font_size(cx, 6.0);
            assert!(
                agent_chat_compact_options_w(cx) < default,
                "the smallest configurable font lets a narrower pane keep the chips"
            );
        });
    }

    /// A chip's resting edge is the only thing that says it is a control
    /// rather than one of the static readouts beside it on the Activity Bar,
    /// so it is held to DESIGN.md's 3:1 component-edge floor. The card tint
    /// that edges tool cards measures 1.44:1 here and is *not* enough — the
    /// two are separate tokens for exactly this reason.
    #[gpui::test]
    fn a_control_edge_on_the_pane_surface_clears_the_component_floor(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            // Both sanctioned directions: the default terminal preset's
            // background and a light one.
            for bg in [(30u8, 30u8, 30u8), (249u8, 250u8, 251u8)] {
                set_agent_chat_bg(cx, bg.0, bg.1, bg.2);
                let s = PaneSurfaceTokens::agent_chat(cx);
                let control = contrast_over(s.control_border, s.background);
                assert!(
                    control >= 3.0,
                    "control edge on {bg:?} measures {control:.2}:1"
                );
                let card = contrast_over(s.border_tint, s.background);
                assert!(
                    card < control,
                    "the card edge must stay the lighter of the two, got \
                     card {card:.2}:1 vs control {control:.2}:1"
                );
            }
        });
    }

    /// A dimmed surface must move each token exactly as the bar's own text
    /// moves, or a control built from it stays at full strength while the
    /// title beside it fades.
    #[gpui::test]
    fn dimming_a_surface_matches_dimming_each_token_by_hand(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            set_agent_chat_bg(cx, 17, 17, 17);
            set_agent_chat_fg(cx, 216, 216, 216);
            for amount in [0.0, 0.4, 1.0] {
                let dimmed = PaneSurfaceTokens::agent_chat(cx).dimmed(amount);
                assert_eq!(
                    dimmed.foreground,
                    dim_toward_gray(agent_chat_fg(cx), amount)
                );
                assert_eq!(
                    dimmed.foreground_muted,
                    dim_toward_gray(agent_chat_fg_muted(cx), amount)
                );
                assert_eq!(
                    dimmed.border_tint,
                    dim_toward_gray(agent_chat_border_tint(cx), amount)
                );
                assert_eq!(
                    dimmed.control_border,
                    dim_toward_gray(PaneSurfaceTokens::agent_chat(cx).control_border, amount)
                );
            }
        });
    }

    /// The built-in gpui_component scrollbar draws its thumb over popover /
    /// modal surfaces (dropdown menus, dialogs). A thumb mapped to an opaque
    /// surface-ladder gray was byte-identical to the popover background —
    /// painted but invisible (the status-bar Ports dropdown regression).
    /// The bridge must keep the thumb visually distinct on those surfaces.
    #[gpui::test]
    fn scrollbar_thumb_contrasts_with_popover_surface(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            init_if_missing(cx);
            let t = Theme::global(cx);
            assert_ne!(
                t.scrollbar_thumb, t.popover,
                "thumb color equals the popover background — invisible scrollbar"
            );
            assert!(
                t.scrollbar_thumb.a < 1.0,
                "thumb should be translucent so it reads on any surface rung"
            );
        });
    }

    /// `apply_daruda_palette` maps ~80 slots by hand; nothing structural
    /// stops a foreground slot from landing on its own surface color (the
    /// invisible-scrollbar class of bug, above). Pin every foreground /
    /// surface pair gpui_component widgets actually paint text with. This
    /// only rejects the "same slot mapped twice" failure — not low
    /// contrast in general — which is exactly the class the bridge has
    /// shipped before.
    #[gpui::test]
    fn bridged_foreground_surface_pairs_stay_distinct(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            init_if_missing(cx);
            let t = Theme::global(cx);
            let pairs = [
                ("foreground on background", t.foreground, t.background),
                (
                    "popover_foreground on popover",
                    t.popover_foreground,
                    t.popover,
                ),
                ("muted_foreground on muted", t.muted_foreground, t.muted),
                ("accent_foreground on accent", t.accent_foreground, t.accent),
                (
                    "secondary_foreground on secondary",
                    t.secondary_foreground,
                    t.secondary,
                ),
                (
                    "primary_foreground on primary",
                    t.primary_foreground,
                    t.primary,
                ),
                ("danger_foreground on danger", t.danger_foreground, t.danger),
                (
                    "tab_active_foreground on tab_active",
                    t.tab_active_foreground,
                    t.tab_active,
                ),
                (
                    "group_box_foreground on group_box",
                    t.group_box_foreground,
                    t.group_box,
                ),
            ];
            for (name, fg, bg) in pairs {
                assert_ne!(fg, bg, "{name}: text color equals its surface — invisible");
            }
        });
    }
}
