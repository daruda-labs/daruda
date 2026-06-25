//! Wrapper over `gpui_component::text::TextView` — rendered, drag-selectable,
//! copyable Markdown (headings, bold, code blocks with syntax highlight, GFM).
//!
//! Shape: a `RenderOnce` that stores only `(id, text)`, so the caller
//! constructs it cx-only in a snapshot render path (no `&mut Window` needed at
//! the call site). `TextView::markdown` — which *does* need `&mut Window` to
//! register its keyed selection state — is built inside `render`, where the
//! window is in scope. This is how a window-needing widget rides a cx-only
//! render path without threading `&mut Window` through it.
//!
//! Selection state is owned by GPUI, keyed by `id`
//! (`window.use_keyed_state("{id}/state")`), so there is no per-message entity
//! to manage. That mirrors the diff editor entities and the scroll handle: a
//! sanctioned "GPUI-owned view state" exception to the MVU single-source rule.
//!
//! `id` must be stable per logical block across renders, or the keyed state
//! (and the live selection) resets. Callers key by the conversation item index
//! — valid only while the item list is append-only (see the agent-chat fold
//! INVARIANT).

use gpui::{
    App, ElementId, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled as _, Window,
};
use gpui_component::ActiveTheme as _;
use gpui_component::text::TextView;

#[derive(IntoElement)]
pub struct Markdown {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    text_size: Option<Pixels>,
    full_width: bool,
}

impl Markdown {
    /// Toggle drag-selection + copy. Default `true` (via [`markdown`]).
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Fill the container width (so text wraps to it) vs. size to content.
    /// Default `true` — needed for block markdown in a column, where without
    /// it `TextView` lays out at its intrinsic max-content width, overflowing
    /// the pane (no wrap, content clipped off-screen). Set `false` for a
    /// shrink-to-fit context like a chat bubble.
    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    /// Override the base text color (plain runs). Code blocks keep their own
    /// syntax colors regardless. Defaults to the theme foreground.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Set the body font size so the markdown follows the host's configured
    /// text size instead of the `TextView` default. Headings still scale
    /// relative to their own base. Unset = inherit the ambient size.
    pub fn text_size(mut self, size: Pixels) -> Self {
        self.text_size = Some(size);
        self
    }
}

impl RenderOnce for Markdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // TextView's plain (non-syntax) runs carry no color of their own, so
        // they inherit the ambient text color. Seed it (caller override else
        // theme foreground) — otherwise the body renders in gpui's default
        // black, invisible on the dark canvas. Code blocks keep syntax colors.
        let color = self.color.unwrap_or_else(|| cx.theme().foreground);
        let mut view = TextView::markdown(self.id, self.text, window, cx)
            .selectable(self.selectable)
            .text_color(color);
        // Without a width constraint TextView lays out at its intrinsic
        // max-content width, overflowing the container (no wrap; content
        // clipped off-screen). Fill + min_w_0 so it wraps to the container.
        if self.full_width {
            view = view.w_full().min_w_0();
        }
        // The paragraph body inherits the ambient text size, so set it to the
        // caller's configured size (headings keep their own relative scale).
        if let Some(size) = self.text_size {
            view = view.text_size(size);
        }
        view
    }
}

/// Rendered, drag-selectable + copyable Markdown. Selectable by default.
/// `id` must be stable per block across renders (keyed selection state).
pub fn markdown(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Markdown {
    Markdown {
        id: id.into(),
        text: text.into(),
        selectable: true,
        color: None,
        text_size: None,
        full_width: true,
    }
}
