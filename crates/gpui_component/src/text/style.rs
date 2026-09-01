use std::sync::Arc;

use gpui::{AbsoluteLength, DefiniteLength, Pixels, StyleRefinement, px, relative, rems};

use crate::highlighter::HighlightTheme;

/// TextViewStyle used to customize the style for [`TextView`].
#[derive(Clone)]
pub struct TextViewStyle {
    /// Gap of each paragraphs, default is 1 rem.
    pub paragraph_gap: AbsoluteLength,
    /// Line height of every rendered line — prose, code blocks, table cells and
    /// the bullet / number a list item is prefixed with. A fraction resolves
    /// against the ambient font size, so the vertical rhythm follows the text
    /// size the host sets; default is 1.6.
    ///
    /// daruda patch: one value for the whole view. Upstream left every line to
    /// gpui's default `phi()` except paragraphs, which carried a hardcoded
    /// `rems(1.3)` — an absolute 20.8 px that ignored the font size. A bullet's
    /// taller `phi()` line box then set the row height of every *single-line*
    /// list item while a wrapped item was driven by the paragraph, so one-line
    /// and two-line items sat at different pitches.
    pub line_height: DefiniteLength,
    /// Base font size for headings, default is 14px.
    pub heading_base_font_size: Pixels,
    /// Function to calculate heading font size based on heading level (1-6).
    ///
    /// The first parameter is the heading level (1-6), the second parameter is the base font size.
    /// The second parameter is the base font size.
    pub heading_font_size: Option<Arc<dyn Fn(u8, Pixels) -> Pixels + Send + Sync + 'static>>,
    /// Highlight theme for code blocks. Default: [`HighlightTheme::default_light()`]
    pub highlight_theme: Arc<HighlightTheme>,
    /// The style refinement for code blocks.
    pub code_block: StyleRefinement,
    pub is_dark: bool,
}

impl PartialEq for TextViewStyle {
    fn eq(&self, other: &Self) -> bool {
        self.paragraph_gap == other.paragraph_gap
            && self.line_height == other.line_height
            && self.heading_base_font_size == other.heading_base_font_size
            && self.highlight_theme == other.highlight_theme
    }
}

impl Default for TextViewStyle {
    fn default() -> Self {
        Self {
            paragraph_gap: rems(1.).into(),
            line_height: relative(1.6),
            heading_base_font_size: px(14.),
            heading_font_size: None,
            highlight_theme: HighlightTheme::default_light().clone(),
            code_block: StyleRefinement::default(),
            is_dark: false,
        }
    }
}

impl TextViewStyle {
    /// Set paragraph gap, default is 1 rem.
    pub fn paragraph_gap(mut self, gap: impl Into<AbsoluteLength>) -> Self {
        self.paragraph_gap = gap.into();
        self
    }

    /// Set the line height every rendered line shares, default is 1.6 of the
    /// ambient font size. Pass a fraction (`relative(1.6)`) to keep it
    /// proportional; an absolute length pins it regardless of text size.
    pub fn line_height(mut self, line_height: impl Into<DefiniteLength>) -> Self {
        self.line_height = line_height.into();
        self
    }

    pub fn heading_font_size<F>(mut self, f: F) -> Self
    where
        F: Fn(u8, Pixels) -> Pixels + Send + Sync + 'static,
    {
        self.heading_font_size = Some(Arc::new(f));
        self
    }

    /// Set style for code blocks.
    pub fn code_block(mut self, style: StyleRefinement) -> Self {
        self.code_block = style;
        self
    }
}
