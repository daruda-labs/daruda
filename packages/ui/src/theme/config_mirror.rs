//! Config values mirrored into GPUI globals for render paths: syntax palette,
//! editor and agent-chat fonts, reading width, and background opacity.

use super::*;

/// The active syntax palette, mirrored from `config.file_viewer.syntax_theme`
/// into a Global so [`apply_daruda_palette`] (which has no config access — it
/// also runs on light/dark theme switches) can re-seed the editor highlight
/// colours without losing the user's selection. Single update site:
/// [`set_active_syntax_palette`], called from the config-reload path.
#[derive(Clone, Copy, Default)]
struct ActiveSyntaxPalette(p::SyntaxPalette);

impl gpui::Global for ActiveSyntaxPalette {}

/// Read the active syntax palette (defaults to the recommended
/// [`SyntaxPalette::Daruda`] before any config load).
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

/// File-viewer / editor font size in points, mirrored from config.
#[derive(Clone, Copy)]
struct EditorFontSize(f32);

impl gpui::Global for EditorFontSize {}

/// Read the file-viewer editor font size (points). Defaults to
/// [`FILE_VIEWER_FONT_SIZE`] before any config load.
pub fn editor_font_size(cx: &App) -> f32 {
    cx.try_global::<EditorFontSize>()
        .map(|g| g.0)
        .unwrap_or(p::FILE_VIEWER_FONT_SIZE)
}

pub fn set_editor_font_size(cx: &mut App, size: f32) {
    cx.set_global(EditorFontSize(size));
}

#[derive(Clone)]
struct EditorFontFamily(gpui::SharedString);

impl gpui::Global for EditorFontFamily {}

pub fn editor_font_family(cx: &App) -> gpui::SharedString {
    cx.try_global::<EditorFontFamily>()
        .map(|font| font.0.clone())
        .unwrap_or_else(|| daruda_config::FontConfig::default().editor.family.into())
}

pub fn set_editor_font_family(cx: &mut App, family: impl Into<gpui::SharedString>) {
    cx.set_global(EditorFontFamily(family.into()));
}

#[derive(Clone, Copy)]
struct EditorLineHeight(f32);

impl gpui::Global for EditorLineHeight {}

pub fn editor_line_height(cx: &App) -> f32 {
    cx.try_global::<EditorLineHeight>()
        .map(|height| height.0)
        .unwrap_or(p::FILE_VIEWER_LINE_H_RATIO)
}

pub fn set_editor_line_height(cx: &mut App, line_height: f32) {
    cx.set_global(EditorLineHeight(line_height));
}

/// Agent-chat font size in points, mirrored from config.
#[derive(Clone, Copy)]
struct AgentChatFontSize(f32);

impl gpui::Global for AgentChatFontSize {}

/// Read the agent-chat font size (points). Defaults to
/// [`AGENT_CHAT_MSG_FONT_SIZE`] before any config
/// load. Every agent-chat text size resolves through this so the whole pane
/// scales as one.
pub fn agent_chat_font_size(cx: &App) -> f32 {
    cx.try_global::<AgentChatFontSize>()
        .map(|g| g.0)
        .unwrap_or(p::AGENT_CHAT_MSG_FONT_SIZE)
}

pub fn set_agent_chat_font_size(cx: &mut App, size: f32) {
    cx.set_global(AgentChatFontSize(size));
}

#[derive(Clone)]
struct AgentChatFontFamily(gpui::SharedString);

impl gpui::Global for AgentChatFontFamily {}

pub fn agent_chat_font_family(cx: &App) -> gpui::SharedString {
    cx.try_global::<AgentChatFontFamily>()
        .map(|font| font.0.clone())
        .unwrap_or_else(|| {
            daruda_config::FontConfig::default()
                .agent_chat
                .family
                .into()
        })
}

pub fn set_agent_chat_font_family(cx: &mut App, family: impl Into<gpui::SharedString>) {
    cx.set_global(AgentChatFontFamily(family.into()));
}

#[derive(Clone, Copy)]
struct AgentChatLineHeight(f32);

impl gpui::Global for AgentChatLineHeight {}

pub fn agent_chat_line_height(cx: &App) -> f32 {
    cx.try_global::<AgentChatLineHeight>()
        .map(|height| height.0)
        .unwrap_or(p::MD_VIEW_LINE_HEIGHT)
}

pub fn set_agent_chat_line_height(cx: &mut App, line_height: f32) {
    cx.set_global(AgentChatLineHeight(line_height));
}

/// Painted row height for code editors embedded in agent-chat cards.
/// GPUI snaps relative line heights to whole logical pixels, so the reserved
/// viewport uses the same ceiling and never loses a row at the configured cap.
pub fn agent_chat_embed_row_height(cx: &App) -> f32 {
    (agent_chat_font_size(cx) * agent_chat_line_height(cx))
        .ceil()
        .max(1.0)
}

/// Pane width at or below which the View button omits its label.
/// Text-derived widths scale with the configurable pane font; padding stays fixed.
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
