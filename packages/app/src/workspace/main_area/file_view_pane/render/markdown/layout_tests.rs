use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::selection::{
    queue_block_selection, take_pending_block_selection, take_pending_block_selection_for_drag,
};
use super::{MdColors, MdRenderAssets, OpenUrl, render_md_body_layout};
use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::images::MdImages;
use crate::workspace::main_area::file_view_pane::markdown_viewer::{
    MdBlock, parse_markdown, resolve_all,
};
use gpui::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Render, StatefulInteractiveElement as _, Styled as _,
    TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    FileViewer,
    AgentChat,
}

impl Engine {
    const ALL: [Self; 2] = [Self::FileViewer, Self::AgentChat];

    fn label(self) -> &'static str {
        match self {
            Self::FileViewer => "file viewer",
            Self::AgentChat => "agent chat",
        }
    }
}

/// Parse, then run the resolve passes with loaders that never produce pixels.
/// The probes assert on layout, not on bitmaps, but the slots still have to be
/// stamped: the renderer resolves each image through the table those passes
/// number, and a span left unresolved would index past its end.
fn parse_and_resolve(md: &str) -> (Vec<MdBlock>, MdImages) {
    let mut blocks = parse_markdown(md, "default", false);
    let rasters = resolve_all(&mut blocks, &mut |_| None, &mut |_| None);
    (blocks, MdImages::from_rasters(rasters))
}

struct Probe {
    md: String,
    engine: Engine,
    first_inline_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
}

struct LinkProbe {
    md: String,
    opened: Arc<Mutex<Vec<String>>>,
    block_mouse_downs: Arc<Mutex<usize>>,
}

impl Render for LinkProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = MdColors::for_pane(cx);
        let (blocks, images) = parse_and_resolve(&self.md);
        let opened = self.opened.clone();
        let on_open_url: OpenUrl = Rc::new(move |url, _, _| {
            opened.lock().unwrap().push(url.to_owned());
        });
        let block_mouse_downs = self.block_mouse_downs.clone();
        let body = render_md_body_layout(
            &blocks,
            None,
            MdRenderAssets {
                t: &colors,
                images: &images,
                font_size: theme::FILE_VIEWER_FONT_SIZE,
                line_height: theme::FILE_VIEWER_LINE_H_RATIO,
            },
            on_open_url,
            move |block, block_idx| {
                let committed = block_mouse_downs.clone();
                let dragged = block_mouse_downs.clone();
                block
                    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                        queue_block_selection(block_idx, event.modifiers.shift, cx);
                    })
                    .on_mouse_move(move |event, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left)
                            && take_pending_block_selection_for_drag(block_idx, cx).is_some()
                        {
                            *dragged.lock().unwrap() += 1;
                        }
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                        if take_pending_block_selection(cx).is_some() {
                            *committed.lock().unwrap() += 1;
                        }
                    })
            },
        )
        .id("md-link-probe")
        .debug_selector(|| "md-link-probe".into());

        div()
            .size_full()
            .font_family("Menlo")
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .child(body)
    }
}

impl Render for Probe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.engine {
            Engine::FileViewer => {
                let colors = MdColors::for_pane(cx);
                let (blocks, images) = parse_and_resolve(&self.md);
                let body = render_md_body_layout(
                    &blocks,
                    None,
                    MdRenderAssets {
                        t: &colors,
                        images: &images,
                        font_size: theme::FILE_VIEWER_FONT_SIZE,
                        line_height: theme::FILE_VIEWER_LINE_H_RATIO,
                    },
                    Rc::new(|_, _, _| {}),
                    |block, _| block,
                )
                .id("md-probe")
                .debug_selector(|| "md-probe".into());
                // Faithful to the real pane, outermost first: the walker's
                // `flex_1().min_h(0).overflow_hidden()` slot in a column, the pane
                // root (`relative().size_full()` + the configured font), the
                // toolbar-offset absolute frame, then `body.rs`'s scroll container.
                div().flex().flex_col().size_full().child(
                    div().flex_1().min_h(px(0.)).overflow_hidden().child(
                        div().relative().size_full().font_family("Menlo").child(
                            div()
                                .absolute()
                                .top(px(theme::FILE_VIEWER_HEADER_H))
                                .left_0()
                                .right_0()
                                .bottom_0()
                                .id("probe-scroll")
                                .overflow_y_scroll()
                                .child(body),
                        ),
                    ),
                )
            }
            Engine::AgentChat => {
                let first_inline_bounds = self.first_inline_bounds.clone();
                div().size_full().font_family("Menlo").child(
                    div()
                        .id("md-probe")
                        .debug_selector(|| "md-probe".into())
                        .w_full()
                        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                        .child(
                            crate::ui::markdown::markdown("conformance", self.md.clone())
                                .debug_inline_bounds(move |bounds| {
                                    let mut first = first_inline_bounds.lock().unwrap();
                                    if first.is_none() {
                                        *first = Some(bounds);
                                    }
                                }),
                        ),
                )
            }
        }
    }
}

