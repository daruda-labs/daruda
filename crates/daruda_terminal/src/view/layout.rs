use gpui::Window;

use super::TerminalView;

/// Physical dimensions of one terminal grid cell, with all spacing
/// multipliers already applied. Single source of truth for grid geometry
/// shared by resize, prepaint, and mouse-event coordinate mapping.
#[derive(Clone, Copy)]
pub struct TerminalLayout {
    /// Scaled cell width in points (`horizontal_spacing` applied).
    pub cell_width: f32,
    /// Scaled line height in points (`vertical_spacing` applied).
    pub line_height: f32,
}

impl TerminalLayout {
    /// Number of fully visible columns in the given pixel width.
    pub fn cols(&self, avail_w: f32) -> u16 {
        // avail_w / cell_width can be N − 1 ULP when both values come from
        // float arithmetic (e.g. 8.0 * 100 / 8.0 → 99.999…). next_up()
        // advances one ULP before floor(), matching Zed's verified fix.
        (avail_w / self.cell_width).next_up().floor().max(1.0) as u16
    }

    /// Number of fully visible rows in the given pixel height.
    pub fn rows(&self, avail_h: f32) -> u16 {
        (avail_h / self.line_height).next_up().floor().max(1.0) as u16
    }
}

/// Stable hash of the glyph-identifying parts of a `Font`. Feeds
/// `line_layout_key` so changes to font family, fallbacks, weight,
/// style, or OpenType features invalidate the shape cache.
pub(crate) fn font_hash(font: &gpui::Font) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    font.family.hash(&mut hasher);
    format!("{:?}", font.fallbacks).hash(&mut hasher);
    format!("{:?}", font.features).hash(&mut hasher);
    format!("{:?}", font.weight).hash(&mut hasher);
    format!("{:?}", font.style).hash(&mut hasher);
    hasher.finish()
}

impl TerminalView {
    /// Grid cell geometry at the view's actual font settings.
    ///
    /// Single source of truth shared by resize, prepaint, and mouse coordinate
    /// mapping. Event handlers run outside the paint cycle where
    /// `window.text_style()` returns GPUI's root 1rem (16 px) rather than the
    /// per-pane `font_size`; all callers must use this method.
    pub fn cell_layout(&self, window: &mut Window) -> Option<TerminalLayout> {
        let (w, h) = super::text_metrics::cell_metrics_at(
            window,
            &self.state.font,
            gpui::px(self.state.font_size),
        )?;
        Some(TerminalLayout {
            cell_width: w * self.state.horizontal_spacing,
            line_height: h * self.state.vertical_spacing,
        })
    }

    /// Replace the primary font. Invalidates the shape cache.
    pub fn set_font(&mut self, font: gpui::Font) {
        self.state.font = font;
        self.line_layouts.clear();
        self.line_layout_key = None;
    }

    /// Push new font settings from the app-level config into this view at
    /// runtime. Invalidates the shape cache. Values are clamped to the sane
    /// range, matching `TerminalConfig::clamp_font_settings`.
    pub fn apply_font_settings(
        &mut self,
        font_size: f32,
        vertical_spacing: f32,
        horizontal_spacing: f32,
    ) {
        self.state.font_size = font_size.clamp(crate::FONT_SIZE_MIN, crate::FONT_SIZE_MAX);
        self.state.vertical_spacing =
            vertical_spacing.clamp(crate::SPACING_MIN, crate::SPACING_MAX);
        self.state.horizontal_spacing =
            horizontal_spacing.clamp(crate::SPACING_MIN, crate::SPACING_MAX);
        self.line_layouts.clear();
        self.line_layout_key = None;
    }

    /// Update background opacity at runtime (0.0–1.0, clamped).
    /// Takes effect on the next frame without invalidating the shape cache.
    pub fn set_background_alpha(&mut self, alpha: f32) {
        self.state.background_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Inactive-pane dim amount (0.0–1.0, clamped). `0.0` paints the
    /// terminal at full color; `> 0` blends every color toward mid-gray
    /// by this amount, alpha preserved (iTerm2-style). Set by the
    /// Workspace from per-pane focus. Does not `cx.notify()` — the
    /// caller (single update site) decides when the view repaints; the
    /// shape cache invalidates automatically since `dim_amount` is part
    /// of the line-layout key.
    pub fn set_dim_amount(&mut self, amount: f32) {
        self.state.dim_amount = amount.clamp(0.0, 1.0);
    }

    /// Current inactive-pane dim amount. The Workspace reads this to
    /// skip redundant updates (notify only on change).
    pub fn dim_amount(&self) -> f32 {
        self.state.dim_amount
    }
}
