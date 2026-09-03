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
pub(in crate::workspace) use self::markdown::CachedImage;

use crate::ui::theme;
use gpui::{Context, IntoElement, SharedString, div, prelude::*, px};

use self::body::render_file_viewer_body;
use self::scrollbar::file_viewer_scrollbar;
use self::search_panel::render_search_panel;
use self::toolbar::render_file_viewer_toolbar;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::{FileViewMode, PaneFileContent, PaneFileView};
use crate::workspace::main_area::pane_tree::PaneId;

/// Top-level file-viewer element.
/// Absolute positioning gives the body a bounded height for scrolling.
pub(in crate::workspace) fn render_pane_file_viewer(
    pane_id: PaneId,
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

    // Preview has variable-height blocks; derive height from measured offset.
    let is_preview_mode = matches!(&fv.content, PaneFileContent::LoadedMarkdown { .. })
        && fv.view_mode == FileViewMode::Preview;
    let surface = theme::PaneSurfaceTokens::file_viewer(cx);
    let viewer_bg = surface.background;

    // One thin daruda thumb; editor modes use the editor scroll handle.
    let scrollbar: Option<crate::ui::scrollbar::Thumb> = if is_editor_mode {
        let editor = editor_state.read(cx);
        // `editor_scroll.bounds()`/`.max_offset()` never populate for this
        // vendored editor (see `InputState::scroll_size` doc comment) — the
        // real viewport/content geometry lives in `last_bounds()` /
        // `scroll_size()` instead.
        let editor_scroll = editor.scroll_handle().clone();
        let viewport_h = editor.last_bounds().map_or(px(0.), |b| b.size.height);
        let content_h = editor.scroll_size().height;
        file_viewer_scrollbar(&editor_scroll, toolbar_h, viewport_h, content_h, cx)
    } else {
        let viewport_h = scroll_handle.bounds().size.height;
        let content_h = if is_preview_mode {
            viewport_h + scroll_handle.max_offset().y
        } else {
            let total_rows = fv.visible_row_count();
            px(total_rows as f32 * theme::FILE_VIEWER_LINE_H)
        };
        file_viewer_scrollbar(scroll_handle, toolbar_h, viewport_h, content_h, cx)
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
                .child(render_file_viewer_toolbar(pane_id, fv, cx)),
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
