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
type LinkClickHandler = Arc<dyn Fn(&str, &mut Window, &mut App) -> bool>;

#[derive(IntoElement)]
pub struct Markdown {
    id: ElementId,
    text: SharedString,
    selectable: bool,
    color: Option<Hsla>,
    text_size: Option<Pixels>,
    full_width: bool,
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
        code_block_render: None,
        link_click_handler: None,
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers,
        MouseButton, MouseDownEvent, ParentElement as _, Pixels, Render, SharedString, Styled as _,
        TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, deferred, div,
        point, prelude::FluentBuilder as _, px, size,
    };

    use std::cell::Cell;
    use std::rc::Rc;

    use crate::test_support::init_gpui_component;

    struct MarkdownProbe {
        text: SharedString,
    }

    impl Render for MarkdownProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("markdown-probe")
                .debug_selector(|| "markdown-probe".into())
                .w_full()
                .child(super::markdown("probe", self.text.clone()))
        }
    }

    /// Opens a real window pinned to `width` (tall enough that height never
    /// clips) with one markdown body, lets it settle, then reads the painted
    /// height of the `markdown-probe` div back via `debug_bounds` — the
    /// wrap- and stack-driven line count made visible.
    fn markdown_probe_height(
        cx: &mut TestAppContext,
        text: impl Into<SharedString>,
        width: Pixels,
    ) -> Pixels {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(width, px(4000.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let text = text.into();
        let window = cx
            .update(|cx| cx.open_window(opts, |_window, cx| cx.new(|_cx| MarkdownProbe { text })))
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
        vcx.debug_bounds("markdown-probe")
            .expect("markdown probe painted")
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

        let narrow = markdown_probe_height(cx, text.clone(), px(220.));
        let wide = markdown_probe_height(cx, text, px(1400.));

        assert!(
            narrow > wide,
            "heading did not reflow at a narrower width: narrow={narrow:?} wide={wide:?}"
        );
    }

    /// A hard line break (two trailing spaces) must start a new line. The
    /// parser dropped `mdast::Node::Break` on its catch-all arm, which glued
    /// the runs on either side into one — an agent reply that puts a citation
    /// link on the line under its claim rendered as `…제거합니다.lib.rs`.
    #[gpui::test]
    fn a_hard_line_break_starts_a_new_line(cx: &mut TestAppContext) {
        init_gpui_component(cx);

        let broken = markdown_probe_height(cx, "AAAA  \nBBBB", px(600.));
        let glued = markdown_probe_height(cx, "AAAABBBB", px(600.));

        assert!(
            broken > glued,
            "hard break did not start a new line: with_break={broken:?} glued={glued:?}"
        );
    }

    /// A list item's second and later paragraphs must stack under the first,
    /// not sit beside it. They were appended to the lead paragraph's `h_flex`
    /// (a flex **row**), so they added no height at all and shared the item's
    /// width — every numbered finding in an agent's review reply collapsed
    /// into side-by-side columns.
    #[gpui::test]
    fn a_list_item_stacks_its_continuation_paragraphs(cx: &mut TestAppContext) {
        init_gpui_component(cx);

        let three = markdown_probe_height(cx, "1. AAAA\n\n   BBBB\n\n   CCCC", px(600.));
        let one = markdown_probe_height(cx, "1. AAAA", px(600.));

        assert!(
            three > one,
            "continuation paragraphs added no height: three={three:?} one={one:?}"
        );
    }

    /// The same defect seen through width rather than height. Sharing a row
    /// makes the continuation's width — and so its wrap — depend on how long
    /// the lead line is; stacked, the lead's length is irrelevant. Both leads
    /// fit on one line at this width, so the two must measure identically.
    #[gpui::test]
    fn a_list_item_continuation_keeps_the_full_width(cx: &mut TestAppContext) {
        init_gpui_component(cx);
        let long = "This continuation paragraph is long enough that it must wrap several times when it is laid out inside a column narrower than the pane it belongs to.";

        let short_lead = markdown_probe_height(cx, format!("1. Lead\n\n   {long}"), px(600.));
        let long_lead = markdown_probe_height(
            cx,
            format!("1. A considerably longer lead line that fills the row\n\n   {long}"),
            px(600.),
        );

        assert_eq!(
            short_lead, long_lead,
            "the continuation's wrap depends on the lead line's length — they share a row"
        );
    }

    /// A selectable markdown block, optionally with a panel painted over it
    /// that declares itself modal to the mouse the way `Popover` does.
    struct SelectionProbe {
        occluded: bool,
    }

    const PROBE_TEXT: &str = "Selectable prose the panel is sitting on top of.";
    /// A link at the very start of the line, so one press position lands on
    /// both it and the panel above.
    const PROBE_LINK: &str = "[link](https://example.invalid/opened) trailing prose.";

    impl Render for SelectionProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .child(super::markdown("probe", SharedString::from(PROBE_TEXT)))
                .when(self.occluded, |this| {
                    // `deferred` is what puts the panel's hitbox in front of the
                    // block's, mirroring how `Popover` paints its content.
                    this.child(deferred(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .occlude()
                            .child("panel"),
                    ))
                })
        }
    }

    /// Press (no release) over the prose and report whether a text block took
    /// the grab. Mouse-down only on purpose: a click that never drags selects
    /// nothing, and the release deregisters the empty selection — so the press
    /// is the only moment the grab is observable.
    fn press_grabs_the_prose(cx: &mut TestAppContext, occluded: bool) -> bool {
        init_gpui_component(cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(400.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| {
                cx.open_window(opts, |_window, cx| {
                    cx.new(|_cx| SelectionProbe { occluded })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();

        vcx.simulate_event(MouseDownEvent {
            position: point(px(40.), px(10.)),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        vcx.run_until_parked();

        vcx.update(|_window, cx| crate::ui::active_text_selection(cx).is_some())
    }

    /// The control for the pair below: with nothing over it, a press on
    /// selectable prose must still start a drag-selection. Without this, a fix
    /// that simply stopped selecting altogether would satisfy the other test.
    #[gpui::test]
    fn a_press_on_selectable_prose_grabs_it(cx: &mut TestAppContext) {
        assert!(
            press_grabs_the_prose(cx, false),
            "a plain press no longer starts a selection at all"
        );
    }

    /// Pressing inside an `occlude()`d panel must not grab the text underneath.
    /// `occlude()` only suppresses `Hitbox::is_hovered`, so a listener that never
    /// asks a hitbox is immune to it.
    #[gpui::test]
    fn a_press_inside_an_occluding_panel_does_not_grab_the_prose_under_it(cx: &mut TestAppContext) {
        assert!(
            !press_grabs_the_prose(cx, true),
            "the press reached the text under the occluding panel and grabbed it"
        );
    }

    /// A markdown block whose first characters are a link, optionally covered by
    /// the same `occlude()`d panel. Owns the "was it opened" flag rather than
    /// sharing a `static`: `#[gpui::test]` expands to `#[test]`, and cargo runs
    /// those across threads, so a shared flag would let the pair clobber
    /// each other.
    struct LinkProbe {
        occluded: bool,
        opened: Rc<Cell<bool>>,
    }

    /// Height the link block is confined to, so the space under it is genuinely
    /// outside the text element and a press there starts no selection.
    const LINK_BLOCK_H: f32 = 30.;

    impl Render for LinkProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div().h(px(LINK_BLOCK_H)).child(
                        super::markdown("link-probe", SharedString::from(PROBE_LINK))
                            .link_click_handler({
                                let opened = self.opened.clone();
                                move |_url, _window, _cx| {
                                    opened.set(true);
                                    true
                                }
                            }),
                    ),
                )
                .child(div().flex_1())
                .when(self.occluded, |this| {
                    this.child(deferred(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .occlude()
                            .child("panel"),
                    ))
                })
        }
    }

    /// Release over the link and report whether it was opened. The link fires on
    /// mouse-*up*, so this is a full click.
    fn click_opens_the_link(cx: &mut TestAppContext, occluded: bool) -> bool {
        init_gpui_component(cx);
        let opened = Rc::new(Cell::new(false));
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(400.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| {
                let opened = opened.clone();
                cx.open_window(opts, |_window, cx| {
                    cx.new(|_cx| LinkProbe { occluded, opened })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();

        vcx.simulate_click(point(px(8.), px(8.)), Modifiers::default());
        vcx.run_until_parked();

        opened.get()
    }

    /// Press somewhere that is not the link, drag onto it, release. Reports
    /// whether the link opened.
    fn drag_onto_link_opens_it(cx: &mut TestAppContext, from: gpui::Point<gpui::Pixels>) -> bool {
        init_gpui_component(cx);
        let opened = Rc::new(Cell::new(false));
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(400.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| {
                let opened = opened.clone();
                cx.open_window(opts, |_window, cx| {
                    cx.new(|_cx| LinkProbe {
                        occluded: false,
                        opened,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();

        let onto = point(px(8.), px(8.));
        vcx.simulate_event(MouseDownEvent {
            position: from,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        vcx.simulate_event(gpui::MouseMoveEvent {
            position: onto,
            modifiers: Modifiers::default(),
            pressed_button: Some(MouseButton::Left),
        });
        vcx.simulate_event(gpui::MouseUpEvent {
            position: onto,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
        vcx.run_until_parked();

        opened.get()
    }

    /// A release is not a click: navigation needs a press that agreed with it.
    /// A drag begun off the block — the pane behind, or a popover's padding —
    /// must not open the link it happens to end on.
    #[gpui::test]
    fn a_drag_that_merely_ends_on_a_link_does_not_open_it(cx: &mut TestAppContext) {
        // Below the block's own height, so the press misses the text element.
        assert!(
            !drag_onto_link_opens_it(cx, point(px(300.), px(LINK_BLOCK_H + 100.))),
            "a drag that started off the block still opened the link it ended on"
        );
    }

    /// Control for the pair below: an uncovered link must still open.
    #[gpui::test]
    fn a_click_on_a_link_opens_it(cx: &mut TestAppContext) {
        assert!(
            click_opens_the_link(cx, false),
            "a plain click no longer opens a link at all"
        );
    }

    /// Same gate as the selection pair, worse consequence: a click inside an
    /// options popover sitting over a link handed that URL to the browser.
    #[gpui::test]
    fn a_click_inside_an_occluding_panel_does_not_open_the_link_under_it(cx: &mut TestAppContext) {
        assert!(
            !click_opens_the_link(cx, true),
            "the click reached the link under the occluding panel and opened it"
        );
    }
}
