//! Diff row builders — hunk-header (non-selectable) and selectable
//! row (Context / Added / Removed / NoNewline). The selectable row
//! defers content rendering to `FileViewerContentElement` so character
//! selection and word-diff highlights work uniformly across body modes.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{App, div, prelude::*, px};

use super::content_element::FileViewerContentElement;
use crate::workspace::Workspace;
use crate::workspace::pane_file_view::{CharSelection, VisualRow, VisualRowKind};

/// Render a diff hunk-header row (the `@@ -N +M @@` line). Not selectable.
pub(super) fn diff_visual_row(
    row: &VisualRow,
    _is_sel: bool,
    search_bg: Option<gpui::Hsla>,
    line_h: gpui::Pixels,
    cx: &App,
) -> gpui::Div {
    let t = theme::current(cx);
    let hunk_border = t.file_diff_hunk_border;
    let hunk_ctx_text = t.file_diff_hunk_ctx_text;
    let (default_bg, text_color) = row_style(row.kind, t);
    let bg = search_bg.or(default_bg);

    div()
        .flex_none()
        .flex()
        .flex_row()
        .h(line_h + px(theme::FILE_DIFF_HUNK_PADDING_Y * 2.0))
        .py(px(theme::FILE_DIFF_HUNK_PADDING_Y))
        .overflow_hidden()
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .when_some(bg, |d, b| d.bg(b))
        .border_t_1()
        .border_b_1()
        .border_color(hunk_border)
        .child(
            div()
                .flex_none()
                .w(px(theme::FILE_VIEWER_LINE_NO_W * 2.0))
                .text_color(text_color),
        )
        .child(
            // @@ -N +M @@ portion in hunk color + optional trailing context in dim color.
            div()
                .flex()
                .flex_row()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .flex_none()
                        .text_color(text_color)
                        .child(row.content.clone()),
                )
                .when(!row.header_context.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .pl(px(theme::FILE_DIFF_HUNK_CTX_GAP_X))
                            .text_color(hunk_ctx_text)
                            .child(row.header_context.clone()),
                    )
                }),
        )
}

/// Render a selectable diff row (Context, Added, Removed, NoNewline).
/// Uses `FileViewerContentElement` for character-level selection in the content cell.
pub(super) fn diff_selectable_row(
    row: &VisualRow,
    row_idx: usize,
    char_selection: Option<&CharSelection>,
    search_bg: Option<gpui::Hsla>,
    line_h: gpui::Pixels,
    workspace: gpui::Entity<Workspace>,
    cx: &App,
) -> gpui::Div {
    let t = theme::current(cx);
    let line_no_text = t.file_viewer_line_no_text;
    let (row_bg, text_color) = row_style(row.kind, t);
    let bg = search_bg.or(row_bg);

    if row.kind == VisualRowKind::NoNewline {
        return div()
            .flex_none()
            .h(line_h)
            .overflow_hidden()
            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
            .text_color(text_color)
            .when_some(bg, |d, b| d.bg(b))
            .child(row.content.clone());
    }

    div()
        .flex_none()
        .flex()
        .flex_row()
        .h(line_h)
        .overflow_hidden()
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .when_some(bg, |d, b| d.bg(b))
        .child(
            div()
                .flex_none()
                .w(px(theme::FILE_VIEWER_LINE_NO_W))
                .text_color(line_no_text)
                .text_align(gpui::TextAlign::Right)
                .pr(px(theme::FILE_VIEWER_DIFF_LINE_NO_PAD_R))
                .child(row.line_no_left.clone()),
        )
        .child(
            div()
                .flex_none()
                .w(px(theme::FILE_VIEWER_LINE_NO_W))
                .text_color(line_no_text)
                .text_align(gpui::TextAlign::Right)
                .pr(px(theme::FILE_VIEWER_DIFF_LINE_NO_PAD_R))
                .child(row.line_no_right.clone()),
        )
        .child(FileViewerContentElement::new(
            workspace,
            row_idx,
            row,
            char_selection,
            text_color,
            line_h,
        ))
}

/// Return `(default_background, text_color)` for a row kind.
/// Background color alone distinguishes Added/Removed rows (no marker column).
fn row_style(kind: VisualRowKind, t: &DarudaTheme) -> (Option<gpui::Hsla>, gpui::Hsla) {
    match kind {
        VisualRowKind::HunkHeader => (Some(t.file_diff_hunk_bg), t.file_diff_hunk_text),
        VisualRowKind::Added => (Some(t.file_diff_add_bg), t.file_diff_add_text),
        VisualRowKind::Removed => (Some(t.file_diff_del_bg), t.file_diff_del_text),
        VisualRowKind::Context => (None, t.file_diff_ctx_text),
        VisualRowKind::NoNewline => (None, t.file_diff_ctx_text),
        VisualRowKind::Plain => (None, t.file_viewer_text),
    }
}
