//! Scrollbar thumb overlay — shared geometry, chrome, and pointer handling.
//!
//! The docks, the file viewer, the task editor, and the agent chat each
//! overlay a thin thumb derived from a viewport/content extent pair and the
//! current scroll offset. The geometry, the chrome, and the pointer behaviour
//! (a track press jumps there, the thumb drags) are identical; only the element
//! id, an optional top offset (a scroll region that starts below a header), the
//! theme colours, and the handle being scrolled differ — so those are the
//! parameters. Callers pass plain pixels plus a [`ThumbTarget`].
//!
//! [`horizontal_thumb`] is the X-axis mirror, for a region whose content
//! overflows sideways — same `thumb_geometry` math, transposed onto the
//! width/left/bottom axis.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Global, Hsla, IntoElement, ListState, Pixels, Point, RenderOnce,
    ScrollHandle, SharedString, StyleRefinement, Window, div, prelude::*, px,
};
use gpui_component::StyledExt as _;

use crate::theme;
use element::ThumbElement;
use geometry::{scroll_at_bottom, thumb_geometry};

/// The gpui_component built-in scrollbar (draggable track + thumb), for
/// regions that want its own chrome and gutter — e.g. the status-bar Ports
/// dropdown. Re-exported here so call sites stay on `crate::ui::*` (shape C
/// wrapper).
pub use gpui_component::scroll::{Scrollbar, ScrollbarShow};

/// Build the thumb overlay for a vertically-scrolling region, or `None`
/// when the content fits the viewport (or bounds are not yet measured).
///
/// `target` is what a track press or a thumb drag scrolls. `content_h` is the
/// total scrollable content height: handles backed by a `ScrollHandle` pass
/// `viewport_h + max_offset().y`, while the file viewer passes a precomputed
/// height. `scroll_offset_y` is the handle's `offset().y` (negative as the
/// content scrolls up). `top_offset` shifts the track down when the scroll
/// region begins below the positioned container's origin; pass `px(0.)`
/// otherwise. The track is pinned to the right edge, so the caller's parent
/// must be `.relative()`.
///
/// Debug selector (test hook): the `id` string, on the press strip, so a test
/// can assert whether a thumb was drawn at all.
#[allow(clippy::too_many_arguments)]
pub fn vertical_thumb(
    id: impl Into<ElementId>,
    target: impl ThumbTarget,
    viewport_h: Pixels,
    content_h: Pixels,
    scroll_offset_y: Pixels,
    top_offset: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<Thumb> {
    let (thumb_top, thumb_h) = thumb_geometry(viewport_h, content_h, scroll_offset_y, px(0.))?;
    Some(Thumb {
        id: id.into(),
        axis: Axis::Vertical,
        target: Rc::new(target),
        top_offset,
        start: thumb_top,
        len: thumb_h,
        track: viewport_h,
        scrollable: content_h - viewport_h,
        bg: thumb_bg,
        hover_bg: thumb_hover_bg,
    })
}

/// [`vertical_thumb`]'s horizontal mirror — for content that overflows on the X
/// axis. `scroll_offset_x` is the handle's `offset().x` (negative as content
/// scrolls right). Unlike [`vertical_thumb`] there is no `top_offset`: the thumb
/// sits flush at the bottom of its container.
///
/// Debug selector (test hook): the `id` string, as in [`vertical_thumb`].
pub fn horizontal_thumb(
    id: impl Into<ElementId>,
    target: impl ThumbTarget,
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
        target: Rc::new(target),
        top_offset: px(0.),
        start: thumb_left,
        len: thumb_w,
        track: viewport_w,
        scrollable: content_w - viewport_w,
        bg: thumb_bg,
        hover_bg: thumb_hover_bg,
    })
}

/// Which axis a [`Thumb`] rides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    /// `point` with this axis' component set to `value`.
    fn replace(self, point: Point<Pixels>, value: Pixels) -> Point<Pixels> {
        match self {
            Axis::Vertical => gpui::point(point.x, value),
            Axis::Horizontal => gpui::point(value, point.y),
        }
    }

    /// This axis' component of `point`.
    fn along(self, point: Point<Pixels>) -> Pixels {
        match self {
            Axis::Vertical => point.y,
            Axis::Horizontal => point.x,
        }
    }
}

