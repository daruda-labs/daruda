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

use gpui::{AnyElement, ElementId, Hsla, Pixels, div, prelude::*, px};

use crate::ui::theme;

/// Build the thumb overlay for a vertically-scrolling region, or `None`
/// when the content fits the viewport (or bounds are not yet measured).
///
/// `content_h` is the total scrollable content height: handles backed by
/// a `ScrollHandle` pass `viewport_h + max_offset().height`, while the
/// file viewer passes a precomputed height. `scroll_offset_y` is the
/// handle's `offset().y` (negative as the content scrolls up).
/// `top_offset` shifts the thumb down when the scroll region begins below
/// the positioned container's origin (the file viewer); pass `px(0.)`
/// otherwise. The thumb is positioned with `.right(SCROLLBAR_MARGIN_R)`,
/// so the caller's parent must be `.relative()`.
pub fn vertical_thumb(
    id: impl Into<ElementId>,
    viewport_h: Pixels,
    content_h: Pixels,
    scroll_offset_y: Pixels,
    top_offset: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<AnyElement> {
    let (thumb_top, thumb_h) = thumb_geometry(viewport_h, content_h, scroll_offset_y, top_offset)?;
    let w = px(theme::SCROLLBAR_W);
    Some(
        div()
            .id(id)
            .absolute()
            .top(thumb_top)
            .right(px(theme::SCROLLBAR_MARGIN_R))
            .w(w)
            .h(thumb_h)
            .rounded(w / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
}

/// Pure thumb geometry: `(thumb_top, thumb_h)` in pixels, or `None` when
/// the content fits the viewport (so no thumb is drawn). Split out from
/// [`vertical_thumb`] so the math is testable without a window.
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

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
