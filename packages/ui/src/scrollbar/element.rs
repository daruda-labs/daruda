//! The thumb's paint and pointer handling, filling the press strip [`Thumb`]
//! lays out.
//!
//! An element rather than a div because a track press and a drag both map the
//! pointer through the strip's window bounds, which only prepaint knows.
//!
//! [`Thumb`]: super::Thumb

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, Bounds, CursorStyle, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseExitEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Style, Window, fill, point, px, relative, size,
};

use super::geometry::pointer_scroll_offset;
use super::{Axis, ThumbTarget, record_press};
use crate::theme;

/// Where the pointer stands with this thumb, kept across frames in element
/// state keyed by the thumb's id, so two thumbs in one window never share it.
#[derive(Default)]
enum ThumbPointer {
    #[default]
    Idle,
    Hovered,
    Dragging(DragSession),
}

impl ThumbPointer {
    fn grab(&self) -> Option<Pixels> {
        match self {
            ThumbPointer::Dragging(session) => Some(session.grab),
            ThumbPointer::Idle | ThumbPointer::Hovered => None,
        }
    }

    /// Hover or drag: either way the thumb paints in its hover colour.
    fn is_lit(&self) -> bool {
        !matches!(self, ThumbPointer::Idle)
    }

    /// The resting state for a pointer at rest, `over` the thumb or not.
    fn resting(over: bool) -> Self {
        if over {
            ThumbPointer::Hovered
        } else {
            ThumbPointer::Idle
        }
    }
}

/// A drag in progress. Ending it is tied to dropping it, so every way a drag
/// can stop — a release, a release outside the window, or the thumb no longer
/// painting and taking its state and listeners with it — tells the target once.
struct DragSession {
    /// How far into the thumb, along the axis, the press landed; the drag
    /// keeps that point under the pointer.
    grab: Pixels,
    target: Rc<dyn ThumbTarget>,
}

impl Drop for DragSession {
    fn drop(&mut self) {
        self.target.drag_ended();
    }
}

type PointerState = Rc<RefCell<ThumbPointer>>;

pub(super) struct ThumbElement {
    pub(super) id: ElementId,
    pub(super) axis: Axis,
    pub(super) target: Rc<dyn ThumbTarget>,
    /// Thumb offset from the track start.
    pub(super) start: Pixels,
    pub(super) len: Pixels,
    pub(super) scrollable: Pixels,
    pub(super) bg: Hsla,
    pub(super) hover_bg: Hsla,
}

pub(super) struct ThumbPrepaint {
    /// The whole strip. `Normal`, so a right press or hover still reaches
    /// what lies under it; a left press is kept by stopping its propagation.
    strip: Hitbox,
    /// The thumb's band across the strip's full width, for its cursor.
    band: Hitbox,
    pointer: PointerState,
}

impl ThumbElement {
    /// The painted thumb inside `track`: `SCROLLBAR_W` thick, inset from the
    /// strip's outer edge by `SCROLLBAR_MARGIN_R`.
    fn thumb_bounds(&self, track: Bounds<Pixels>) -> Bounds<Pixels> {
        let thickness = px(theme::SCROLLBAR_W);
        let inset = px(theme::SCROLLBAR_MARGIN_R);
        match self.axis {
            Axis::Vertical => Bounds::new(
                point(track.right() - inset - thickness, track.top() + self.start),
                size(thickness, self.len),
            ),
            Axis::Horizontal => Bounds::new(
                point(
                    track.left() + self.start,
                    track.bottom() - inset - thickness,
                ),
                size(self.len, thickness),
            ),
        }
    }

    /// The thumb's extent along the axis, across the strip's full width: the
    /// grab target, wider than the painted thumb so it is easy to hit.
    fn band_bounds(&self, track: Bounds<Pixels>) -> Bounds<Pixels> {
        match self.axis {
            Axis::Vertical => Bounds::new(
                point(track.left(), track.top() + self.start),
                size(track.size.width, self.len),
            ),
            Axis::Horizontal => Bounds::new(
                point(track.left() + self.start, track.top()),
                size(self.len, track.size.height),
            ),
        }
    }
}

