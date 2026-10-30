use super::geometry::*;
use super::*;
use gpui::px;

#[test]
fn scroll_at_bottom_detects_bottom_top_and_slack() {
    // Content fits (no scroll) → trivially at bottom.
    assert!(scroll_at_bottom(0.0, 0.0, 24.0));
    // At the very bottom: max + offset == 0.
    assert!(scroll_at_bottom(-100.0, 100.0, 24.0));
    // Within slack of the bottom (10px from the edge, slack 24).
    assert!(scroll_at_bottom(-90.0, 100.0, 24.0));
    // At the top of a scrollable view → not at bottom.
    assert!(!scroll_at_bottom(0.0, 100.0, 24.0));
    // Scrolled up beyond slack (90px from the edge) → not at bottom.
    assert!(!scroll_at_bottom(-10.0, 100.0, 24.0));
}

#[test]
fn thumb_geometry_absence_cases() {
    assert_eq!(thumb_geometry(px(100.), px(100.), px(0.), px(0.)), None);
    assert_eq!(thumb_geometry(px(100.), px(80.), px(0.), px(0.)), None);
    // Bounds not yet measured (zero viewport).
    assert_eq!(thumb_geometry(px(0.), px(200.), px(0.), px(0.)), None);
    // viewport_h (20) < SCROLLBAR_MIN_THUMB_H (24): after clamping the
    // thumb would be taller than the track, producing a negative
    // track_h. No thumb is more useful than an inverted one.
    assert_eq!(thumb_geometry(px(20.), px(25.), px(0.), px(0.)), None);
}

#[test]
fn thumb_geometry_position_cases() {
    // viewport 100 of 200 → half-height thumb at the track origin.
    assert_eq!(
        thumb_geometry(px(100.), px(200.), px(0.), px(0.)),
        Some((px(0.), px(50.)))
    );

    // offset = -(content - viewport) = -100 → scroll_frac 1 → top = track_h.
    assert_eq!(
        thumb_geometry(px(100.), px(200.), px(-100.), px(0.)),
        Some((px(50.), px(50.)))
    );

    assert_eq!(
        thumb_geometry(px(100.), px(200.), px(-100.), px(30.)),
        Some((px(80.), px(50.)))
    );
}

struct ScrollAreaProbe {
    rows: usize,
    max_h: Pixels,
    handle: ScrollHandle,
}

impl gpui::Render for ScrollAreaProbe {
    fn render(&mut self, _window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let content = div()
            .flex()
            .flex_col()
            .children((0..self.rows).map(|i| div().child(SharedString::from(format!("row {i}")))));
        scroll_area("probe-scroll", self.max_h, content).track(&self.handle)
    }
}

/// Ground truth that the composed structure actually scrolls: content far
/// taller than `max_h` must produce a nonzero tracked `max_offset` after a
/// real paint. Guards the primitive's body wiring (`overflow_y_scroll` +
/// `track_scroll` + the height cap) — not observable from the code alone.
#[gpui::test]
async fn overflowing_scroll_area_computes_nonzero_scroll_offset(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let handle = ScrollHandle::default();
    let probe_handle = handle.clone();
    let (_probe, cx) = cx.add_window_view(move |_, _| ScrollAreaProbe {
        rows: 200,
        max_h: px(100.),
        handle: probe_handle,
    });
    cx.run_until_parked();

    assert!(
        handle.max_offset().y > px(0.),
        "200 text rows inside a 100px cap must overflow, got {:?}",
        handle.max_offset()
    );
    let wrapper = cx
        .debug_bounds("probe-scroll-wrapper")
        .expect("wrapper painted");
    let bar_layer = cx
        .debug_bounds("probe-scroll-scrollbar-layer")
        .expect("scrollbar overlay painted");
    assert_eq!(wrapper.size.height, px(100.), "wrapper capped at max_h");
    assert_eq!(
        bar_layer, wrapper,
        "scrollbar overlay must be pinned to the wrapper bounds"
    );
}

