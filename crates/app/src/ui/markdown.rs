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
    Window,
};
use gpui_component::ActiveTheme as _;
use gpui_component::text::TextView;

/// Optional code-block override hook kept domain-free for this UI wrapper.
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

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _,
        Pixels, Render, SharedString, Styled as _, TestAppContext, VisualTestContext, Window,
        WindowBounds, WindowOptions, div, point, px, size,
    };

    use crate::test_support::init_gpui_component;

    struct HeadingProbe {
        text: SharedString,
    }

    impl Render for HeadingProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("heading-probe")
                .debug_selector(|| "heading-probe".into())
                .w_full()
                .child(super::markdown("probe-heading", self.text.clone()))
        }
    }

    /// Opens a real window pinned to `width` (tall enough that height never
    /// clips) with a single heading, lets it settle, then reads the painted
    /// height of the `heading-probe` div back via `debug_bounds` — the
    /// wrap-driven line count made visible.
    fn heading_probe_height(cx: &mut TestAppContext, text: SharedString, width: Pixels) -> Pixels {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(width, px(4000.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| cx.open_window(opts, |_window, cx| cx.new(|_cx| HeadingProbe { text })))
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        // The window's construction-time paint (`open_window`'s "at least one
        // draw") can land before layout has fully settled; force + wait out a
        // second frame so `debug_bounds` reads the settled wrap, not a
        // first-frame transient (same rationale as the diff-editor probe's
        // frame-1-vs-frame-2 check in `workspace/tests/agent_diff_layout.rs`).
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        vcx.debug_bounds("heading-probe")
            .expect("heading probe painted")
            .size
            .height
    }

    /// A long `# ` heading laid out at a narrow window width must wrap to
    /// more lines (grow taller) than the same heading laid out at a wide
    /// window width — proof it reflows with the container instead of holding
    /// its intrinsic max-content width and overflowing unbroken (the
    /// agent-chat heading wrap regression:
    /// `gpui_component::text::node::Node::Heading` wraps its text in a bare
    /// `h_flex()`, whose flex-row default `min-width: auto` stops the child
    /// from shrinking below its unwrapped width).
    #[gpui::test]
    fn heading_reflows_narrower_than_wide(cx: &mut TestAppContext) {
        init_gpui_component(cx);
        let text: SharedString =
            "# This heading has enough words that it must wrap across more than one line once the pane narrows"
                .into();

        let narrow = heading_probe_height(cx, text.clone(), px(220.));
        let wide = heading_probe_height(cx, text, px(1400.));

        assert!(
            narrow > wide,
            "heading did not reflow at a narrower width: narrow={narrow:?} wide={wide:?}"
        );
    }
}
