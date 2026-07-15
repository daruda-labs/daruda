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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, Pixels, RenderOnce, SharedString, Styled as _,
    Window,
};
use gpui_component::ActiveTheme as _;
use gpui_component::text::TextView;

/// Host hook to replace a code block's rendering with a custom element,
/// keyed off the fence's `(lang, source)`. Returns `Some(el)` to override,
/// `None` to keep the default code rendering. `Send + Sync + 'static` because
/// `TextView` threads it through its async parse. Domain-free: the closure
/// takes/returns only primitives + `AnyElement`, so this module stays free of
/// any workspace / raster types.
type CodeBlockRender =
    Arc<dyn Fn(&str, &str, &mut Window, &mut App) -> Option<AnyElement> + Send + Sync>;

#[derive(IntoElement)]
pub struct Markdown {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    text_size: Option<Pixels>,
    full_width: bool,
    code_block_render: Option<CodeBlockRender>,
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
        // The paragraph body inherits the ambient text size, so set it to the
        // caller's configured size (headings keep their own relative scale).
        if let Some(size) = self.text_size {
            view = view.text_size(size);
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
        code_block_render: None,
    }
}
