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

/// Absolute (unified-frame) screen row of a 1-indexed live-grid row.
///
/// The live grid sits below `line_buffer` scrollback in the unified
/// frame, so grid row `1` maps to `total_rows - viewport_rows`. Pair
/// with [`screen_row_to_visible`] to place grid-anchored overlays
/// (cursor, IME preedit) correctly when the viewport is scrolled into
/// history — without this shift they paint at `grid_row - 1`, landing on
/// scrollback content rather than the live grid.
pub(crate) fn grid_row_to_screen_row(
    grid_row_1indexed: u16,
    total_rows: u32,
    viewport_rows: u32,
) -> u32 {
    total_rows
        .saturating_sub(viewport_rows)
        .saturating_add(u32::from(grid_row_1indexed).saturating_sub(1))
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

    #[test]
    fn grid_row_offsets_by_scrollback_height() {
        // 3-row grid with 5 rows of scrollback (total = 8): grid row 1 lands
        // at absolute row 5, grid row 3 at 7.
        assert_eq!(grid_row_to_screen_row(1, 8, 3), 5);
        assert_eq!(grid_row_to_screen_row(3, 8, 3), 7);
    }

    #[test]
    fn grid_row_with_no_scrollback_starts_at_zero() {
        // total_rows == viewport_rows: the grid is the whole frame.
        assert_eq!(grid_row_to_screen_row(1, 3, 3), 0);
        assert_eq!(grid_row_to_screen_row(3, 3, 3), 2);
    }

    #[test]
    fn grid_row_then_visible_is_identity_at_bottom() {
        // Pinned to bottom (viewport_top == total - rows): the grid->screen
        // ->viewport round trip yields grid_row - 1, matching the original
        // unscrolled cursor math.
        let (total, rows) = (20u32, 5u32);
        let viewport_top = total - rows; // scroll_offset == 0
        for grid_row in 1..=rows as u16 {
            let abs = grid_row_to_screen_row(grid_row, total, rows);
            assert_eq!(
                screen_row_to_visible(abs, viewport_top, rows),
                Some(u32::from(grid_row) - 1)
            );
        }
    }

    #[test]
    fn grid_cursor_scrolled_into_history_is_hidden() {
        // Scrolled up by 4 (viewport_top = total - rows - 4): grid row 1 maps
        // to viewport row 4, still visible; deeper grid rows fall off the
        // bottom and return None instead of painting over scrollback.
        let (total, rows) = (20u32, 5u32);
        let viewport_top = total - rows - 4;
        assert_eq!(
            screen_row_to_visible(grid_row_to_screen_row(1, total, rows), viewport_top, rows),
            Some(4)
        );
        assert_eq!(
            screen_row_to_visible(grid_row_to_screen_row(2, total, rows), viewport_top, rows),
            None
        );
    }
}