impl IntoElement for ThumbElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ThumbElement {
    type RequestLayoutState = ();
    type PrepaintState = ThumbPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let strip = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let band = window.insert_hitbox(self.band_bounds(bounds), HitboxBehavior::Normal);
        let pointer = match id {
            Some(id) => window.with_element_state(id, |state, _| {
                let state: PointerState = state.unwrap_or_default();
                (state.clone(), state)
            }),
            None => Rc::default(),
        };
        ThumbPrepaint {
            strip,
            band,
            pointer,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        let thumb = self.thumb_bounds(bounds);
        let band = self.band_bounds(bounds);
        let pointer = prepaint.pointer.clone();
        let hitbox = prepaint.strip.clone();
        let (lit, dragging) = {
            let state = pointer.borrow();
            (state.is_lit(), state.grab().is_some())
        };
        let color = if lit { self.hover_bg } else { self.bg };
        window.paint_quad(fill(thumb, color).corner_radii(px(theme::SCROLLBAR_W / 2.)));
        // The later request wins where both are hovered: a hand on the thumb,
        // an arrow over the rest of the track rather than the content's cursor.
        if dragging {
            window.set_window_cursor_style(CursorStyle::PointingHand);
        } else {
            window.set_cursor_style(CursorStyle::Arrow, &hitbox);
            window.set_cursor_style(CursorStyle::PointingHand, &prepaint.band);
        }

        let view_id = window.current_view();
        let axis = self.axis;
        let track_start = axis.along(bounds.origin);
        let track_len = match axis {
            Axis::Vertical => bounds.size.height,
            Axis::Horizontal => bounds.size.width,
        };
        let (len, scrollable) = (self.len, self.scrollable);
        let scroll = {
            let target = self.target.clone();
            move |pointer_at: Pixels, grab: Pixels, cx: &mut App| {
                let offset = pointer_scroll_offset(
                    pointer_at,
                    track_start,
                    track_len,
                    len,
                    grab,
                    scrollable,
                );
                target.scroll_to(axis, offset, cx);
                cx.notify(view_id);
            }
        };

        window.on_mouse_event({
            let (pointer, hitbox, target, scroll) = (
                pointer.clone(),
                hitbox.clone(),
                self.target.clone(),
                scroll.clone(),
            );
            move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble()
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                if band.contains(&event.position) {
                    let grab = axis.along(event.position) - axis.along(band.origin);
                    target.drag_started();
                    let session = DragSession {
                        grab,
                        target: target.clone(),
                    };
                    *pointer.borrow_mut() = ThumbPointer::Dragging(session);
                    cx.notify(view_id);
                } else {
                    scroll(axis.along(event.position), len / 2., cx);
                }
                record_press(event.position, cx);
                cx.stop_propagation();
            }
        });

        // Settle the pointer at rest; repaint only when the colour changes.
        let rest = {
            let pointer = pointer.clone();
            move |over: bool, cx: &mut App| {
                let next = ThumbPointer::resting(over);
                let mut state = pointer.borrow_mut();
                if state.is_lit() != next.is_lit() {
                    cx.notify(view_id);
                }
                *state = next;
            }
        };

        // Capture phase, so a drag keeps the pointer even when it strays over
        // content that would otherwise take the move.
        window.on_mouse_event({
            let (pointer, hitbox, rest) = (pointer.clone(), hitbox.clone(), rest.clone());
            move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.capture() {
                    return;
                }
                let grab = pointer.borrow().grab();
                if let Some(grab) = grab {
                    if event.dragging() {
                        scroll(axis.along(event.position), grab, cx);
                        cx.stop_propagation();
                        return;
                    }
                    // The button came up outside the window, so no mouse-up
                    // arrived: the drag is over. Repaint even if the colour
                    // stays, to drop the window-wide cursor.
                    *pointer.borrow_mut() = ThumbPointer::Idle;
                    cx.notify(view_id);
                }
                rest(
                    hitbox.is_hovered(window) && band.contains(&event.position),
                    cx,
                );
            }
        });

        window.on_mouse_event({
            let (pointer, hitbox, rest) = (pointer.clone(), hitbox.clone(), rest.clone());
            // ANY-BUTTON: a drag starts only from a left press; any release ends it.
            move |event: &MouseUpEvent, phase, window, cx| {
                if !phase.capture() || pointer.borrow().grab().is_none() {
                    return;
                }
                // The window-wide drag cursor goes even if the thumb stays lit.
                cx.notify(view_id);
                rest(
                    hitbox.is_hovered(window) && band.contains(&event.position),
                    cx,
                );
            }
        });

        // Leaving the window sends no move, so hover would otherwise linger
        // on a thumb at the window's edge. A drag outlives the exit.
        window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
            if phase.capture() && pointer.borrow().grab().is_none() {
                rest(false, cx);
            }
        });
    }
}
