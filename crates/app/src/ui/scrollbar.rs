//! Vertical scrollbar thumb overlay — shared geometry + chrome.
//!
//! The right dock, files view, git-changes view, settings window, and
//! file viewer each render an absolutely-positioned thumb derived from a
//! viewport/content height pair and the current scroll offset. The
//! geometry math and the thumb chrome are identical; only the element id,
//! an optional top offset (the file viewer renders inside a container
//! that starts below its own origin), and the theme colours differ — so
//! those are the parameters. Callers keep their handle-extraction code
//! (handle types differ) and pass plain pixels in.
//!
//! [`horizontal_thumb`] is the X-axis mirror, for a region whose content
//! overflows sideways (an agent-chat embed's long, non-wrapped lines) — same
//! [`thumb_geometry`] math, transposed onto the width/left/bottom axis.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, Hsla, IntoElement, ListState, Pixels, Render, RenderOnce,
    ScrollHandle, SharedString, StyleRefinement, Window, div, prelude::*, px,
};
use gpui_component::StyledExt as _;

use crate::ui::theme;

/// The gpui_component built-in scrollbar (draggable track + thumb), for
/// regions where the display-only thumbs above aren't enough — e.g. the
/// status-bar Ports dropdown. Re-exported here so call sites stay on
/// `crate::ui::*` (shape C wrapper).
pub use gpui_component::scroll::{Scrollbar, ScrollbarShow};