struct ProbeMeasurement {
    selected: Option<gpui::Size<Pixels>>,
    first_inline: Option<gpui::Size<Pixels>>,
}

/// Painted sizes from a `width`-wide probe: the requested div and the
/// agent renderer's first actual `Inline` prose run, when it has one.
fn measure_engine(
    cx: &mut TestAppContext,
    engine: Engine,
    md: &str,
    width: Pixels,
    selector: &'static str,
) -> ProbeMeasurement {
    let bounds = Bounds::new(point(px(0.), px(0.)), size(width, px(4000.)));
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let md = md.to_string();
    let first_inline_bounds = Arc::new(Mutex::new(None));
    let window = cx
        .update(|cx| {
            let first_inline_bounds = first_inline_bounds.clone();
            cx.open_window(opts, |_w, cx| {
                cx.new(|_cx| Probe {
                    md,
                    engine,
                    first_inline_bounds,
                })
            })
        })
        .expect("window opens");
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    *first_inline_bounds.lock().unwrap() = None;
    vcx.update(|w, _| w.refresh());
    vcx.run_until_parked();
    let selected = vcx.debug_bounds(selector).map(|bounds| bounds.size);
    let first_inline = first_inline_bounds
        .lock()
        .unwrap()
        .as_ref()
        .map(|bounds| bounds.size);
    ProbeMeasurement {
        selected,
        first_inline,
    }
}

fn bounds_for_engine(
    cx: &mut TestAppContext,
    engine: Engine,
    md: &str,
    width: Pixels,
    selector: &'static str,
) -> Option<gpui::Size<Pixels>> {
    measure_engine(cx, engine, md, width, selector).selected
}

fn bounds_of(
    cx: &mut TestAppContext,
    md: &str,
    width: Pixels,
    selector: &'static str,
) -> gpui::Size<Pixels> {
    bounds_for_engine(cx, Engine::FileViewer, md, width, selector).expect("painted")
}

fn height(cx: &mut TestAppContext, md: &str, width: Pixels) -> Pixels {
    bounds_of(cx, md, width, "md-probe").height
}

#[gpui::test]
fn a_blockquote_soft_break_wraps_like_a_space(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let split = "> A blockquote reads one step down from the prose around it, on the bar it is\n\
                 > marked with. It must stay legible on the pane background.";
    let joined = "> A blockquote reads one step down from the prose around it, on the bar it is \
                  marked with. It must stay legible on the pane background.";

    assert_eq!(height(cx, split, px(430.)), height(cx, joined, px(430.)));
}

#[gpui::test]
fn style_boundaries_do_not_change_prose_layout(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let width = px(330.);
    for (label, plain, styled) in [
        (
            "paragraph",
            "alpha beta gamma delta epsilon zeta eta theta",
            "[alpha](https://a.test) [beta](https://b.test) gamma delta epsilon zeta eta theta",
        ),
        (
            "heading",
            "## alpha beta gamma delta epsilon",
            "## [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
        ),
        (
            "list",
            "- alpha beta gamma delta epsilon",
            "- [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
        ),
        (
            "blockquote",
            "> alpha beta gamma delta epsilon",
            "> [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
        ),
        (
            "footnote",
            "[^n]: alpha beta gamma delta epsilon",
            "[^n]: [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
        ),
    ] {
        assert_eq!(
            bounds_of(cx, plain, width, "md-probe"),
            bounds_of(cx, styled, width, "md-probe"),
            "{label}"
        );
    }
}

#[gpui::test]
fn table_columns_ignore_inline_style_boundaries(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let plain = "| alpha beta gamma delta | short |\n| --- | --- |\n| body | value |";
    let styled = "| [alpha](https://a.test) [beta](https://b.test) gamma delta | short |\n\
                  | --- | --- |\n| body | value |";

    assert_eq!(
        bounds_of(cx, plain, px(430.), "markdown-table-first-header-cell"),
        bounds_of(cx, styled, px(430.), "markdown-table-first-header-cell")
    );
}