#[test]
fn thumb_geometry_clamping_cases() {
    // viewport 100 of 10000 → 1px raw thumb, clamped up to the min.
    let (_, thumb_h) = thumb_geometry(px(100.), px(10000.), px(0.), px(0.)).unwrap();
    assert_eq!(thumb_h, px(theme::SCROLLBAR_MIN_THUMB_H));

    // offset beyond the scrollable range must not push the thumb past
    // the track; track_h = 100 - 24 = 76.
    let (thumb_top, _) = thumb_geometry(px(100.), px(10000.), px(-99999.), px(0.)).unwrap();
    assert_eq!(thumb_top, px(100.) - px(theme::SCROLLBAR_MIN_THUMB_H));

    // The file viewer's path: minimum-clamped thumb, a mid-track scroll
    // fraction, and a non-zero top offset, all at once. raw thumb =
    // 100 * (100/500) = 20 → clamped to 24; track_h = 76; frac = 0.25;
    // thumb_top = 40 + 76 * 0.25 = 59. Pins how top_offset combines with
    // the clamped track against operator-precedence regressions.
    assert_eq!(
        thumb_geometry(px(100.), px(500.), px(-100.), px(40.)),
        Some((px(59.), px(theme::SCROLLBAR_MIN_THUMB_H)))
    );
}

#[test]
fn pointer_scroll_offset_cases() {
    // Track 0..100, thumb 50, scrollable 100 (viewport 100 of content 200).
    // A track press grabs the thumb at its centre, so the pointer lands there.
    assert_eq!(
        pointer_scroll_offset(px(25.), px(0.), px(100.), px(50.), px(25.), px(100.)),
        px(0.)
    );
    assert_eq!(
        pointer_scroll_offset(px(75.), px(0.), px(100.), px(50.), px(25.), px(100.)),
        px(-100.)
    );
    assert_eq!(
        pointer_scroll_offset(px(50.), px(0.), px(100.), px(50.), px(25.), px(100.)),
        px(-50.)
    );
    // A track that starts below its container's origin (a header above it).
    assert_eq!(
        pointer_scroll_offset(px(90.), px(40.), px(100.), px(50.), px(25.), px(100.)),
        px(-50.)
    );
    // A pointer past either end pins the region to that end.
    assert_eq!(
        pointer_scroll_offset(px(-500.), px(0.), px(100.), px(50.), px(25.), px(100.)),
        px(0.)
    );
    assert_eq!(
        pointer_scroll_offset(px(500.), px(0.), px(100.), px(50.), px(25.), px(100.)),
        px(-100.)
    );
    // A degenerate track carries no position at all.
    assert_eq!(
        pointer_scroll_offset(px(50.), px(0.), px(50.), px(50.), px(25.), px(100.)),
        px(0.)
    );
    assert_eq!(
        pointer_scroll_offset(px(50.), px(0.), px(100.), px(50.), px(25.), px(0.)),
        px(0.)
    );
}

#[gpui::test]
fn scroll_handle_target_moves_only_its_axis(cx: &mut gpui::TestAppContext) {
    let handle = ScrollHandle::new();
    handle.set_offset(gpui::point(px(-7.), px(-3.)));
    cx.update(|cx| handle.scroll_to(Axis::Vertical, px(-40.), cx));
    assert_eq!(handle.offset(), gpui::point(px(-7.), px(-40.)));
}

mod track_press_tests {
    use gpui::{
        Bounds, Context, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Render,
        ScrollHandle, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div,
        point, prelude::*, px, size,
    };

    use super::{Scrollbar, ScrollbarShow};
    use crate::test_support::init_gpui_component;

    /// A scroll area far taller than its window, with the draggable scrollbar
    /// laid over it. `Always` so the bar's listeners are installed without
    /// having to scroll or hover first.
    struct ScrollbarProbe {
        handle: ScrollHandle,
    }

