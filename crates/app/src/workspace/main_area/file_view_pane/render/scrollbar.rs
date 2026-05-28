//! File-viewer scrollbar — visual indicator only (no drag-to-scroll yet).
//!
//! Sized from the pre-computed `content_h` so the thumb size and position
//! stay accurate across both the fixed-line-height path (Raw / Changes)
//! and the variable-block path (Markdown Preview, derived from
//! `max_offset` instead of `total_rows * line_h`).

use crate::ui::theme;
use gpui::{AnyElement, App, IntoElement, div, prelude::*, px};

/// Build an optional scrollbar thumb element based on the current scroll state.
/// Returns `None` when there is nothing to scroll (content fits in viewport).
///
/// `body_top` offsets the thumb within the `.relative()` file-viewer container.
/// `content_h` is the pre-computed total content height:
/// - Raw/Changes: `total_rows * FILE_VIEWER_LINE_H` (stable across virtual-list shifts).
/// - Preview: `viewport_h + max_offset().height` (measured after layout; variable blocks).
pub(super) fn file_viewer_scrollbar(
    scroll_handle: &gpui::ScrollHandle,
    body_top: gpui::Pixels,
    content_h: gpui::Pixels,
    cx: &App,
) -> Option<AnyElement> {
    let viewport_h = scroll_handle.bounds().size.height;

    // Not scrollable or bounds not yet computed (first frame).
    if content_h <= viewport_h || viewport_h <= px(0.) {
        return None;
    }

    let thumb_ratio = (viewport_h / content_h).min(1.0_f32);
    let raw_thumb_h = viewport_h * thumb_ratio;
    let thumb_h = raw_thumb_h.max(px(theme::DOCK_SCROLLBAR_MIN_THUMB_H));
    let track_h = viewport_h - thumb_h;

    let scroll_frac = {
        let scrollable = content_h - viewport_h;
        ((-scroll_handle.offset().y) / scrollable).clamp(0.0_f32, 1.0_f32)
    };
    // viewport_h is the scroll container height (toolbar and hint bar already
    // excluded), so the thumb range is [body_top, body_top + viewport_h].
    let thumb_top = body_top + track_h * scroll_frac;

    let scrollbar_right = px(theme::DOCK_SCROLLBAR_MARGIN_R);
    let scrollbar_w = px(theme::DOCK_SCROLLBAR_W);

    let t = theme::current(cx);
    let thumb_bg = t.file_viewer_scrollbar_thumb;
    let thumb_hover_bg = t.file_viewer_scrollbar_thumb_hover;

    Some(
        div()
            .id("file-viewer-scrollbar")
            .absolute()
            .top(thumb_top)
            .right(scrollbar_right)
            .w(scrollbar_w)
            .h(thumb_h)
            .rounded(scrollbar_w / 2.0)
            .bg(thumb_bg)
            .hover(move |d| d.bg(thumb_hover_bg))
            .into_any_element(),
    )
}