#[gpui::test]
fn inline_images_do_not_split_surrounding_text_into_equal_columns(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let plain = "alpha beta gamma delta epsilon zeta eta theta iota kappa [thumbnail] tail";
    let image = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                 ![thumbnail](missing.png) tail";

    assert_eq!(height(cx, plain, px(635.)), height(cx, image, px(635.)));
}

/// The window/probe/measure setup the link tests share. Returning the two
/// recorders keeps each test to its own assertions: what the URL handler
/// saw, and whether the press still reached block selection.
#[allow(clippy::type_complexity)]
fn open_link_probe(
    cx: &mut TestAppContext,
    md: &str,
) -> (
    VisualTestContext,
    Arc<Mutex<Vec<String>>>,
    Arc<Mutex<usize>>,
) {
    crate::test_support::init_gpui_component(cx);
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(430.), px(220.)));
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    let opened = Arc::new(Mutex::new(Vec::new()));
    let block_mouse_downs = Arc::new(Mutex::new(0));
    let md = md.to_owned();
    let window = cx
        .update(|cx| {
            let opened = opened.clone();
            let block_mouse_downs = block_mouse_downs.clone();
            cx.open_window(options, |_window, cx| {
                cx.new(|_| LinkProbe {
                    md,
                    opened,
                    block_mouse_downs,
                })
            })
        })
        .expect("window opens");
    let vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    (vcx, opened, block_mouse_downs)
}

/// A URL this host will not open must not be clickable at all. Left in the
/// range list it would take the pointer cursor and consume the click,
/// discarding the block selection the press would otherwise have started —
/// a link that does nothing whatsoever.
#[gpui::test]
fn a_link_this_host_will_not_open_is_not_clickable(cx: &mut TestAppContext) {
    let (mut vcx, opened, block_mouse_downs) = open_link_probe(cx, "[docs](./other.md) tail");
    let text = vcx.debug_bounds("md-plain").expect("text painted");
    let inside_link = point(text.origin.x + px(2.), text.center().y);

    vcx.simulate_click(inside_link, Modifiers::none());
    assert!(
        opened.lock().unwrap().is_empty(),
        "a relative link must not reach the URL handler"
    );
    assert!(
        *block_mouse_downs.lock().unwrap() > 0,
        "the click falls through to block selection instead of vanishing"
    );
}

