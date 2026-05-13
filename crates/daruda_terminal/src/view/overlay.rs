//! Viewport overlay helpers.
//!
//! Small visual primitives shared by every overlay that floats above
//! the terminal grid — search highlights, prompt-mark gutter, bell
//! flash, prompt-jump wrap flash. Keeping them separate avoids
//! re-duplicating viewport-row math and time-bounded quad checks
//! across each overlay's `build_*` method.

use gpui::PaintQuad;

/// Map an absolute screen row to its 0-based index within the
/// currently visible viewport, or `None` when the row lies outside.
/// Shared by search highlights and prompt gutter marks so the
/// "row ∈ [viewport_top, viewport_top + rows)" guard is not
/// duplicated (and doesn't drift) across overlays.
pub(crate) fn screen_row_to_visible(
    row: u32,
    viewport_top: u32,
    viewport_rows: u32,
) -> Option<u32> {
    // `then_some(row - viewport_top)` would evaluate the subtraction
    // even when the predicate is false — underflows in debug builds
    // when `row < viewport_top` (a search/prompt mark sitting in
    // scrollback above or below the visible region). `then(|| …)`
    // is lazy. Also guard the upper bound with `checked_add` so
    // `viewport_top + rows` cannot wrap.
    let viewport_bottom = viewport_top.checked_add(viewport_rows)?;
    (row >= viewport_top && row < viewport_bottom).then(|| row - viewport_top)
}

/// Build a time-limited overlay quad only while `deadline` is in the
/// future. Centralises the bell / prompt-jump flash pattern so the
/// `Instant::now() < until` check isn't open-coded at every call site.
pub(crate) fn flash_overlay_if_active<F>(
    deadline: Option<std::time::Instant>,
    build: F,
) -> Option<PaintQuad>
where
    F: FnOnce() -> PaintQuad,
{
    deadline
        .filter(|&until| std::time::Instant::now() < until)
        .map(|_| build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_row_to_visible_does_not_underflow_when_row_is_above_viewport() {
        // Rows in scrollback (row < viewport_top) must return None without
        // evaluating `row - viewport_top`, which would panic on debug builds.
        assert_eq!(screen_row_to_visible(5, 10, 24), None);
        assert_eq!(screen_row_to_visible(0, 100, 24), None);
    }

    #[test]
    fn screen_row_to_visible_inside_viewport_returns_offset() {
        assert_eq!(screen_row_to_visible(12, 10, 24), Some(2));
        assert_eq!(screen_row_to_visible(10, 10, 24), Some(0));
    }

    #[test]
    fn screen_row_to_visible_returns_none_at_or_past_bottom() {
        assert_eq!(screen_row_to_visible(34, 10, 24), None);
    }

    #[test]
    fn screen_row_to_visible_handles_overflow_in_bottom_calculation() {
        assert_eq!(screen_row_to_visible(0, u32::MAX, 1), None);
    }
}
