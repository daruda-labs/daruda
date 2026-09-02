//! Tests for the markdown wrapper: link/selection gating and the rendered
//! vertical metrics (line pitch, list rows, loose-list spacing, heading scale).

use gpui::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Render, SharedString, Styled as _, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, deferred, div, point,
    prelude::FluentBuilder as _, px, size,
};

use std::cell::Cell;
use std::rc::Rc;

use crate::test_support::init_gpui_component;

struct MarkdownProbe {
    text: SharedString,
    /// Body text size, or `None` to inherit the ambient one.
    size: Option<Pixels>,
}

impl Render for MarkdownProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("markdown-probe")
            .debug_selector(|| "markdown-probe".into())
            .w_full()
            .child(
                super::markdown("probe", self.text.clone())
                    .when_some(self.size, |md, size| md.text_size(size)),
            )
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
    probe_height(cx, text, width, None)
}

/// [`markdown_probe_height`] at the body text size a host would set, so the
/// size-derived metrics (line pitch, heading base, paragraph gap) are the ones
/// under test rather than the ambient defaults.
fn sized_probe_height(
    cx: &mut TestAppContext,
    text: impl Into<SharedString>,
    width: Pixels,
    size: Pixels,
) -> Pixels {
    probe_height(cx, text, width, Some(size))
}

