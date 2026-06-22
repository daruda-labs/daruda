//! Scrollable body area — routes by content kind to the raw renderer,
//! the diff renderer, or the Markdown preview renderer. The body is
//! absolutely positioned between toolbar and hint bar so Taffy gives
//! it a definite height; without that, `overflow_y_scroll` would
//! compute `scroll_max == 0` and scrolling would silently break.

use crate::ui::theme;
use gpui::{AnyElement, Context, IntoElement, ScrollWheelEvent, div, prelude::*, px};

use super::content_element::FileViewerContentElement;
use super::markdown::render_md_body;
use super::virtual_list::virtual_range;
use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::{
    CharSelection, FileViewMode, PaneFileContent, PaneFileView, VisualRow,
};

/// Scrollable body area: routes to the appropriate renderer by content type and mode.
/// Absolutely positioned between `top_offset` and `bottom_offset` so Taffy gives it a
/// definite, bounded height — required for overflow_y_scroll to compute scroll_max > 0.
pub(super) fn render_file_viewer_body(
    fv: &PaneFileView,
    editor_state: &gpui::Entity<gpui_component::input::InputState>,
    scroll_handle: &gpui::ScrollHandle,
    top_offset: gpui::Pixels,
    bottom_offset: gpui::Pixels,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let char_selection = fv.selection_drag.char_selection().cloned();
    let hide_unchanged = fv.hide_unchanged;
    let search_state: Option<(&[usize], Option<usize>)> = fv.search.as_ref().map(|s| {
        let focused_row = s.focused.and_then(|fi| s.matches.get(fi).copied());
        (s.matches.as_slice(), focused_row)
    });

    let t = theme::current(cx);
    let ctx_text = t.text_muted;
    let del_text = t.file_diff_del_text;

    let frame = div()
        .absolute()
        .top(top_offset)
        .left_0()
        .right_0()
        .bottom(bottom_offset);

    match &fv.content {
        PaneFileContent::Loading => frame
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .text_color(ctx_text)
            .child(strings::file_viewer_loading())
            .into_any_element(),

        PaneFileContent::Error(msg) => frame
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .text_color(del_text)
            .child(msg.clone())
            .into_any_element(),

        PaneFileContent::Binary => frame
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .text_color(ctx_text)
            .child(strings::file_viewer_binary())
            .into_any_element(),

        PaneFileContent::Deleted => frame
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .text_color(del_text)
            .child(strings::file_viewer_deleted())
            .into_any_element(),

        PaneFileContent::LoadedRaw => frame
            .id("file-viewer-body")
            // The code editor stretches via `flex_grow` / `height: 100%`, which
            // only resolves inside a flex parent; without `.flex()` it collapses
            // to its 1-line `min_height`. (Matches the other branches above.)
            .flex()
            .child(crate::ui::file_viewer_editor(editor_state, cx))
            .into_any_element(),

        PaneFileContent::LoadedDiff {
            rows_all,
            rows_no_ctx,
            ..
        } => {
            let rows = if hide_unchanged {
                rows_no_ctx
            } else {
                rows_all
            };
            if rows.is_empty() {
                frame
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                    .text_color(ctx_text)
                    .child(strings::file_viewer_empty_diff())
                    .into_any_element()
            } else {
                // The diff renders through the same editor as raw — its
                // synthetic buffer + decorations + injected highlights were
                // installed at load time. `.flex()` lets the editor fill the
                // body (see the LoadedRaw branch).
                frame
                    .id("file-viewer-body")
                    .flex()
                    .child(crate::ui::file_viewer_editor(editor_state, cx))
                    .into_any_element()
            }
        }

        PaneFileContent::LoadedMarkdown {
            blocks: _,
            raw_rows,
            total_count,
            byte_truncated,
        } if fv.view_mode == FileViewMode::Raw => frame
            .id("file-viewer-body")
            .overflow_y_scroll()
            .track_scroll(scroll_handle)
            .child(render_raw_body(
                raw_rows,
                *total_count,
                *byte_truncated,
                char_selection.as_ref(),
                search_state,
                scroll_handle,
                cx,
            ))
            .into_any_element(),

        PaneFileContent::LoadedMarkdown { blocks, .. } => frame
            .id("file-viewer-body")
            .overflow_y_scroll()
            .track_scroll(scroll_handle)
            .child(render_md_body(blocks, char_selection.as_ref(), cx))
            .into_any_element(),
    }
}

