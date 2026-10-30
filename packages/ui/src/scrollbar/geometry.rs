//! Pure scrollbar geometry, split out so the math is testable without a window.

use gpui::{Pixels, px};

use crate::theme;

/// Pure thumb geometry: `(thumb_start, thumb_len)` in pixels along one axis,
/// or `None` when the content fits the viewport (so no thumb is drawn).
/// Axis-agnostic 1-D math — [`super::vertical_thumb`] feeds it height/Y values,
/// [`super::horizontal_thumb`] feeds it width/X values. Split out so the math is
/// testable without a window.
pub(super) fn thumb_geometry(
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

/// Pure: is a scroll region within `slack` of the bottom? `offset_y <= 0` (more
/// negative = scrolled further down) and `max_y >= 0` is the bottom extent, so
/// at the bottom `max_y + offset_y ≈ 0`. Content that fits (`max_y <= 0`) is
/// trivially at the bottom. Split out so it is testable without a laid-out list.
pub(super) fn scroll_at_bottom(offset_y: f32, max_y: f32, slack: f32) -> bool {
    max_y <= 0.0 || (max_y + offset_y) <= slack
}

/// The scroll offset (`-scrollable..=0`) that puts the thumb under `pointer`,
/// where `grab` is how far into the thumb the pointer holds it: the press point
/// for a drag, half the thumb for a track press (so the thumb centres on it).
///
/// Absolute rather than a delta from the previous move, so moves arriving
/// between two paints never compound — each maps the pointer on its own.
/// A track with no room for the thumb to travel carries no position, so it
/// returns `0` instead of dividing by it.
pub(super) fn pointer_scroll_offset(
    pointer: Pixels,
    track_start: Pixels,
    track_len: Pixels,
    thumb_len: Pixels,
    grab: Pixels,
    scrollable: Pixels,
) -> Pixels {
    let travel = track_len - thumb_len;
    if travel <= px(0.) || scrollable <= px(0.) {
        return px(0.);
    }
    let thumb_start = (pointer - track_start - grab).clamp(px(0.), travel);
    -(scrollable * (thumb_start / travel))
}
