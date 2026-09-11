use crate::text::TextView;
use gpui::{
    AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, point, px, size,
};

struct Probe {
    source: SharedString,
    format: ProbeFormat,
    width: Pixels,
    font_size: Pixels,
}

#[derive(Clone, Copy)]
enum ProbeFormat {
    Markdown,
    Html,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = match self.format {
            ProbeFormat::Markdown => {
                TextView::markdown("table-probe", self.source.clone(), window, cx)
            }
            ProbeFormat::Html => TextView::html("table-probe", self.source.clone(), window, cx),
        };
        div()
            .w(self.width)
            .font_family(".ZedMono")
            .text_size(self.font_size)
            .child(text.w_full())
    }
}

fn probe(
    cx: &mut TestAppContext,
    markdown: &str,
    width: f32,
) -> (gpui::Entity<Probe>, VisualTestContext) {
    probe_with_format(cx, markdown, width, ProbeFormat::Markdown)
}

fn probe_with_format(
    cx: &mut TestAppContext,
    source: &str,
    width: f32,
    format: ProbeFormat,
) -> (gpui::Entity<Probe>, VisualTestContext) {
    cx.update(crate::init);
    let source = SharedString::from(source.to_string());
    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1600.), px(1600.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Probe {
                        source,
                        format,
                        width: px(width),
                        font_size: px(16.),
                    })
                },
            )
        })
        .unwrap();
    let entity = window.entity(cx).unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    (entity, vcx)
}

fn cell(cx: &mut VisualTestContext, row: usize, column: usize) -> Bounds<Pixels> {
    const SELECTORS: [[&str; 3]; 4] = [
        [
            "markdown-table-cell-0-0",
            "markdown-table-cell-0-1",
            "markdown-table-cell-0-2",
        ],
        [
            "markdown-table-cell-1-0",
            "markdown-table-cell-1-1",
            "markdown-table-cell-1-2",
        ],
        [
            "markdown-table-cell-2-0",
            "markdown-table-cell-2-1",
            "markdown-table-cell-2-2",
        ],
        [
            "markdown-table-cell-3-0",
            "markdown-table-cell-3-1",
            "markdown-table-cell-3-2",
        ],
    ];
    cx.debug_bounds(SELECTORS[row][column])
        .expect("cell painted")
}

const MIXED: &str = "| Commit | Description | State |\n|---|---|---|\n| `a1b2c3d4` | A long sentence with many short words that should receive the remaining table width and wrap naturally. | OK |\n| `e5f6a7b8` | Short. | Done |\n| empty | | |";

#[gpui::test]
fn short_columns_keep_their_tokens_and_prose_gets_the_space(cx: &mut TestAppContext) {
    let (_, mut vcx) = probe(cx, MIXED, 600.);
    let hash = cell(&mut vcx, 1, 0);
    let prose = cell(&mut vcx, 1, 1);
    assert!(prose.size.width > hash.size.width * 2.);
    for row in 0..4 {
        for column in 0..3 {
            let header = cell(&mut vcx, 0, column);
            let body = cell(&mut vcx, row, column);
            assert_eq!(header.origin.x, body.origin.x);
            assert_eq!(header.size.width, body.size.width);
            assert_eq!(body.origin.y, cell(&mut vcx, row, 0).origin.y);
            assert_eq!(body.size.height, cell(&mut vcx, row, 0).size.height);
        }
    }
    let viewport = vcx.debug_bounds("markdown-table-viewport").unwrap();
    assert!(cell(&mut vcx, 1, 2).right() <= viewport.right() + px(1.));
}

#[gpui::test]
fn html_tables_derive_columns_from_their_rows(cx: &mut TestAppContext) {
    let (_, mut vcx) = probe_with_format(
        cx,
        "<table><tr><th>A</th><th>Description</th></tr><tr><td>1</td><td>prose</td></tr></table>",
        400.,
        ProbeFormat::Html,
    );
    let first = cell(&mut vcx, 1, 0);
    let second = cell(&mut vcx, 1, 1);
    assert!(first.size.width > px(0.));
    assert!(second.size.width > px(0.));
    assert_eq!(cell(&mut vcx, 0, 0).origin.x, first.origin.x);
    assert_eq!(cell(&mut vcx, 0, 1).origin.x, second.origin.x);
}

#[gpui::test]
fn resize_reflows_prose_without_changing_the_hash_column(cx: &mut TestAppContext) {
    let (view, mut vcx) = probe(cx, MIXED, 700.);
    let hash = cell(&mut vcx, 1, 0).size.width;
    let wide = cell(&mut vcx, 1, 1);
    view.update(&mut vcx, |view, cx| {
        view.width = px(400.);
        cx.notify();
    });
    vcx.run_until_parked();
    let narrow = cell(&mut vcx, 1, 1);
    assert_eq!(hash, cell(&mut vcx, 1, 0).size.width);
    assert!(narrow.size.width < wide.size.width);
    assert!(
        narrow.size.height > wide.size.height,
        "narrow={narrow:?}, wide={wide:?}"
    );
    view.update(&mut vcx, |view, cx| {
        view.font_size = px(24.);
        cx.notify();
    });
    vcx.run_until_parked();
    assert!(cell(&mut vcx, 1, 0).size.width > hash);
}

