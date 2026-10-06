//! Drag payload for left-dock path items (Files + Git Changes).
//!
//! `PathDrag` is registered as the drag value on each file/directory row.
//! `TextArea` drop zones receive it via `on_drop::<PathDrag>` and insert
//! the absolute path at the current cursor position.

use std::path::PathBuf;

use crate::ui::theme;
use gpui::{Context, IntoElement, Pixels, Point, Render, Window, div, prelude::*, px};

/// The value carried during a dock → TextArea drag operation.
#[derive(Clone)]
pub(in crate::workspace) struct PathDrag {
    pub path: PathBuf,
    /// Cursor position within the source row at drag start.
    /// Used to offset the pill so it appears near the cursor
    /// rather than at the element's top-left corner.
    pub offset: Point<Pixels>,
}

impl Render for PathDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned());
        let pill_bg = theme::CANVAS;
        let pill_text = theme::TEXT_PRIMARY;
        // GPUI positions the ghost at (cursor - offset), so padding the wrapper
        // by (offset + DRAG_PILL_CURSOR_OFFSET) places the pill at cursor + (4, 4).
        div()
            .pl(self.offset.x + px(theme::DRAG_PILL_CURSOR_OFFSET))
            .pt(self.offset.y + px(theme::DRAG_PILL_CURSOR_OFFSET))
            .child(
                div()
                    .px(px(theme::MODAL_INPUT_PAD))
                    .py(px(theme::PANEL_BODY_PAD_Y))
                    .bg(pill_bg)
                    .rounded(px(theme::MODAL_BUTTON_RADIUS))
                    .text_size(px(theme::FILES_ROW_FONT_SIZE))
                    .text_color(pill_text)
                    .child(label),
            )
    }
}