#[gpui::test]
fn interactive_text_opens_only_a_pressed_link_range(cx: &mut TestAppContext) {
    let (mut vcx, opened, block_mouse_downs) =
        open_link_probe(cx, "[open](https://example.com/right) tail");
    let text = vcx.debug_bounds("md-plain").expect("text painted");
    let inside_link = point(text.origin.x + px(2.), text.center().y);
    let outside_link = point(text.origin.x + text.size.width - px(2.), text.center().y);

    vcx.simulate_click(inside_link, Modifiers::none());
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["https://example.com/right"]
    );
    assert_eq!(*block_mouse_downs.lock().unwrap(), 0);

    let prior_block_downs = *block_mouse_downs.lock().unwrap();
    vcx.simulate_click(outside_link, Modifiers::none());
    assert_eq!(opened.lock().unwrap().len(), 1);
    assert!(*block_mouse_downs.lock().unwrap() > prior_block_downs);

    vcx.simulate_mouse_down(inside_link, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(outside_link, MouseButton::Left, Modifiers::none());
    assert_eq!(opened.lock().unwrap().len(), 1);

    let prior_block_downs = *block_mouse_downs.lock().unwrap();
    let moved_inside_link = point(inside_link.x + px(1.), inside_link.y);
    vcx.simulate_mouse_down(inside_link, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        moved_inside_link,
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.simulate_mouse_up(moved_inside_link, MouseButton::Left, Modifiers::none());
    assert_eq!(opened.lock().unwrap().len(), 2);
    assert_eq!(*block_mouse_downs.lock().unwrap(), prior_block_downs);

    let prior_block_downs = *block_mouse_downs.lock().unwrap();
    vcx.simulate_mouse_down(inside_link, MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(inside_link, MouseButton::Right, Modifiers::none());
    assert_eq!(opened.lock().unwrap().len(), 2);
    assert_eq!(*block_mouse_downs.lock().unwrap(), prior_block_downs);
}

#[gpui::test]
fn linked_images_open_without_committing_block_selection(cx: &mut TestAppContext) {
    let (mut vcx, opened, block_mouse_downs) =
        open_link_probe(cx, "[![thumbnail](missing.png)](https://example.com/full)");
    let image = vcx
        .debug_bounds("markdown-linked-image")
        .expect("linked image painted");

    vcx.simulate_click(image.center(), Modifiers::none());
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["https://example.com/full"]
    );
    assert_eq!(*block_mouse_downs.lock().unwrap(), 0);

    vcx.simulate_mouse_down(image.center(), MouseButton::Right, Modifiers::none());
    vcx.simulate_mouse_up(image.center(), MouseButton::Right, Modifiers::none());
    assert_eq!(opened.lock().unwrap().len(), 1);
}

/// The image path builds its own click target, so it needs the same gate
/// the text path gets: an unopenable link around an image must leave a
/// plain image behind, not a pointer cursor over a dead click.
#[gpui::test]
fn an_image_linked_to_an_unopenable_url_is_not_clickable(cx: &mut TestAppContext) {
    let (mut vcx, opened, block_mouse_downs) =
        open_link_probe(cx, "[![thumbnail](missing.png)](./doc.md)");
    assert!(
        vcx.debug_bounds("markdown-linked-image").is_none(),
        "no click target is built for a link this host will not open"
    );

    // No selector of its own once the link is gone, and the image is the
    // probe's only content — so the body's centre is over its block.
    let body = vcx.debug_bounds("md-link-probe").expect("body painted");
    vcx.simulate_click(body.center(), Modifiers::none());
    assert!(opened.lock().unwrap().is_empty());
    assert!(
        *block_mouse_downs.lock().unwrap() > 0,
        "the press reaches block selection instead of being swallowed"
    );
}

/// The gap a loose list takes is the one between blocks, so three items
/// grow by exactly two steps. Asserting the step and not just `>` is what
/// catches the gap being swapped for some other constant.
#[gpui::test]
fn a_loose_list_spaces_its_items(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let step = px(theme::MD_BLOCK_GAP - theme::MD_LIST_ITEM_GAP);

    for engine in Engine::ALL {
        for (label, tight, loose) in [
            (
                "bullet",
                "- AAAA\n- BBBB\n- CCCC",
                "- AAAA\n\n- BBBB\n\n- CCCC",
            ),
            (
                "ordered",
                "1. AAAA\n2. BBBB\n3. CCCC",
                "1. AAAA\n\n2. BBBB\n\n3. CCCC",
            ),
        ] {
            let tight = bounds_for_engine(cx, engine, tight, px(430.), "md-probe")
                .expect("body painted")
                .height;
            let loose = bounds_for_engine(cx, engine, loose, px(430.), "md-probe")
                .expect("body painted")
                .height;
            assert!(
                loose > tight,
                "{} {label}: loose={loose:?} tight={tight:?}",
                engine.label()
            );
            if engine == Engine::FileViewer {
                assert_eq!(loose - tight, step * 2., "{label}");
            }
        }
    }
}

/// CommonMark has two loose-list spellings. Both renderers must treat an
/// item containing two blocks like the equivalent list whose following
/// item is also separated by a blank line.
#[gpui::test]
fn both_engines_recognize_a_multi_block_item_as_loose(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);

    for engine in Engine::ALL {
        for (label, multi_block, blank_between) in [
            (
                "bullet",
                "- AAAA\n\n  cont\n- BBBB",
                "- AAAA\n\n  cont\n\n- BBBB",
            ),
            (
                "ordered",
                "1. AAAA\n\n   cont\n2. BBBB",
                "1. AAAA\n\n   cont\n\n2. BBBB",
            ),
        ] {
            let multi = bounds_for_engine(cx, engine, multi_block, px(430.), "md-probe")
                .expect("body painted")
                .height;
            let blank = bounds_for_engine(cx, engine, blank_between, px(430.), "md-probe")
                .expect("body painted")
                .height;
            assert_eq!(
                multi,
                blank,
                "{} {label}: multi-block={multi:?} blank-line={blank:?}",
                engine.label()
            );
        }
    }
}

#[gpui::test]
fn both_engines_render_task_markers_as_checkboxes(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);

    for engine in Engine::ALL {
        for md in ["- [ ] unchecked", "- [x] checked"] {
            assert!(
                bounds_for_engine(cx, engine, md, px(430.), "markdown-task-checkbox").is_some(),
                "{} did not paint a checkbox for {md:?}",
                engine.label()
            );
        }
        assert!(
            bounds_for_engine(cx, engine, "- plain", px(430.), "markdown-task-checkbox").is_none(),
            "{} painted a checkbox for a plain item",
            engine.label()
        );
    }
}

