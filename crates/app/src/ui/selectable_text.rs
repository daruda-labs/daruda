//! Wrapper over `gpui_component::text::TextView::plain` — the raw string
//! rendered verbatim (no Markdown interpretation), drag-selectable + copyable.
//!
//! This is the plain-text counterpart to [`crate::ui::markdown`]: same keyed
//! selection state and copy support, but the source is shown literally so
//! command output / logs / titles with `*`, `_`, `#`, backticks don't get
//! mis-formatted. Mirrors zed's `Markdown::new_text` (parse-links-only) role —
//! a selectable primitive for non-markdown text.
//!
//! Shape: a `RenderOnce` storing only `(id, text)` so callers construct it on a
//! cx-only snapshot render path; `TextView::plain` (which needs `&mut Window`
//! for its keyed state) is built inside `render`.
//!
//! `id` must be stable per logical block across renders, or the keyed state
//! (and the live selection) resets. Callers key by a stable per-block id
//! (e.g. tool-call id + block index).

use gpui::{ElementId, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled as _, Window};
use gpui_component::ActiveTheme as _;
use gpui_component::text::TextView;

#[derive(IntoElement)]
pub struct SelectableText {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    text_size: Option<Pixels>,
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

    /// Set the text size. Unset = inherit the ambient size.
    pub fn text_size(mut self, size: Pixels) -> Self {
        self.text_size = Some(size);
        self
    }
}

impl RenderOnce for SelectableText {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        // Plain runs carry no color of their own, so they inherit the ambient
        // text color; seed it (caller override else theme foreground) or the
        // body renders in gpui's default black, invisible on the dark canvas.
        let color = self.color.unwrap_or_else(|| cx.theme().foreground);
        let mut view = TextView::plain(self.id, self.text, window, cx)
            .selectable(self.selectable)
            .text_color(color);
        // Without a width constraint TextView lays out at its intrinsic
        // max-content width, overflowing the container. Fill + min_w_0 so it
        // wraps to the container.
        if self.full_width {
            view = view.w_full().min_w_0();
        }
        if let Some(size) = self.text_size {
            view = view.text_size(size);
        }
        view
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
        text_size: None,
        full_width: true,
    }
}
