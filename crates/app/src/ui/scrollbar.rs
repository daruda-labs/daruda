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
//! [`horizontal_thumb`] is the X-axis mirror, for a region that only scrolls
//! horizontally (the agent-chat diff embed's long, non-wrapped lines) —
//! same [`thumb_geometry`] math, transposed onto the width/left/bottom axis.

use gpui::{AnyElement, ElementId, Hsla, ListState, Pixels, div, prelude::*, px};

use crate::ui::theme;

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

/// [`vertical_thumb`]'s horizontal mirror — for a region that only scrolls on
/// the X axis (the agent-chat diff embed's long, non-wrapped lines, which
/// keep the built-in scrollbar's drag interaction traded away in favour of
/// matching every other pane's display-only daruda thumb). `scroll_offset_x`
/// is the handle's `offset().x` (negative as content scrolls right). Unlike
/// [`vertical_thumb`] there is no `top_offset` — the diff embed reserves its
/// own bottom strip (see `AGENT_CHAT_DIFF_ROW_H` + `SCROLLBAR_W` at the call
/// site) so the thumb sits flush at the bottom of its container.
pub fn horizontal_thumb(
    id: impl Into<ElementId>,
    viewport_w: Pixels,
    content_w: Pixels,
    scroll_offset_x: Pixels,
    thumb_bg: Hsla,
    thumb_hover_bg: Hsla,
) -> Option<AnyElement> {
    let (thumb_left, thumb_w) = thumb_geometry(viewport_w, content_w, scroll_offset_x, px(0.))?;
    let h = px(theme::SCROLLBAR_W);
    Some(
        div()
            .id(id)
            .absolute()
            .left(thumb_left)
            .bottom(px(theme::SCROLLBAR_MARGIN_R))
            .h(h)
            .w(thumb_w)
            .rounded(h / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
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
) -> Option<AnyElement> {
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