fn probe_height(
    cx: &mut TestAppContext,
    text: impl Into<SharedString>,
    width: Pixels,
    text_size: Option<Pixels>,
) -> Pixels {
    let bounds = Bounds::new(point(px(0.), px(0.)), size(width, px(4000.)));
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let text = text.into();
    let window = cx
        .update(|cx| {
            cx.open_window(opts, |_window, cx| {
                cx.new(|_cx| MarkdownProbe {
                    text,
                    size: text_size,
                })
            })
        })
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

/// The body size the reported defects were visible at. Any size works for the
/// properties below; 13px happened to sit within a quarter pixel of the old
/// absolute `rems(1.3)` line, which is why they went unnoticed at the default.
const BODY: Pixels = px(15.);
/// Wide enough that a short item never wraps, narrow enough that `LONG` does.
const PROBE_W: Pixels = px(400.);
const LONG: &str = "wwww wwww wwww wwww wwww wwww wwww wwww wwww wwww wwww wwww wwww wwww";

/// A single-line list item must be exactly as tall as a single-line
/// paragraph. The bullet / number is a bare string child of the item's
/// `h_flex`, so it kept gpui's default `phi()` (1.618 em) line box while the
/// paragraph beside it was pinned to an absolute `rems(1.3)`; at any body size
/// above ~12.8px the taller bullet then set the row height — but only for
/// items short enough that the paragraph didn't already exceed it. One-line
/// items sat 3.5px further apart than the lines inside a wrapped one.
#[gpui::test]
fn a_single_line_list_item_is_one_line_tall(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let paragraph = sized_probe_height(cx, "AAAA", PROBE_W, BODY);
    let bullet = sized_probe_height(cx, "- AAAA", PROBE_W, BODY);
    let numbered = sized_probe_height(cx, "1. AAAA", PROBE_W, BODY);

    assert_eq!(
        bullet, paragraph,
        "a one-line bullet item is not one line tall: item={bullet:?} line={paragraph:?}"
    );
    assert_eq!(
        numbered, paragraph,
        "a one-line numbered item is not one line tall: item={numbered:?} line={paragraph:?}"
    );
}

/// The same list must not change pitch between its own rows. A task item
/// renders a rem-sized checkbox instead of a bullet, so it never inherited the
/// bullet's oversized line box — a list mixing the two had uneven rows.
#[gpui::test]
fn a_bullet_item_and_a_task_item_share_a_row_height(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let bullet = sized_probe_height(cx, "- AAAA", PROBE_W, BODY);
    let task = sized_probe_height(cx, "- [ ] AAAA", PROBE_W, BODY);

    assert_eq!(
        bullet, task,
        "bullet and task rows disagree: bullet={bullet:?} task={task:?}"
    );
}

/// Items separated by blank lines are a *loose* list and must be spaced like
/// paragraphs. The spacer was an empty `div()` in a gap-less flex column, so it
/// added no height at all and a loose list rendered pixel-identical to a tight
/// one.
#[gpui::test]
fn a_loose_list_spaces_its_items(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let tight = sized_probe_height(cx, "- AAAA\n- BBBB\n- CCCC", PROBE_W, BODY);
    let loose = sized_probe_height(cx, "- AAAA\n\n- BBBB\n\n- CCCC", PROBE_W, BODY);

    assert!(
        loose > tight,
        "a loose list is spaced like a tight one: loose={loose:?} tight={tight:?}"
    );
}

/// CommonMark calls a list loose two ways, and mdast reports them on different
/// nodes: an item holding two blocks with a blank line between them sets that
/// *item*'s spread, not the list's. Reading only the list's left this shape
/// stacked flush while the blank-line-between-items shape spaced correctly.
#[gpui::test]
fn an_item_holding_two_blocks_makes_its_list_loose(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    // Item 0 is identical in all three; only what separates the items differs.
    let tight = sized_probe_height(cx, "- AAAA\n\n  cont\n- BBBB", PROBE_W, BODY);
    let blank = sized_probe_height(cx, "- AAAA\n\n  cont\n\n- BBBB", PROBE_W, BODY);

    assert_eq!(
        tight, blank,
        "a multi-block item is as loose as a blank line between items: \
         multi-block={tight:?} blank-line={blank:?}"
    );
}

/// Every vertical metric follows the configured body size. Each was anchored to
/// the 16px rem instead — an absolute 20.8px line and a 1rem paragraph gap — so
/// raising `font.agent_chat_size` grew the glyphs while the leading around them
/// stayed put, crowding the prose.
#[gpui::test]
fn the_vertical_metrics_follow_the_text_size(cx: &mut TestAppContext) {
    init_gpui_component(cx);
    let (small, large) = (px(13.), px(18.));

    let line_small = sized_probe_height(cx, "AAAA", PROBE_W, small);
    let line_large = sized_probe_height(cx, "AAAA", PROBE_W, large);
    assert!(
        line_large > line_small,
        "line pitch ignored the text size: small={line_small:?} large={line_large:?}"
    );

    let gap = |cx: &mut TestAppContext, size, line| {
        sized_probe_height(cx, "AAAA\n\nBBBB", PROBE_W, size) - line - line
    };
    let (gap_small, gap_large) = (gap(cx, small, line_small), gap(cx, large, line_large));
    assert!(
        gap_large > gap_small,
        "paragraph gap ignored the text size: small={gap_small:?} large={gap_large:?}"
    );
}

/// Headings scale from the body size, not from a fixed 14px base. With the
/// base pinned, `#####` and `######` rendered *smaller* than a 15px body and
/// every heading stopped growing with the configured size.
#[gpui::test]
fn headings_scale_from_the_body_text_size(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let small = sized_probe_height(cx, "# AAAA", PROBE_W, px(13.));
    let large = sized_probe_height(cx, "# AAAA", PROBE_W, px(18.));
    assert!(
        large > small,
        "heading ignored the body text size: small={small:?} large={large:?}"
    );

    let body = sized_probe_height(cx, "AAAA", PROBE_W, BODY);
    let h5 = sized_probe_height(cx, "##### AAAA", PROBE_W, BODY);
    assert!(
        h5 >= body,
        "an h5 renders smaller than the body text: h5={h5:?} body={body:?}"
    );
}

/// A code block's rows sit on the same pitch as prose. Code and table cells
/// were left on gpui's `phi()` default while prose carried its own absolute
/// line height, so one document had two vertical rhythms.
#[gpui::test]
fn a_code_block_shares_the_prose_line_pitch(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let prose = sized_probe_height(cx, "AAAA", PROBE_W, BODY);
    let one = sized_probe_height(cx, "```\nA\n```", PROBE_W, BODY);
    let three = sized_probe_height(cx, "```\nA\nB\nC\n```", PROBE_W, BODY);

    assert_eq!(
        (three - one) / 2.0,
        prose,
        "code rows sit on a different pitch than prose: code={:?} prose={prose:?}",
        (three - one) / 2.0
    );
}

/// The wrapped-vs-single-line pitch, stated as the defect was reported: three
/// items that each wrap to two lines must be exactly twice as tall as three
/// that don't.
#[gpui::test]
fn a_wrapped_list_keeps_the_pitch_of_an_unwrapped_one(cx: &mut TestAppContext) {
    init_gpui_component(cx);

    let short = sized_probe_height(cx, "- AAAA\n- BBBB\n- CCCC", PROBE_W, BODY);
    let wrapped = sized_probe_height(cx, format!("- {LONG}\n- {LONG}\n- {LONG}"), PROBE_W, BODY);

    assert_eq!(
        wrapped,
        short * 2.0,
        "wrapped items sit at a different pitch: wrapped={wrapped:?} short={short:?}"
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
            .child(div().h(px(LINK_BLOCK_H)).child(
                super::markdown("link-probe", SharedString::from(PROBE_LINK)).link_click_handler({
                    let opened = self.opened.clone();
                    move |_url, _window, _cx| {
                        opened.set(true);
                        true
                    }
                }),
            ))
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