/// Render plain raw file content with virtual-list scrolling.
fn render_raw_body(
    rows: &[VisualRow],
    total_count: usize,
    byte_truncated: bool,
    char_selection: Option<&CharSelection>,
    search: Option<(&[usize], Option<usize>)>,
    scroll_handle: &gpui::ScrollHandle,
    cx: &mut Context<Workspace>,
) -> gpui::Div {
    let t = theme::current(cx);
    let line_no_text = t.text_subtle;
    let body_text = t.text_body;
    let focused_bg = t.file_viewer_search_focused_bg;
    let match_bg = t.file_viewer_search_match_bg;
    // Config-driven editor font + matching row height, so the markdown raw
    // view shares one size with the raw/diff editor (`font.editor_size`).
    let editor_font = theme::editor_font_size(cx);
    let row_h = editor_font * theme::FILE_VIEWER_LINE_H_RATIO;
    let line_h = px(row_h);
    let overscan = theme::FILE_VIEWER_VIRTUAL_OVERSCAN;

    // offset().y is 0 at top, negative when scrolled down; negate for a positive value.
    let scroll_y = -scroll_handle.offset().y;
    let viewport_h = scroll_handle.bounds().size.height;

    let (start, end) = virtual_range(rows.len(), scroll_y, viewport_h, line_h, overscan);
    let top_h = px(start as f32 * row_h);
    let bottom_h = px((rows.len().saturating_sub(end)) as f32 * row_h);

    let workspace = cx.entity().clone();
    let scroll_handle_guard = scroll_handle.clone();
    let mut col = div()
        .flex()
        .flex_col()
        .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
            let delta_y = event.delta.pixel_delta(window.line_height()).y;
            if delta_y == px(0.) {
                return;
            }
            let current_y = scroll_handle_guard.offset().y;
            let max_y = scroll_handle_guard.max_offset().y;
            if max_y > px(0.) && (current_y + delta_y).clamp(-max_y, px(0.)) == current_y {
                cx.stop_propagation();
            }
        })
        .child(div().flex_none().h(top_h)); // top spacer

    for (offset, row) in rows[start..end].iter().enumerate() {
        let i = start + offset;
        let search_bg = search.and_then(|(m, f)| search_row_bg(i, m, f, focused_bg, match_bg));
        let el = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(line_h)
            .overflow_hidden()
            .text_size(px(editor_font))
            .when_some(search_bg, |d, bg| d.bg(bg))
            .child(
                div()
                    .flex_none()
                    .w(px(theme::FILE_VIEWER_LINE_NO_W))
                    .text_color(line_no_text)
                    .text_align(gpui::TextAlign::Right)
                    .pr(px(theme::FILE_VIEWER_LINE_NO_PAD_R))
                    .child(row.line_no_left.clone()),
            )
            .child(FileViewerContentElement::new(
                workspace.clone(),
                i,
                row,
                char_selection,
                body_text,
                line_h,
            ));
        col = col.child(el);
    }

    col = col.child(div().flex_none().h(bottom_h)); // bottom spacer

    // Footer: byte-truncation notice takes priority over line-cap notice.
    let shown = rows.len();
    if byte_truncated {
        col = col.child(footer_row(
            strings::file_viewer_byte_truncated(shown, theme::FILE_VIEWER_MAX_BYTES, total_count),
            line_no_text,
            editor_font,
            row_h,
        ));
    } else if total_count > shown {
        col = col.child(footer_row(
            strings::file_viewer_more_lines(total_count - shown),
            line_no_text,
            editor_font,
            row_h,
        ));
    }

    col
}

/// Return the search match background for a row, if any.
/// `matches` must be sorted; uses binary search for O(log n) lookup.
fn search_row_bg(
    row_idx: usize,
    matches: &[usize],
    focused_row: Option<usize>,
    focused_bg: gpui::Hsla,
    match_bg: gpui::Hsla,
) -> Option<gpui::Hsla> {
    if matches.binary_search(&row_idx).is_ok() {
        if focused_row == Some(row_idx) {
            Some(focused_bg)
        } else {
            Some(match_bg)
        }
    } else {
        None
    }
}

/// A non-interactive footer row for truncation notices. Sized from the
/// config-driven editor font (`font` / `row_h`) so it matches the
/// virtual-list rows above it instead of the fixed default.
fn footer_row(text: String, text_color: gpui::Hsla, font: f32, row_h: f32) -> gpui::Div {
    div()
        .flex_none()
        .px(px(theme::FILE_VIEWER_LINE_NO_W))
        .h(px(row_h))
        .flex()
        .items_center()
        .text_size(px(font))
        .text_color(text_color)
        .child(text)
}
