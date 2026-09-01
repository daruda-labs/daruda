//! Render-once wrapper over `gpui_component::text::TextView` Markdown.
//!
//! Builds the window-needing `TextView::markdown` inside render so callers can
//! construct Markdown from cx-only snapshot paths. Selection is GPUI keyed
//! state, a sanctioned MVU exception; callers must provide stable ids or live
//! selection resets.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled as _,
    Window, relative,
};
use gpui_component::ActiveTheme as _;
use gpui_component::text::{TextView, TextViewStyle};

use crate::ui::theme;

/// Optional code-block override hook kept domain-free for this UI wrapper.
type CodeBlockRender =
    Arc<dyn Fn(&str, &str, &mut Window, &mut App) -> Option<AnyElement> + Send + Sync>;
type LinkClickHandler = Arc<dyn Fn(&str, &mut Window, &mut App) -> bool>;

#[derive(IntoElement)]
pub struct Markdown {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    text_size: Option<Pixels>,
    full_width: bool,
    surface: Option<Hsla>,
    code_block_render: Option<CodeBlockRender>,
    link_click_handler: Option<LinkClickHandler>,
}

impl Markdown {
    /// Toggle drag-selection + copy. Default `true` (via [`markdown`]).
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// Fill the container width so block markdown wraps; set false for bubbles.
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

    /// The background this markdown is painted on. Its lightness decides
    /// whether the view's fills and structural lines — inline-code chips, the
    /// code-block fill and border, table lines, the `<hr>` rule — lighten or
    /// darken. Unset leaves that to the UI theme's canvas, which is only the
    /// right surface for a caller painting on it: a pane mirroring the terminal
    /// palette must pass its own (DESIGN.md §AgentChatPane).
    pub fn surface(mut self, surface: Hsla) -> Self {
        self.surface = Some(surface);
        self
    }

    /// Override how fenced code blocks render. The closure receives the fence
    /// language (empty string when none) and raw source, and returns
    /// `Some(element)` to replace the default code rendering or `None` to keep
    /// it. Must be `Send + Sync + 'static` — `TextView` threads it through its
    /// async parse. Domain-free: only `&str` in / `AnyElement` out.
    pub fn code_block_render<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &str, &mut Window, &mut App) -> Option<AnyElement> + Send + Sync + 'static,
    {
        self.code_block_render = Some(Arc::new(f));
        self
    }

    /// Override link clicks. Return `true` when the link was handled; `false`
    /// keeps TextView's default platform URL opener.
    pub fn link_click_handler<F>(mut self, f: F) -> Self
    where
        F: Fn(&str, &mut Window, &mut App) -> bool + 'static,
    {
        self.link_click_handler = Some(Arc::new(f));
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
        // Keep the markdown id to key each code block's copy button (below).
        let base_id = self.id.clone();
        let mut view = TextView::markdown(self.id, self.text, window, cx)
            .selectable(self.selectable)
            .text_color(color);
        // Without a width constraint TextView lays out at its intrinsic
        // max-content width, overflowing the container (no wrap; content
        // clipped off-screen). Fill + min_w_0 so it wraps to the container.
        if self.full_width {
            view = view.w_full().min_w_0();
        }
        // A blockquote is de-emphasised against the body, and the view would
        // otherwise reach for the UI theme's muted tone — which says nothing
        // about a caller painting on its own surface (the agent-chat pane
        // mirrors the terminal palette). Derive it from the body colour so it
        // tracks that surface, and any dimming the caller already applied.
        view = view.muted_text_color(color.opacity(theme::MD_VIEW_MUTED_ALPHA));
        if let Some(surface) = self.surface {
            view = view.surface_color(surface);
        }
        // The paragraph body inherits the ambient text size, so set it to the
        // caller's configured size, and rebase the view's vertical metrics on
        // that size. `TextViewStyle`'s defaults are rem-anchored — a 14px
        // heading base and a 1 rem paragraph gap — so at any configured size
        // other than the 13px default they stop agreeing with the body: an
        // `#####` heading renders *smaller* than the prose around it, and the
        // gap between paragraphs stays 16px however large the text gets.
        if let Some(size) = self.text_size {
            view = view.text_size(size).style(TextViewStyle {
                line_height: relative(theme::MD_VIEW_LINE_HEIGHT),
                paragraph_gap: (size * theme::MD_VIEW_PARAGRAPH_GAP).into(),
                heading_base_font_size: size,
                ..TextViewStyle::default()
            });
        }
        // Adapt the domain-free `(lang, source)` hook to TextView's
        // `CodeBlock`-shaped one: extract lang + raw source and forward.
        if let Some(cbr) = self.code_block_render {
            view = view.code_block_render(move |cb, window, cx| {
                let lang = cb.lang();
                let lang = lang.as_deref().unwrap_or("");
                let source = cb.code();
                cbr(lang, source.as_ref(), window, cx)
            });
        }
        if let Some(handler) = self.link_click_handler {
            view = view.link_click_handler(move |url, window, cx| handler(url, window, cx));
        }
        // A hover-revealed copy button on every rendered code block, on for all
        // markdown. Mermaid fences take the separate `code_block_render`
        // replace-path above, which fully replaces the block's rendering (and
        // supplies its own copy button), so they never reach this actions
        // overlay and are naturally excluded. Key each button by the markdown
        // id plus a content hash so its ✓ feedback state stays stable across
        // renders even with multiple blocks of the same language. (Two
        // byte-identical code blocks in one message hash alike and share one
        // ✓ state — benign: they copy the same text.)
        view = view.code_block_actions(move |cb, window, cx| {
            let mut hasher = DefaultHasher::new();
            cb.code().as_ref().hash(&mut hasher);
            let id = ElementId::Name(format!("{base_id}-codecopy-{}", hasher.finish()).into());
            crate::ui::code_copy_button(id, cb.code(), window, cx)
        });
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
        surface: None,
        code_block_render: None,
        link_click_handler: None,
    }
}

#[cfg(test)]
mod tests;