/// Build the thumb overlay for a vertically-scrolling region, or `None`
/// when the content fits the viewport (or bounds are not yet measured).
///
/// `content_h` is the total scrollable content height: handles backed by
/// a `ScrollHandle` pass `viewport_h + max_offset().y`, while the
/// file viewer passes a precomputed height. `scroll_offset_y` is the
/// handle's `offset().y` (negative as the content scrolls up).
/// `top_offset` shifts the thumb down when the scroll region begins below
/// the positioned container's origin (the file viewer); pass `px(0.)`
/// otherwise. The thumb is positioned with `.right(SCROLLBAR_MARGIN_R)`,
/// so the caller's parent must be `.relative()`.
///
/// Display-only unless the caller chains [`Thumb::on_drag`].
///
/// Debug selector (test hook): the `id` string, so a test can assert whether a
/// thumb was drawn at all.
pub fn vertical_thumb(
    id: impl Into<ElementId>,
    viewport_h: Pixels,
    content_h: Pixels,
    scroll_offset_y: Pixels,
    top_offset: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<Thumb> {
    let (thumb_top, thumb_h) = thumb_geometry(viewport_h, content_h, scroll_offset_y, top_offset)?;
    Some(Thumb {
        id: id.into(),
        axis: Axis::Vertical,
        start: thumb_top,
        len: thumb_h,
        track: viewport_h,
        scrollable: content_h - viewport_h,
        offset: scroll_offset_y,
        bg: thumb_bg,
        hover_bg: thumb_hover_bg,
        on_drag: None,
    })
}

/// [`vertical_thumb`]'s horizontal mirror — for content that overflows on the X
/// axis (an agent-chat embed's long, non-wrapped lines). `scroll_offset_x` is the handle's
/// `offset().x` (negative as content scrolls right). Unlike [`vertical_thumb`]
/// there is no `top_offset` — an embed reserves its own bottom strip (see
/// `bounded_embed_height`) so the thumb sits flush at the bottom of its
/// container.
///
/// Debug selector (test hook): the `id` string, as in [`vertical_thumb`].
pub fn horizontal_thumb(
    id: impl Into<ElementId>,
    viewport_w: Pixels,
    content_w: Pixels,
    scroll_offset_x: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<Thumb> {
    let (thumb_left, thumb_w) = thumb_geometry(viewport_w, content_w, scroll_offset_x, px(0.))?;
    Some(Thumb {
        id: id.into(),
        axis: Axis::Horizontal,
        start: thumb_left,
        len: thumb_w,
        track: viewport_w,
        scrollable: content_w - viewport_w,
        offset: scroll_offset_x,
        bg: thumb_bg,
        hover_bg: thumb_hover_bg,
        on_drag: None,
    })
}

/// Which axis a [`Thumb`] rides.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

/// A scrollbar thumb overlay. Display-only until [`Thumb::on_drag`] installs a
/// handler, so the ten existing call sites keep the behaviour they had.
///
/// Absolutely positioned against the caller's `.relative()` parent.
#[derive(IntoElement)]
pub struct Thumb {
    id: ElementId,
    axis: Axis,
    /// Thumb offset from the track start, already including any `top_offset`.
    start: Pixels,
    len: Pixels,
    /// Viewport extent along the axis — the track the thumb rides.
    track: Pixels,
    /// `content - viewport`: how far the region can actually scroll.
    scrollable: Pixels,
    /// The region's current scroll offset (negative as content moves away).
    offset: Pixels,
    bg: Hsla,
    hover_bg: Hsla,
    #[allow(clippy::type_complexity)]
    on_drag: Option<Rc<dyn Fn(Pixels, &mut Window, &mut App)>>,
}

impl Thumb {
    /// Make the thumb draggable: `handler` receives the scroll offset the new
    /// thumb position implies (negative, already clamped to the scrollable
    /// range) and writes it back to whatever handle the region scrolls on.
    ///
    /// Opt-in because the handle types differ per call site and most regions are
    /// content-driven; a thumb without this stays display-only.
    pub fn on_drag(mut self, handler: impl Fn(Pixels, &mut Window, &mut App) + 'static) -> Self {
        self.on_drag = Some(Rc::new(handler));
        self
    }
}

/// Live drag state, carried by gpui's active drag so it survives the re-renders
/// each move triggers.
///
/// `last` is the previous move's cursor position, so each move applies a
/// *delta*: deriving an absolute thumb position instead would need the track's
/// origin in window coordinates, which a thumb positioned against a parent it
/// cannot measure does not have.
///
/// `id` is which thumb is being dragged. `on_drag_move::<T>` dispatches on the
/// payload's **type** alone (gpui `div.rs`) and runs in the capture phase, so
/// every thumb in the window sees every `ThumbDrag` — without this, a vertical
/// and a horizontal thumb would overwrite each other's `last` with values from
/// different axes, and a second embed's thumbs would scroll in lockstep with the
/// one actually grabbed.
#[derive(Clone)]
struct ThumbDrag {
    id: ElementId,
    last: Rc<Cell<Option<Pixels>>>,
}

/// gpui requires a rendered view to follow the cursor during a drag; a
/// scrollbar wants none, so this paints nothing.
struct ThumbDragGhost;

impl Render for ThumbDragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl RenderOnce for Thumb {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let thickness = px(theme::SCROLLBAR_W);
        let selector = self.id.clone();
        let own_id = self.id.clone();
        let vertical = self.axis == Axis::Vertical;
        let hover_bg = self.hover_bg;
        let mut thumb = div()
            .id(self.id.clone())
            .debug_selector(move || selector.to_string())
            .absolute()
            .rounded(thickness / 2.0)
            .bg(self.bg)
            .hover(move |d| d.bg(hover_bg));
        thumb = if vertical {
            thumb
                .top(self.start)
                .right(px(theme::SCROLLBAR_MARGIN_R))
                .w(thickness)
                .h(self.len)
        } else {
            thumb
                .left(self.start)
                .bottom(px(theme::SCROLLBAR_MARGIN_R))
                .h(thickness)
                .w(self.len)
        };
        let Some(handler) = self.on_drag else {
            return thumb;
        };
        // Cursor travel maps onto scroll travel by the ratio of the two ranges,
        // so the content keeps pace with the thumb regardless of their sizes.
        let track_travel = self.track - self.len;
        let scrollable = self.scrollable;
        let offset = self.offset;
        let drag = ThumbDrag {
            id: self.id,
            last: Rc::new(Cell::new(None)),
        };
        thumb
            .cursor_pointer()
            .on_drag(drag, |_, _, _, cx| cx.new(|_| ThumbDragGhost))
            .on_drag_move::<ThumbDrag>(move |ev, window, cx| {
                if track_travel <= px(0.) || scrollable <= px(0.) {
                    return;
                }
                let drag = ev.drag(cx);
                // Not our drag: this handler fires for every `ThumbDrag` in the
                // window, including the other axis' thumb on this very embed.
                if drag.id != own_id {
                    return;
                }
                let cursor = if vertical {
                    ev.event.position.y
                } else {
                    ev.event.position.x
                };
                let last = drag.last.replace(Some(cursor));
                // The first move only anchors: without a previous position there
                // is no travel yet, and treating the grab point as travel would
                // jump the content by however far into the thumb the user clicked.
                let Some(last) = last else {
                    return;
                };
                let travelled = cursor - last;
                if travelled == px(0.) {
                    return;
                }
                let next =
                    (offset - scrollable * (travelled / track_travel)).clamp(-scrollable, px(0.));
                handler(next, window, cx);
            })
    }
}

/// Pure thumb geometry: `(thumb_start, thumb_len)` in pixels along one axis,
/// or `None` when the content fits the viewport (so no thumb is drawn).
/// Axis-agnostic 1-D math — [`vertical_thumb`] feeds it height/Y values,
/// [`horizontal_thumb`] feeds it width/X values. Split out so the math is
/// testable without a window.
fn thumb_geometry(
    viewport_h: Pixels,
    content_h: Pixels,
    scroll_offset_y: Pixels,
    top_offset: Pixels,
) -> Option<(Pixels, Pixels)> {
    if content_h <= viewport_h || viewport_h <= px(0.) {
        return None;
    }
    let thumb_ratio = (viewport_h / content_h).min(1.0_f32);
    let thumb_h = (viewport_h * thumb_ratio).max(px(theme::SCROLLBAR_MIN_THUMB_H));
    if thumb_h >= viewport_h {
        return None;
    }
    let track_h = viewport_h - thumb_h;
    let scrollable = content_h - viewport_h;
    let scroll_frac = ((-scroll_offset_y) / scrollable).clamp(0.0_f32, 1.0_f32);
    Some((top_offset + track_h * scroll_frac, thumb_h))
}

/// [`vertical_thumb`] for a virtualized [`gpui::list`] / [`ListState`]. Derives
/// the viewport / content / offset geometry from the list state's scrollbar API
/// (the method names — `viewport_bounds`, `max_offset_for_scrollbar`,
/// `scroll_px_offset_for_scrollbar` — are non-obvious) so every gpui-`list` pane
/// gets the same display-only daruda thumb without re-deriving it. The geometry
/// reflects the previous frame's layout, which is fine for a thumb; read it at
/// *render* time. `None` until the first layout or when the content fits.
pub fn vertical_thumb_for_list(
    id: impl Into<ElementId>,
    list_state: &ListState,
    top_offset: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<Thumb> {
    let viewport_h = list_state.viewport_bounds().size.height;
    let content_h = viewport_h + list_state.max_offset_for_scrollbar().y;
    vertical_thumb(
        id,
        viewport_h,
        content_h,
        list_state.scroll_px_offset_for_scrollbar().y,
        top_offset,
        thumb_bg,
        thumb_hover_bg,
    )
}

/// Whether a virtualized [`ListState`] is scrolled to (within `slack` px of) the
/// bottom — drives a scroll-to-bottom affordance's visibility (read at render
/// time). Before the first layout both extents are zero, so this returns `true`
/// (no overflow yet → the affordance stays hidden, the desired first-frame
/// state).
pub fn list_at_bottom(list_state: &ListState, slack: f32) -> bool {
    scroll_at_bottom(
        f32::from(list_state.scroll_px_offset_for_scrollbar().y),
        f32::from(list_state.max_offset_for_scrollbar().y),
        slack,
    )
}

/// Pure: is a scroll region within `slack` of the bottom? `offset_y <= 0` (more
/// negative = scrolled further down) and `max_y >= 0` is the bottom extent, so
/// at the bottom `max_y + offset_y ≈ 0`. Content that fits (`max_y <= 0`) is
/// trivially at the bottom. Split out so it is testable without a laid-out list.
fn scroll_at_bottom(offset_y: f32, max_y: f32, slack: f32) -> bool {
    max_y <= 0.0 || (max_y + offset_y) <= slack
}

/// Vertically scrolling region capped at `max_h`, with the built-in draggable
/// [`Scrollbar`] pinned over a right gutter. Owns every layout invariant a
/// hand-assembled version gets wrong: the scrollbar must sit in an inset-0
/// absolute container (a bare `Scrollbar` has auto insets, and in a non-flex
/// parent Taffy resolves that to the static position *below* the body — the
/// ports-dropdown misplacement), the body and bar must share one tracked
/// handle, and content needs [`theme::SCROLL_AREA_GUTTER`] to clear the thumb.
/// Thumb visibility follows the theme (`scrollbar_show = Always` in daruda);
/// the bar still hides itself when content fits.
///
/// Debug selectors (test hooks): `{id}-wrapper` and `{id}-scrollbar-layer`.
#[derive(IntoElement)]
pub struct ScrollArea {
    id: SharedString,
    max_h: Pixels,
    handle: Option<ScrollHandle>,
    style: StyleRefinement,
    content: AnyElement,
}

/// Build a [`ScrollArea`]. Width comes from the content (plus the gutter);
/// chain `Styled` methods for overrides. The scroll handle is internal,
/// keyed by `id` — use [`ScrollArea::track`] when the caller needs offsets.
pub fn scroll_area(
    id: impl Into<SharedString>,
    max_h: Pixels,
    content: impl IntoElement,
) -> ScrollArea {
    ScrollArea {
        id: id.into(),
        max_h,
        handle: None,
        style: StyleRefinement::default(),
        content: content.into_any_element(),
    }
}

impl ScrollArea {
    /// Track an external handle instead of the internal keyed one — for
    /// callers (or tests) that read scroll offsets back.
    pub fn track(mut self, handle: &ScrollHandle) -> Self {
        self.handle = Some(handle.clone());
        self
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = match self.handle {
            Some(handle) => handle,
            None => window
                .use_keyed_state(
                    ElementId::Name(format!("{}-handle", self.id).into()),
                    cx,
                    |_, _| ScrollHandle::default(),
                )
                .read(cx)
                .clone(),
        };
        let id = self.id;
        let wrapper_selector = format!("{id}-wrapper");
        let layer_selector = format!("{id}-scrollbar-layer");
        div()
            .relative()
            .debug_selector(move || wrapper_selector)
            .max_h(self.max_h)
            .refine_style(&self.style)
            .child(
                div()
                    .id(ElementId::Name(id.clone()))
                    .max_h(self.max_h)
                    .pr(px(theme::SCROLL_AREA_GUTTER))
                    .overflow_y_scroll()
                    .track_scroll(&handle)
                    .child(self.content),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .debug_selector(move || layer_selector)
                    .child(
                        Scrollbar::vertical(&handle)
                            .id(ElementId::Name(format!("{id}-scrollbar").into())),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
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
    fn content_fits_viewport_draws_no_thumb() {
        assert_eq!(thumb_geometry(px(100.), px(100.), px(0.), px(0.)), None);
        assert_eq!(thumb_geometry(px(100.), px(80.), px(0.), px(0.)), None);
        // Bounds not yet measured (zero viewport).
        assert_eq!(thumb_geometry(px(0.), px(200.), px(0.), px(0.)), None);
    }

    #[test]
    fn tiny_viewport_where_min_thumb_exceeds_height_draws_no_thumb() {
        // viewport_h (20) < SCROLLBAR_MIN_THUMB_H (24): after clamping the
        // thumb would be taller than the track, producing a negative
        // track_h. No thumb is more useful than an inverted one.
        assert_eq!(thumb_geometry(px(20.), px(25.), px(0.), px(0.)), None);
    }

    #[test]
    fn thumb_sits_at_top_when_unscrolled() {
        // viewport 100 of 200 → half-height thumb at the track origin.
        assert_eq!(
            thumb_geometry(px(100.), px(200.), px(0.), px(0.)),
            Some((px(0.), px(50.)))
        );
    }

    #[test]
    fn thumb_sits_at_track_bottom_when_fully_scrolled() {
        // offset = -(content - viewport) = -100 → scroll_frac 1 → top = track_h.
        assert_eq!(
            thumb_geometry(px(100.), px(200.), px(-100.), px(0.)),
            Some((px(50.), px(50.)))
        );
    }

    #[test]
    fn top_offset_shifts_the_thumb_down() {
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
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let content = div().flex().flex_col().children(
                (0..self.rows).map(|i| div().child(SharedString::from(format!("row {i}")))),
            );
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
    fn thumb_height_is_clamped_to_the_minimum() {
        // viewport 100 of 10000 → 1px raw thumb, clamped up to the min.
        let (_, thumb_h) = thumb_geometry(px(100.), px(10000.), px(0.), px(0.)).unwrap();
        assert_eq!(thumb_h, px(theme::SCROLLBAR_MIN_THUMB_H));
    }

    #[test]
    fn over_scroll_clamps_to_the_track_bottom() {
        // offset beyond the scrollable range must not push the thumb past
        // the track; track_h = 100 - 24 = 76.
        let (thumb_top, _) = thumb_geometry(px(100.), px(10000.), px(-99999.), px(0.)).unwrap();
        assert_eq!(thumb_top, px(100.) - px(theme::SCROLLBAR_MIN_THUMB_H));
    }

    #[test]
    fn clamped_thumb_at_partial_scroll_with_top_offset() {
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
}
