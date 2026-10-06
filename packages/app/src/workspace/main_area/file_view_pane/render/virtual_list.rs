//! Virtual-list math — given total rows, scroll position, viewport
//! height, and per-row height, return the `[start, end)` row window
//! to render plus the off-screen overscan.
//!
//! Both `render_raw_body` and `render_diff_body` use this so only the
//! rows that fall within the current viewport (plus
//! `FILE_VIEWER_VIRTUAL_OVERSCAN` rows above and below) are emitted as
//! GPUI elements. Top and bottom spacer divs carry the height of the
//! off-screen rows so the scroll container reports the correct total
//! height and the native scrollbar thumb stays accurate.

use gpui::px;

/// Compute the [start, end) row range to render.
///
/// Uses `Pixels / Pixels = f32` (GPUI arithmetic) so the caller never needs
/// to extract the inner `f32` from a `Pixels` value directly.
///
/// `scroll_y`   — how many pixels the user has scrolled (non-negative).
/// `viewport_h` — visible body height in pixels.
/// `line_h`     — fixed row height in pixels.
///
/// Returns `(0, fallback_end)` on the first frame before GPUI measures bounds.
pub(super) fn virtual_range(
    total: usize,
    scroll_y: gpui::Pixels,
    viewport_h: gpui::Pixels,
    line_h: gpui::Pixels,
    overscan: usize,
) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    if viewport_h <= px(0.) || line_h <= px(0.) {
        // First frame: bounds not yet computed; render an initial window.
        let end = (50 + overscan).min(total);
        return (0, end);
    }
    // Pixels / Pixels → f32
    let start_f: f32 = (scroll_y / line_h).max(0.0_f32);
    let end_f: f32 = ((scroll_y + viewport_h) / line_h).ceil();
    let raw_end = end_f as usize + overscan;
    let end = raw_end.min(total);
    // Clamp start to end so stale scroll positions (e.g. after a file reload
    // that reduced row count) never produce an invalid start..end range.
    let start = (start_f as usize).saturating_sub(overscan).min(end);
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::virtual_range;
    use gpui::px;

    #[test]
    fn virtual_range_cases() {
        for (total, scroll_y, viewport_h, line_h, overscan, expected) in [
            (0, 0., 600., 20., 8, (0, 0)),
            // viewport_h = 0 -> first-frame fallback: render first 50 + overscan rows.
            (100, 0., 0., 20., 8, (0, 58)),
            // Fewer rows than the fallback window: end is clamped to total.
            (10, 0., 0., 20., 8, (0, 10)),
            // Top, middle, near-end, and exact-fit scroll positions.
            (100, 0., 600., 20., 8, (0, 38)),
            (100, 200., 600., 20., 8, (2, 48)),
            (100, 1600., 600., 20., 8, (72, 100)),
            (5, 0., 600., 20., 8, (0, 5)),
        ] {
            assert_eq!(
                virtual_range(total, px(scroll_y), px(viewport_h), px(line_h), overscan),
                expected
            );
        }

        // File was reloaded and shrank to 10 rows while scroll_y still reflects the
        // old position (row ~80).  start must be clamped to end so rows[start..end]
        // is always valid and never panics.
        let (start, end) = virtual_range(10, px(1600.), px(600.), px(20.), 8);
        assert!(start <= end, "start ({start}) must not exceed end ({end})");
        assert!(end <= 10, "end ({end}) must not exceed total");
    }
}