/// A paragraph break is block-level. Left inside a wrapping row as a
/// full-width flex item, it collapsed the *preceding* text element to zero
/// width — the prose then wrapped one character per line and ran over
/// whatever sat below. Block height never moved, so only the text element's
/// own painted width catches it.
///
/// Every block that can hold a break is covered: the list item was fixed
/// first and the blockquote stayed broken behind it. The assertion is the
/// collapse itself, not a width the three would share — the footnote's
/// inline label leaves it a different amount of room.
#[gpui::test]
fn a_second_paragraph_does_not_squeeze_the_first(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let long = "the quick brown fox jumps over the lazy dog and keeps running far";
    let w = px(635.);

    for engine in Engine::ALL {
        for (label, two) in [
            ("list item", format!("- {long}\n\n  {long}")),
            ("blockquote", format!("> {long}\n>\n> {long}")),
            ("footnote", format!("[^a]: {long}\n\n    {long}")),
        ] {
            let first = match engine {
                Engine::FileViewer => bounds_for_engine(cx, engine, &two, w, "md-plain"),
                Engine::AgentChat => measure_engine(cx, engine, &two, w, "md-probe").first_inline,
            }
            .unwrap_or_else(|| panic!("{} {label}: no prose painted", engine.label()))
            .width;
            assert!(
                first > px(0.),
                "{} {label}: a second paragraph squeezed the first to {first:?}",
                engine.label()
            );
        }
    }
}

/// Every host that puts its prose in a column beside a cell — a list item, a
/// blockquote, a footnote definition — must lay text out like the paragraph
/// that has no such column. An inline image is what exposes a difference: it
/// is the only span that splits a run, so the text loses its `flex_1().w_0()`
/// fill shape and becomes the shrink-to-fit part the column's layout decides.
#[gpui::test]
fn an_inline_image_does_not_collapse_the_text_beside_it(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let width = px(800.);
    let inline_image = "item with ![red](p.png) inline";
    let reference = bounds_of(cx, &format!("{inline_image}\n"), width, "md-plain");
    // A collapsed reference would make every comparison below vacuous.
    let line = height(cx, "x\n", width) - height(cx, "", width);
    assert_eq!(
        reference.height, line,
        "the reference paragraph must be one line, got {reference:?}"
    );

    for (host, md) in [
        ("list item", format!("- {inline_image}\n")),
        ("blockquote", format!("> {inline_image}\n")),
        ("footnote definition", format!("[^a]: {inline_image}\n")),
    ] {
        let measured = bounds_of(cx, &md, width, "md-plain");
        assert_eq!(
            measured, reference,
            "a {host} must lay its text out like the paragraph mirroring it"
        );
    }
}

/// The prose column stacks a multi-paragraph item's runs with a margin rather
/// than flex `gap` (see `render_md_prose`); this pins that spacing.
#[gpui::test]
fn a_multi_paragraph_item_keeps_the_gap_between_its_runs(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let width = px(300.);
    // One line, measured against a body that holds nothing but its padding.
    let line = height(cx, "x\n", width) - height(cx, "", width);
    let one_run = height(cx, "- first para\n", width);
    let two_runs = height(cx, "- first para\n\n  second para\n", width);

    assert_eq!(
        two_runs - one_run,
        line + px(theme::MD_BLOCK_GAP),
        "a second run costs its own line plus one block gap"
    );
}

/// Text sharing a wrapping row with an inline image keeps no flex basis, so
/// only `min_w_0` plus the default shrink lets it wrap instead of running past
/// the pane. Guards `render_prose_run` for every host: the same text without an
/// image is the reference.
#[gpui::test]
fn text_beside_an_inline_image_wraps_like_text_without_one(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    const LONG: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                        lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega";
    let width = px(300.);
    // The image comes first so the long text is the last `md-plain` measured.
    for (host, prefix) in [("paragraph", ""), ("list item", "- "), ("blockquote", "> ")] {
        let beside_image = bounds_of(
            cx,
            &format!("{prefix}![red](p.png) {LONG}\n"),
            width,
            "md-plain",
        );
        let alone = bounds_of(cx, &format!("{prefix}{LONG}\n"), width, "md-plain");

        assert!(
            beside_image.width <= width,
            "a {host}'s text must stay inside the pane, got {beside_image:?} in {width:?}"
        );
        assert_eq!(
            beside_image, alone,
            "an inline image must not change how a {host}'s text wraps"
        );
    }
}
