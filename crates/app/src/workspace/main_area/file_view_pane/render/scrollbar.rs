//! File-viewer scrollbar — visual indicator only (no drag-to-scroll yet).
//!
//! Sized from the pre-computed `content_h` so the thumb size and position
//! stay accurate across both the fixed-line-height path (Raw / Changes)
//! and the variable-block path (Markdown Preview, derived from
//! `max_offset` instead of `total_rows * line_h`).

use crate::ui::theme;
use gpui::App;

/// Build an optional scrollbar thumb element based on the current scroll state.
/// Returns `None` when there is nothing to scroll (content fits in viewport).
///
/// `body_top` offsets the thumb within the `.relative()` file-viewer container.
/// `viewport_h` and `content_h` are supplied by the caller rather than read
/// off `scroll_handle.bounds()` here: for the Raw/Diff editor mode the
/// "scroll handle" is `gpui_component::input::InputState`'s own internal
/// one, whose `.bounds()` this vendored fork's text element never populates
/// (it never calls `div().track_scroll(&handle)` — see `InputState::scroll_size`'s
/// doc comment) and would silently draw no thumb. `content_h`:
/// - Raw/Changes: `total_rows * FILE_VIEWER_LINE_H` (stable across virtual-list shifts).
/// - Preview: `viewport_h + max_offset().y` (measured after layout; variable blocks).
/// - Raw/Diff editor: `InputState::scroll_size().height`.
pub(super) fn file_viewer_scrollbar(
    scroll_handle: &gpui::ScrollHandle,
    body_top: gpui::Pixels,
    viewport_h: gpui::Pixels,
    content_h: gpui::Pixels,
    cx: &App,
) -> Option<crate::ui::scrollbar::Thumb> {
    let t = theme::current(cx);
    // `body_top` offsets the thumb within the `.relative()` file-viewer
    // container; the thumb range is [body_top, body_top + viewport_h].
    crate::ui::scrollbar::vertical_thumb(
        "file-viewer-scrollbar",
        viewport_h,
        content_h,
        scroll_handle.offset().y,
        body_top,
        t.scrollbar_thumb,
        t.file_viewer_scrollbar_thumb_hover,
    )
}
