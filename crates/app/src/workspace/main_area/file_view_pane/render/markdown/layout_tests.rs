use std::rc::Rc;
use std::sync::{Arc, Mutex};

use super::selection::{
    queue_block_selection, take_pending_block_selection, take_pending_block_selection_for_drag,
};
use super::{MdColors, OpenUrl, render_md_body_layout};
use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::markdown_viewer::parse_markdown;
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
        let blocks = parse_markdown(&self.md, "default", false);
        let opened = self.opened.clone();
        let on_open_url: OpenUrl = Rc::new(move |url, _, _| {
            opened.lock().unwrap().push(url.to_owned());
        });
        let block_mouse_downs = self.block_mouse_downs.clone();
        let body = render_md_body_layout(
            &blocks,
            None,
            &colors,
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
                let blocks = parse_markdown(&self.md, "default", false);
                let body = render_md_body_layout(
                    &blocks,
                    None,
                    &colors,
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