#[gpui::test]
fn narrow_tables_scroll_without_crushing_identifiers(cx: &mut TestAppContext) {
    let (_, mut vcx) = probe(
        cx,
        "| Identifier | Other |\n|---|---|\n| abcdefghijklmnopqrstuvwxyz | 01234567890123456789 |",
        180.,
    );
    let viewport = vcx.debug_bounds("markdown-table-viewport").unwrap();
    assert!(viewport.size.width <= px(180.));
    assert!(cell(&mut vcx, 1, 1).right() > viewport.right());
    assert_eq!(
        cell(&mut vcx, 1, 0).size.height,
        cell(&mut vcx, 0, 0).size.height + px(1.)
    );
    let before = cell(&mut vcx, 1, 0).origin.x;
    vcx.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-100.), px(0.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert!(cell(&mut vcx, 1, 0).origin.x < before);
    assert_eq!(cell(&mut vcx, 0, 0).origin.x, cell(&mut vcx, 1, 0).origin.x);
}

#[gpui::test]
fn reading_past_a_table_does_not_drag_its_columns(cx: &mut TestAppContext) {
    let (_, mut vcx) = probe(
        cx,
        "| Identifier | Other |\n|---|---|\n| abcdefghijklmnopqrstuvwxyz | 01234567890123456789 |",
        180.,
    );
    let viewport = vcx.debug_bounds("markdown-table-viewport").unwrap();
    let before = cell(&mut vcx, 1, 0).origin.x;
    vcx.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-100.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert_eq!(before, cell(&mut vcx, 1, 0).origin.x);
}

#[gpui::test]
async fn newly_streamed_content_resizes_every_row(cx: &mut TestAppContext) {
    use smol::stream::StreamExt as _;
    let (view, mut vcx) = probe(cx, MIXED, 600.);
    let initial = cell(&mut vcx, 1, 0).size.width;
    let mut notifications = vcx.notifications(&view);
    view.update(&mut vcx, |view, cx| {
        view.source = MIXED
            .replace("`e5f6a7b8`", "`abcdefghijklmnopqrstuvwx`")
            .into();
        cx.notify();
    });
    // TextView reparses on a real smol debounce timer, outside GPUI's clock.
    vcx.executor().allow_parking();
    smol::future::or(
        async {
            loop {
                vcx.run_until_parked();
                if cell(&mut vcx, 1, 0).size.width > initial {
                    break;
                }
                notifications.next().await.expect("probe is live");
            }
        },
        async {
            smol::Timer::after(std::time::Duration::from_secs(3)).await;
            panic!("streamed table did not reflow");
        },
    )
    .await;
    assert_eq!(
        cell(&mut vcx, 0, 0).size.width,
        cell(&mut vcx, 2, 0).size.width
    );
}

#[gpui::test]
async fn streamed_updates_preserve_horizontal_scroll(cx: &mut TestAppContext) {
    use smol::stream::StreamExt as _;
    let initial =
        "| Identifier | Other |\n|---|---|\n| abcdefghijklmnopqrstuvwxyz | 01234567890123456789 |";
    let (view, mut vcx) = probe(cx, initial, 180.);
    let viewport = vcx.debug_bounds("markdown-table-viewport").unwrap();
    let unscrolled = cell(&mut vcx, 1, 0).origin.x;
    vcx.simulate_event(gpui::ScrollWheelEvent {
        position: viewport.center(),
        delta: gpui::ScrollDelta::Pixels(point(px(-100.), px(0.))),
        ..Default::default()
    });
    vcx.run_until_parked();
    let scrolled = cell(&mut vcx, 1, 0).origin.x;
    assert!(scrolled < unscrolled);

    let old_width = cell(&mut vcx, 1, 1).size.width;
    let mut notifications = vcx.notifications(&view);
    view.update(&mut vcx, |view, cx| {
        view.source = initial
            .replace("01234567890123456789", "012345678901234567890123456789")
            .into();
        cx.notify();
    });
    vcx.executor().allow_parking();
    smol::future::or(
        async {
            loop {
                vcx.run_until_parked();
                if cell(&mut vcx, 1, 1).size.width > old_width {
                    break;
                }
                notifications.next().await.expect("probe is live");
            }
        },
        async {
            smol::Timer::after(std::time::Duration::from_secs(3)).await;
            panic!("streamed table did not reflow");
        },
    )
    .await;
    assert_eq!(cell(&mut vcx, 1, 0).origin.x, scrolled);
}

#[gpui::test]
fn short_and_empty_tables_still_fill_the_available_width(cx: &mut TestAppContext) {
    for markdown in ["| A | B |\n|---|---|\n| 1 | 2 |", "| | |\n|---|---|\n| | |"] {
        let (_, mut vcx) = probe(cx, markdown, 400.);
        let viewport = vcx.debug_bounds("markdown-table-viewport").unwrap();
        assert!((cell(&mut vcx, 1, 1).right() - viewport.right()).abs() <= px(1.));
        assert!(cell(&mut vcx, 1, 0).size.width > px(0.));
    }
}
