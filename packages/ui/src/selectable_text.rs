//! Wrapper over `gpui_base::SelectableText` — the raw string
//! rendered verbatim (no Markdown interpretation), drag-selectable + copyable.
//!
//! This is the plain-text counterpart to [`crate::markdown()`]: same keyed
//! selection state and copy support, but the source is shown literally so
//! command output / logs / titles with `*`, `_`, `#`, backticks don't get
//! mis-formatted. Mirrors zed's `Markdown::new_text` (parse-links-only) role —
//! a selectable primitive for non-markdown text.
//!
//! Shape: a `RenderOnce` storing only `(id, text)` so callers construct it on a
//! cx-only snapshot render path; the selectable element is built inside
//! `render`.
//!
//! `id` must be stable per logical block across renders, or the keyed state
//! (and the live selection) resets. Callers key by a stable per-block id
//! (e.g. tool-call id + block index).

use gpui::{
    ElementId, Hsla, IntoElement, ParentElement as _, Pixels, RenderOnce, SharedString,
    Styled as _, Window, div, relative,
};
use gpui_component::ActiveTheme as _;

#[derive(IntoElement)]
pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    font_family: Option<SharedString>,
    text_size: Option<Pixels>,
    line_height: Option<f32>,
    full_width: bool,
}

impl SelectableText {
    /// Toggle drag-selection + copy. Default `true` (via [`selectable_text`]).
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Fill the container width (so text wraps to it) vs. size to content.
    /// Default `true`.
    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    /// Override the base text color. Defaults to the theme foreground.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn font_family(mut self, family: impl Into<SharedString>) -> Self {
        self.font_family = Some(family.into());
        self
    }

    /// Set the text size. Unset = inherit the ambient size.
    pub fn text_size(mut self, size: Pixels) -> Self {
        self.text_size = Some(size);
        self
    }

    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height);
        self
    }
}

impl RenderOnce for SelectableText {
    fn render(self, _window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        // Plain runs carry no color of their own, so they inherit the ambient
        // text color; seed it (caller override else theme foreground) or the
        // body renders in gpui's default black, invisible on the dark canvas.
        let color = self.color.unwrap_or_else(|| cx.theme().foreground);
        let mut frame = div().text_color(color);
        if let Some(family) = self.font_family {
            frame = frame.font_family(family);
        }
        // Fill the container so the text wraps to it instead of laying out at
        // its intrinsic max-content width.
        if self.full_width {
            frame = frame.w_full();
        }
        if let Some(size) = self.text_size {
            frame = frame.text_size(size);
        }
        if let Some(line_height) = self.line_height {
            frame = frame.line_height(relative(line_height));
        }
        if self.selectable {
            frame.child(gpui_base::SelectableText::new(self.id, self.text))
        } else {
            frame.child(self.text)
        }
    }
}

/// Verbatim, drag-selectable + copyable plain text. Selectable by default.
/// `id` must be stable per block across renders (keyed selection state).
pub fn selectable_text(id: impl Into<ElementId>, text: impl Into<SharedString>) -> SelectableText {
    SelectableText {
        id: id.into(),
        text: text.into(),
        selectable: true,
        color: None,
        font_family: None,
        text_size: None,
        line_height: None,
        full_width: true,
    }
}