    impl Render for ScrollbarProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .relative()
                .size_full()
                .child(
                    div()
                        .id("scroll-area")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.handle)
                        .child(div().w_full().h(px(4000.))),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .child(
                            Scrollbar::vertical(&self.handle)
                                .scrollbar_show(ScrollbarShow::Always)
                                .id("probe-scrollbar"),
                        ),
                )
        }
    }

    /// Click the track near its bottom — below the thumb, so this is the
    /// "jump to here" path, not a thumb drag. Reports the resulting offset,
    /// which is negative once the view has scrolled down.
    fn track_click_offset(cx: &mut TestAppContext, button: MouseButton) -> f32 {
        track_click_offset_at(cx, button, 360.)
    }

    /// [`track_click_offset`] at an arbitrary track `y`.
    fn track_click_offset_at(cx: &mut TestAppContext, button: MouseButton, y: f32) -> f32 {
        init_gpui_component(cx);
        let handle = ScrollHandle::new();
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(600.), px(400.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| {
                let handle = handle.clone();
                cx.open_window(opts, |_window, cx| cx.new(|_cx| ScrollbarProbe { handle }))
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();

        // The vertical bar is the right-edge strip; y near the bottom is track.
        let on_track = point(px(594.), px(y));
        vcx.simulate_event(MouseDownEvent {
            position: on_track,
            modifiers: Modifiers::default(),
            button,
            click_count: 1,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            position: on_track,
            modifiers: Modifiers::default(),
            button,
            click_count: 1,
        });
        vcx.run_until_parked();

        f32::from(handle.offset().y)
    }

    /// Control: the primary button must still jump the view to the track spot.
    #[gpui::test]
    fn a_left_press_on_the_track_jumps_the_view(cx: &mut TestAppContext) {
        assert!(
            track_click_offset(cx, MouseButton::Left) < 0.,
            "a left press on the scrollbar track no longer scrolls"
        );
    }

    /// A right press belongs to the context menu, not the scrollbar.
    #[gpui::test]
    fn a_right_press_on_the_track_does_not_jump_the_view(cx: &mut TestAppContext) {
        assert_eq!(
            track_click_offset(cx, MouseButton::Right),
            0.,
            "a right press on the scrollbar track scrolled the view"
        );
    }

    /// A press mid-track centres the thumb on the pointer, which maps onto the
    /// *scrollable* range (content minus viewport), not the whole content.
    /// Window 400, content 4000: the thumb is 48 (`MIN_THUMB_SIZE`), so a press
    /// at y=200 puts the thumb start at (200 - 24) / (400 - 48) = 0.5 of its
    /// travel, i.e. half of the 3600px scrollable range.
    #[gpui::test]
    fn a_mid_track_press_centres_the_thumb_on_the_pointer(cx: &mut TestAppContext) {
        let offset = track_click_offset_at(cx, MouseButton::Left, 200.);
        assert!(
            (offset + 1800.).abs() < 1.,
            "expected about -1800px, got {offset}"
        );
    }
}

/// The shared thumb's own pointer handling: track presses, thumb drags, and
/// what reaches the content underneath.
mod thumb_pointer_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{
        Bounds, Context, IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseExitEvent,
        MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollDelta, ScrollHandle,
        ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, Window, WindowBounds,
        WindowOptions, div, point, prelude::*, px, size,
    };

    use super::super::geometry::pointer_scroll_offset;
    use super::super::{Axis, ThumbTarget, take_press, vertical_thumb};
    use crate::test_support::init_gpui_component;
    use crate::theme;

    const WINDOW_W: f32 = 600.;
    const WINDOW_H: f32 = 400.;
    const CONTENT_H: f32 = 4000.;
    /// `400 * 400 / 4000`, above `SCROLLBAR_MIN_THUMB_H`.
    const THUMB_LEN: f32 = 40.;

    fn x_thumb(right: f32) -> Pixels {
        px(right - theme::SCROLLBAR_MARGIN_R - theme::SCROLLBAR_W / 2.)
    }
    fn x_strip(right: f32) -> Pixels {
        px(right - theme::SCROLLBAR_HIT_W / 2.)
    }
    fn x_outside(right: f32) -> Pixels {
        px(right - theme::SCROLLBAR_HIT_W - 10.)
    }

    /// A `ScrollHandle` target that also counts the drags it saw end.
    struct CountingTarget {
        handle: ScrollHandle,
        ended: Rc<Cell<usize>>,
    }

    impl ThumbTarget for CountingTarget {
        fn scroll_to(&self, axis: Axis, offset: Pixels, cx: &mut gpui::App) {
            self.handle.scroll_to(axis, offset, cx);
        }

        fn drag_ended(&self) {
            self.ended.set(self.ended.get() + 1);
        }
    }

    /// One scroll region with its thumb, filling a `.relative()` column.
    fn region(
        id: &'static str,
        handle: &ScrollHandle,
        content_h: f32,
        presses: &Rc<Cell<usize>>,
        thumb: &ThumbSwitch,
    ) -> impl IntoElement {
        let viewport_h = handle.bounds().size.height;
        let presses = presses.clone();
        div()
            .relative()
            .flex_1()
            .h_full()
            .child(
                div()
                    .id(format!("{id}-area"))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(handle)
                    .child({
                        let right = thumb.right_presses.clone();
                        div()
                            .w_full()
                            .h(px(content_h))
                            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                presses.set(presses.get() + 1)
                            })
                            .on_mouse_down(MouseButton::Right, move |_, _, _| {
                                right.set(right.get() + 1)
                            })
                    }),
            )
            .children(thumb.shown.get().then_some(()).and_then(|()| {
                vertical_thumb(
                    id,
                    CountingTarget {
                        handle: handle.clone(),
                        ended: thumb.ended.clone(),
                    },
                    viewport_h,
                    viewport_h + handle.max_offset().y,
                    handle.offset().y,
                    px(0.),
                    theme::SCROLLBAR_THUMB,
                    theme::SCROLLBAR_THUMB_HOVER,
                )
            }))
    }

    /// Whether the thumbs are drawn, and how many drags they saw end.
    #[derive(Clone)]
    struct ThumbSwitch {
        shown: Rc<Cell<bool>>,
        ended: Rc<Cell<usize>>,
        /// Right presses that reached the content under the strip.
        right_presses: Rc<Cell<usize>>,
    }

    struct Probe {
        handles: Vec<ScrollHandle>,
        content_h: f32,
        renders: Rc<Cell<usize>>,
        presses: Rc<Cell<usize>>,
        thumb: ThumbSwitch,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            const IDS: [&str; 2] = ["probe-thumb", "probe-thumb-2"];
            div().flex().size_full().children(
                self.handles
                    .iter()
                    .zip(IDS)
                    .map(|(h, id)| region(id, h, self.content_h, &self.presses, &self.thumb)),
            )
        }
    }

    struct Rig {
        vcx: VisualTestContext,
        handles: Vec<ScrollHandle>,
        renders: Rc<Cell<usize>>,
        presses: Rc<Cell<usize>>,
        thumb: ThumbSwitch,
    }

    fn rig(cx: &mut TestAppContext, regions: usize, content_h: f32) -> Rig {
        init_gpui_component(cx);
        let handles: Vec<_> = (0..regions).map(|_| ScrollHandle::new()).collect();
        let renders = Rc::new(Cell::new(0));
        let presses = Rc::new(Cell::new(0));
        let thumb = ThumbSwitch {
            shown: Rc::new(Cell::new(true)),
            ended: Rc::new(Cell::new(0)),
            right_presses: Rc::new(Cell::new(0)),
        };
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(WINDOW_W), px(WINDOW_H)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let window = cx
            .update(|cx| {
                let (handles, renders, presses, thumb) = (
                    handles.clone(),
                    renders.clone(),
                    presses.clone(),
                    thumb.clone(),
                );
                cx.open_window(opts, |_, cx| {
                    cx.new(|_| Probe {
                        handles,
                        content_h,
                        renders,
                        presses,
                        thumb,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        // The first frame measures the region; the thumb needs those bounds.
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
        Rig {
            vcx,
            handles,
            renders,
            presses,
            thumb,
        }
    }

    impl Rig {
        fn down(&mut self, at: Point<Pixels>, button: MouseButton) {
            self.vcx.simulate_event(MouseDownEvent {
                position: at,
                modifiers: Modifiers::default(),
                button,
                click_count: 1,
                first_mouse: false,
            });
            self.vcx.run_until_parked();
        }
        fn up(&mut self, at: Point<Pixels>, button: MouseButton) {
            self.vcx.simulate_event(MouseUpEvent {
                position: at,
                modifiers: Modifiers::default(),
                button,
                click_count: 1,
            });
            self.vcx.run_until_parked();
        }
        fn click(&mut self, at: Point<Pixels>, button: MouseButton) {
            self.down(at, button);
            self.up(at, button);
        }
        fn move_to(&mut self, at: Point<Pixels>, pressed: Option<MouseButton>) {
            self.vcx.simulate_event(MouseMoveEvent {
                position: at,
                modifiers: Modifiers::default(),
                pressed_button: pressed,
            });
            self.vcx.run_until_parked();
        }
        fn offset_y(&self, ix: usize) -> f32 {
            f32::from(self.handles[ix].offset().y)
        }
    }

    fn expected(pointer: f32, grab: f32) -> f32 {
        f32::from(pointer_scroll_offset(
            px(pointer),
            px(0.),
            px(WINDOW_H),
            px(THUMB_LEN),
            px(grab),
            px(CONTENT_H - WINDOW_H),
        ))
    }

    #[gpui::test]
    fn left_press_on_track_jumps_to_pointer(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.click(point(x_strip(WINDOW_W), px(360.)), MouseButton::Left);
        let want = expected(360., THUMB_LEN / 2.);
        assert!(
            (rig.offset_y(0) - want).abs() < 1.,
            "a track press must centre the thumb on the pointer: want {want}, got {}",
            rig.offset_y(0)
        );
    }

    #[gpui::test]
    fn right_press_on_track_does_not_scroll(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.click(point(x_strip(WINDOW_W), px(360.)), MouseButton::Right);
        assert_eq!(
            rig.offset_y(0),
            0.,
            "a right press belongs to the context menu"
        );
    }

    #[gpui::test]
    fn press_on_thumb_does_not_jump(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.click(point(x_thumb(WINDOW_W), px(5.)), MouseButton::Left);
        assert_eq!(rig.offset_y(0), 0., "grabbing the thumb must not move it");
    }

    #[gpui::test]
    fn dragging_thumb_follows_pointer(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        rig.move_to(point(x, px(205.)), Some(MouseButton::Left));
        let want = expected(205., 5.);
        assert!(
            (rig.offset_y(0) - want).abs() < 1.,
            "the grab point must stay under the pointer: want {want}, got {}",
            rig.offset_y(0)
        );
    }

    #[gpui::test]
    fn move_without_pressed_button_ends_drag(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        // The release happened outside the window, so no mouse-up arrived.
        rig.move_to(point(x, px(205.)), None);
        assert_eq!(rig.offset_y(0), 0.);
        rig.move_to(point(x, px(300.)), Some(MouseButton::Left));
        assert_eq!(
            rig.offset_y(0),
            0.,
            "a drag that lost its button must stay over"
        );
    }

    #[gpui::test]
    fn drag_ended_by_buttonless_move_repaints_host(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        let before = rig.renders.get();
        // Off the thumb, so the hover check alone sees no change.
        rig.move_to(point(x_outside(WINDOW_W), px(300.)), None);
        assert!(
            rig.renders.get() > before,
            "ending the drag must repaint, or its window-wide cursor stays"
        );
    }

    #[gpui::test]
    fn a_released_drag_ends_once(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        rig.up(point(x, px(5.)), MouseButton::Left);
        assert_eq!(rig.thumb.ended.get(), 1, "one drag, one end");
    }

    #[gpui::test]
    fn a_drag_ends_when_its_thumb_stops_painting(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.down(point(x_thumb(WINDOW_W), px(5.)), MouseButton::Left);
        // The pane switched away mid-drag: no listener is left for the release.
        rig.thumb.shown.set(false);
        rig.vcx.update(|window, _| window.refresh());
        rig.vcx.run_until_parked();
        assert_eq!(
            rig.thumb.ended.get(),
            1,
            "a list would keep its drag-frozen height forever"
        );
    }

    #[gpui::test]
    fn leaving_the_window_clears_thumb_hover(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.move_to(point(x_thumb(WINDOW_W), px(5.)), None);
        let before = rig.renders.get();
        rig.vcx.simulate_event(MouseExitEvent {
            position: point(px(WINDOW_W), px(5.)),
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        rig.vcx.run_until_parked();
        assert!(
            rig.renders.get() > before,
            "the thumb must repaint without its hover colour"
        );
    }

    #[gpui::test]
    fn releasing_on_the_thumb_keeps_it_hovered(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        rig.up(point(x, px(5.)), MouseButton::Left);
        let before = rig.renders.get();
        // Already hovered, so a move inside the thumb changes nothing.
        rig.move_to(point(x, px(6.)), None);
        assert_eq!(
            rig.renders.get(),
            before,
            "the release left the thumb idle under the pointer"
        );
    }

    #[gpui::test]
    fn press_beside_painted_thumb_grabs_it(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        // Inside the strip, in the margin right of the painted 4px thumb.
        let x = px(WINDOW_W - theme::SCROLLBAR_MARGIN_R / 2.);
        rig.down(point(x, px(5.)), MouseButton::Left);
        assert_eq!(
            rig.offset_y(0),
            0.,
            "a press level with the thumb must not jump"
        );
        rig.move_to(point(x, px(205.)), Some(MouseButton::Left));
        let want = expected(205., 5.);
        assert!(
            (rig.offset_y(0) - want).abs() < 1.,
            "the press must grab the thumb: want {want}, got {}",
            rig.offset_y(0)
        );
    }

    #[gpui::test]
    fn right_press_on_strip_reaches_content(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.click(point(x_strip(WINDOW_W), px(360.)), MouseButton::Right);
        assert_eq!(
            rig.thumb.right_presses.get(),
            1,
            "a context menu under the strip must still open"
        );
    }

    #[gpui::test]
    fn a_strip_press_is_recorded_once_at_its_position(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let at = point(x_strip(WINDOW_W), px(360.));
        rig.click(at, MouseButton::Left);
        rig.vcx.update(|_, cx| {
            assert!(take_press(at, cx), "the strip took this press");
            assert!(!take_press(at, cx), "a record answers once");
        });
        // A query for another press clears a stale record without matching it.
        rig.click(at, MouseButton::Left);
        rig.vcx.update(|_, cx| {
            assert!(!take_press(point(px(1.), px(1.)), cx), "another position");
            assert!(!take_press(at, cx), "the stale record is gone");
        });
    }

    #[gpui::test]
    fn track_press_is_not_seen_by_content_below(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.click(point(x_strip(WINDOW_W), px(360.)), MouseButton::Left);
        assert_eq!(rig.presses.get(), 0, "the strip must own its press");
        rig.click(point(x_outside(WINDOW_W), px(360.)), MouseButton::Left);
        assert_eq!(
            rig.presses.get(),
            1,
            "content beside the strip still gets presses"
        );
    }

    #[gpui::test]
    fn wheel_over_track_still_scrolls_content(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        rig.vcx.simulate_event(ScrollWheelEvent {
            position: point(x_strip(WINDOW_W), px(200.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        rig.vcx.run_until_parked();
        assert!(
            rig.offset_y(0) < 0.,
            "the wheel must pass through the strip"
        );
    }

    #[gpui::test]
    fn dragging_one_thumb_leaves_sibling_still(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 2, CONTENT_H);
        let x = x_thumb(WINDOW_W);
        rig.down(point(x, px(5.)), MouseButton::Left);
        rig.move_to(point(x, px(205.)), Some(MouseButton::Left));
        rig.up(point(x, px(205.)), MouseButton::Left);
        assert!(rig.offset_y(1) < 0., "the grabbed (right) region scrolls");
        assert_eq!(rig.offset_y(0), 0., "its sibling must not scroll along");
    }

    #[gpui::test]
    fn track_click_notifies_host_view(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let before = rig.renders.get();
        rig.click(point(x_strip(WINDOW_W), px(360.)), MouseButton::Left);
        assert!(
            rig.renders.get() > before,
            "the host must re-render so the thumb moves with the content"
        );
    }

    #[gpui::test]
    fn thumb_debug_selector_tracks_presence(cx: &mut TestAppContext) {
        let mut rig = rig(cx, 1, CONTENT_H);
        let strip = rig
            .vcx
            .debug_bounds("probe-thumb")
            .expect("overflow draws a thumb");
        assert_eq!(strip.size.width, px(theme::SCROLLBAR_HIT_W));

        let mut rig = self::rig(cx, 1, WINDOW_H / 2.);
        assert!(
            rig.vcx.debug_bounds("probe-thumb").is_none(),
            "content that fits draws no thumb"
        );
    }
}
