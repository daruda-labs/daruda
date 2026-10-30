//! Pane-area file viewer renderers.
//!
//! `render_pane_file_viewer` composes toolbar, body, search panel, and
//! scrollbar. Raw/diff bodies virtualize rows with spacer divs so scroll height
//! and the native thumb stay accurate.

mod body;
mod content_element;
mod markdown;
mod scrollbar;
mod search_panel;
mod toolbar;
mod virtual_list;

/// Cacheable GPU-ready diagram image re-exported for agent-chat mermaid.
use crate::ui::theme;
use gpui::{Context, IntoElement, div, prelude::*, px, relative};

use self::body::render_file_viewer_body;
use self::scrollbar::file_viewer_scrollbar;
use self::search_panel::render_search_panel;
use self::toolbar::render_file_viewer_toolbar;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::PaneFileContent;
use crate::workspace::main_area::pane::FileContent;
use crate::workspace::main_area::pane_tree::PaneId;

/// Top-level file-viewer element.
/// Absolute positioning gives the body a bounded height for scrolling.
///
pub(in crate::workspace) fn render_pane_file_viewer(
    pane_id: PaneId,
    fc: &FileContent,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let FileContent {
        view: fv,
        images,
        editor_state,
        scroll_handle,
        search_input,
        ..
    } = fc;
    let editor_state = editor_state.clone();
    let search_input = search_input.clone();
    let toolbar_h = px(theme::FILE_VIEWER_HEADER_H);
    // Raw and diff both render through the shared editor.
    let is_editor_mode = matches!(
        &fv.content,
        PaneFileContent::LoadedRaw { .. } | PaneFileContent::LoadedDiff { .. }
    );

    let surface = theme::PaneSurfaceTokens::file_viewer(cx);
    let viewer_bg = surface.background;

    // One thin daruda thumb; editor modes use the editor scroll handle.
    let scrollbar: Option<crate::ui::scrollbar::Thumb> = if is_editor_mode {
        let editor = editor_state.read(cx);
        // `editor_scroll.bounds()`/`.max_offset()` never populate for this
        // vendored editor (see `InputState::scroll_size` doc comment) — the
        // real viewport/content geometry lives in `last_bounds()` /
        // `scroll_size()` instead.
        let offset_y = editor.scroll_handle().offset().y;
        let viewport_h = editor.last_bounds().map_or(px(0.), |b| b.size.height);
        let content_h = editor.scroll_size().height;
        let editor = editor_state.clone();
        file_viewer_scrollbar(editor, offset_y, toolbar_h, viewport_h, content_h, cx)
    } else {
        // Measured, not `rows * line height`: that estimate omits the body's
        // padding, so a press at the track's end stopped short of the last row.
        // The virtual list's spacers keep the measured extent stable.
        let viewport_h = scroll_handle.bounds().size.height;
        let content_h = viewport_h + scroll_handle.max_offset().y;
        let offset_y = scroll_handle.offset().y;
        file_viewer_scrollbar(
            scroll_handle.clone(),
            offset_y,
            toolbar_h,
            viewport_h,
            content_h,
            cx,
        )
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
        .font_family(theme::editor_font_family(cx))
        .line_height(relative(theme::editor_line_height(cx)))
        .bg(viewer_bg)
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(toolbar_h)
                .child(render_file_viewer_toolbar(pane_id, fv, cx)),
        )
        .child(render_file_viewer_body(
            fv,
            images,
            &editor_state,
            scroll_handle,
            toolbar_h,
            px(0.),
            cx,
        ))
        .children(scrollbar)
        .children(search_panel)
}
