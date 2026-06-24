//! Pane-area file viewer renderers.
//!
//! Entry point is `render_pane_file_viewer`; the sub-modules below
//! split the renderer by surface (toolbar / search panel / scrollbar)
//! and by content kind (raw / diff / markdown). All cross-module
//! helpers are `pub(super)` so visibility stays scoped to this module.
//!
//! ## Virtual list
//!
//! Both `render_raw_body` and `render_diff_body` use a virtual-list
//! strategy: only the rows that fall within the current viewport
//! (plus `FILE_VIEWER_VIRTUAL_OVERSCAN` rows above and below) are
//! emitted as GPUI elements. Top and bottom spacer divs carry the
//! height of the off-screen rows so the scroll container reports the
//! correct total height and the native scrollbar thumb stays accurate.

mod body;
mod content_element;
mod markdown;
mod scrollbar;
mod search_panel;
mod toolbar;
mod virtual_list;

use crate::ui::theme;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use self::body::render_file_viewer_body;
pub(in crate::workspace) use self::markdown::render_md_blocks_plain;
use self::scrollbar::file_viewer_scrollbar;
use self::search_panel::render_search_panel;
use self::toolbar::render_file_viewer_toolbar;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileContent, PaneFileView};

/// Top-level entry: toolbar (top) + scrollable body (middle) + hint bar (bottom).
/// Uses absolute positioning so the body receives a definite, bounded height from
/// Taffy — required for overflow_y_scroll to compute a non-zero scroll_max.
pub(in crate::workspace) fn render_pane_file_viewer(
    fv: &PaneFileView,
    editor_state: gpui::Entity<gpui_component::input::InputState>,
    scroll_handle: &gpui::ScrollHandle,
    search_input: gpui::Entity<crate::ui::InputState>,
    font_family: SharedString,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let toolbar_h = px(theme::FILE_VIEWER_HEADER_H);
    // Raw and diff both render through the shared editor.
    let is_editor_mode = matches!(
        &fv.content,
        PaneFileContent::LoadedRaw | PaneFileContent::LoadedDiff { .. }
    );

    // Preview mode renders variable-height blocks; derive content height from GPUI's
    // measured max_offset rather than total_rows * fixed_line_h to keep the thumb accurate.
    let is_preview_mode = matches!(&fv.content, PaneFileContent::LoadedMarkdown { .. })
        && fv.view_mode == FileViewMode::Preview;
    let viewer_bg = theme::current(cx).file_viewer_bg;

    // One scrollbar style across all modes: a thin daruda thumb. For
    // editor modes (raw / diff) the editor's built-in bar is suppressed
    // (`show_scrollbar(false)`) and the thumb is driven by the editor's
    // own scroll position; other modes use the pane's scroll handle.
    let scrollbar: Option<AnyElement> = if is_editor_mode {
        let editor_scroll = editor_state.read(cx).scroll_handle().clone();
        let viewport_h = editor_scroll.bounds().size.height;
        let content_h = viewport_h + editor_scroll.max_offset().y;
        file_viewer_scrollbar(&editor_scroll, toolbar_h, content_h, cx)
    } else {
        let content_h = if is_preview_mode {
            let viewport_h = scroll_handle.bounds().size.height;
            viewport_h + scroll_handle.max_offset().y
        } else {
            let total_rows = fv.visible_row_count();
            px(total_rows as f32 * theme::FILE_VIEWER_LINE_H)
        };
        file_viewer_scrollbar(scroll_handle, toolbar_h, content_h, cx)
    };
    // The editor provides its own find (Cmd+F); the custom search panel
    // only drives the non-editor renderers.
    let search_panel = if is_editor_mode {
        None
    } else {
        fv.search
            .as_ref()
            .map(|s| render_search_panel(s, toolbar_h, search_input, cx))
    };

    div()
        .relative()
        .size_full()
        .font_family(font_family)
        .bg(viewer_bg)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(toolbar_h)
                .child(render_file_viewer_toolbar(fv, cx)),
        )
        .child(render_file_viewer_body(
            fv,
            &editor_state,
            scroll_handle,
            toolbar_h,
            px(0.),
            cx,
        ))
        .children(scrollbar)
        .children(search_panel)
}