/// A scrollbar thumb overlay over a press strip along the region's edge.
///
/// Absolutely positioned against the caller's `.relative()` parent.
#[derive(IntoElement)]
pub struct Thumb {
    id: ElementId,
    axis: Axis,
    target: Rc<dyn ThumbTarget>,
    /// Where the track starts below the parent's origin (vertical only).
    top_offset: Pixels,
    /// Thumb offset from the track start.
    start: Pixels,
    len: Pixels,
    /// Viewport extent along the axis — the track the thumb rides.
    track: Pixels,
    /// `content - viewport`: how far the region can actually scroll.
    scrollable: Pixels,
    bg: Hsla,
    hover_bg: Hsla,
}

impl RenderOnce for Thumb {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // The strip div owns placement and the test hook; the element inside
        // owns the pointer, which needs the strip's window bounds a div cannot
        // hand to its listeners.
        let strip = px(theme::SCROLLBAR_HIT_W);
        let selector = self.id.to_string();
        let track = div().absolute().debug_selector(move || selector);
        let track = match self.axis {
            Axis::Vertical => track.top(self.top_offset).right_0().w(strip).h(self.track),
            Axis::Horizontal => track.left_0().bottom_0().h(strip).w(self.track),
        };
        track.child(ThumbElement {
            id: self.id,
            axis: self.axis,
            target: self.target,
            start: self.start,
            len: self.len,
            scrollable: self.scrollable,
            bg: self.bg,
            hover_bg: self.hover_bg,
        })
    }
}

/// [`vertical_thumb`] for a virtualized [`gpui::list`] / [`ListState`], which it
/// also scrolls. Derives the viewport / content / offset geometry from the list
/// state's scrollbar API (the method names — `viewport_bounds`,
/// `max_offset_for_scrollbar`, `scroll_px_offset_for_scrollbar` — are
/// non-obvious) so every gpui-`list` pane gets the same daruda thumb without
/// re-deriving it. The geometry reflects the previous frame's layout, which is
/// fine for a thumb; read it at *render* time. `None` until the first layout or
/// when the content fits.
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
        list_state.clone(),
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
    fill: bool,
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
        fill: false,
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

    /// Take whatever height the flex column above leaves, instead of capping
    /// at `max_h` — for a body between pinned bands (a popover panel's header
    /// and footer). No gutter: the caller's own inset must clear the thumb.
    pub fn fill(mut self) -> Self {
        self.fill = true;
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
        let fill = self.fill;
        div()
            .relative()
            .debug_selector(move || wrapper_selector)
            .map(|wrapper| {
                if fill {
                    wrapper.flex_1().min_h(px(0.)).flex().flex_col()
                } else {
                    wrapper.max_h(self.max_h)
                }
            })
            .refine_style(&self.style)
            .child(
                div()
                    .id(ElementId::Name(id.clone()))
                    .map(|body| {
                        if fill {
                            body.flex_1().min_h(px(0.))
                        } else {
                            body.max_h(self.max_h).pr(px(theme::SCROLL_AREA_GUTTER))
                        }
                    })
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

/// The position of the last left press a scrollbar strip took. A strip stops
/// that press's propagation so content under it never reacts, which also hides
/// it from a host wrapper that reacts to any press inside it (a pane's
/// activation). The host clears this before dispatch and asks after it.
#[derive(Default)]
struct StripPress(Option<Point<Pixels>>);

impl Global for StripPress {}

fn record_press(at: Point<Pixels>, cx: &mut App) {
    cx.set_global(StripPress(Some(at)));
}

/// Drop any earlier strip press, so [`take_press`] after this press's dispatch
/// answers for this press alone. Call from a capture-phase listener.
pub fn forget_press(cx: &mut App) {
    cx.set_global(StripPress(None));
}

/// Whether a scrollbar strip took the left press at `at`, consuming the record.
pub fn take_press(at: Point<Pixels>, cx: &mut App) -> bool {
    cx.default_global::<StripPress>().0.take() == Some(at)
}

mod element;
mod geometry;
mod target;

pub use target::ThumbTarget;
#[cfg(test)]
mod tests;
