//! Window-position to cell / screen-position conversion and the URL, word,
//! and line lookups built on it.

use super::*;

impl TerminalView {
    /// Convert a window pixel position to an absolute `ScreenPos`.
    /// Returns `None` if the terminal has no rows or no valid cell metrics.
    pub(in crate::view) fn mouse_position_to_screen_pos(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<ScreenPos> {
        let rows = self.session.rows() as usize;
        if rows == 0 {
            return None;
        }

        let local_pos = self.mouse_position_to_local(position);
        let layout = self.cell_layout(window)?;
        let x = f32::from(local_pos.x);
        let y = f32::from(local_pos.y);
        let mut row_index = (y / layout.line_height).floor() as i32;
        if row_index < 0 {
            row_index = 0;
        }
        if row_index >= rows as i32 {
            row_index = rows as i32 - 1;
        }
        let row_index = row_index as usize;

        let vp_offset = self.session.viewport_row_offset();
        let screen_row = vp_offset + row_index as u32;

        let content_rows = self.state.viewport_lines.len();
        if row_index >= content_rows {
            // Click past all content rows — clamp to one-past-last-content.
            let last_len = self
                .state
                .viewport_lines
                .last()
                .map(|l| l.len() + 1)
                .unwrap_or(0);
            let last_row = vp_offset + content_rows.saturating_sub(1) as u32;
            return Some(ScreenPos::viewport(last_row, last_len));
        }

        let line_text = self
            .state
            .viewport_lines
            .get(row_index)
            .map(|s| s.as_str())
            .unwrap_or("");

        let byte = if let Some(Some(line)) = self.line_layouts.get(row_index) {
            let b = line.closest_index_for_x(px(x)).min(line.text.len());
            if b >= line.text.len() {
                let text_px_width = f32::from(line.x_for_index(line.text.len()));
                if x > text_px_width {
                    line_text.len() + 1
                } else {
                    b
                }
            } else {
                b
            }
        } else {
            let col_index = (x / layout.cell_width).floor() as i32 + 1;
            let col = col_index.max(1).min(self.session.cols() as i32) as u16;
            let b = super::super::text_metrics::byte_index_for_column_in_line(line_text, col)
                .min(line_text.len());
            if b >= line_text.len() {
                use unicode_width::UnicodeWidthStr as _;
                let display_cols = line_text.width();
                if col as usize > display_cols {
                    line_text.len() + 1
                } else {
                    b
                }
            } else {
                b
            }
        };

        // When the row lies in scrollback (covered by `LineBuffer`),
        // promote the anchor to a width-invariant `LineBufferPosition`
        // so the selection survives a subsequent resize. Viewport-resident
        // rows keep the byte-offset storage; they get re-clamped after a
        // resize via the normal `refresh_viewport_preserving_selection`
        // path.
        let cell_cols = self.session.cols();
        let lb_rows = self.session.line_buffer().wrapped_row_count(cell_cols);
        if screen_row < lb_rows {
            // Translate the row-local byte offset back to a 1-indexed
            // cell column so `anchor_at` can derive the cumulative
            // cell column within the logical line.
            let cell_col = super::super::text_metrics::column_for_byte_in_line(line_text, byte);
            return Some(ScreenPos::anchor_at(&self.session, screen_row, cell_col));
        }
        Some(ScreenPos::viewport(screen_row, byte))
    }

    /// URL at the given absolute screen position (viewport rows only).
    pub(in crate::view) fn url_at_screen_pos(&self, pos: ScreenPos) -> Option<String> {
        let (screen_row, byte) = pos.resolve(&self.session)?;
        let vp_offset = self.session.viewport_row_offset();
        let vp_row = screen_row.checked_sub(vp_offset)? as usize;
        let line = self.state.viewport_lines.get(vp_row)?;
        super::super::url::url_at_byte_index(line, byte.min(line.len().saturating_sub(1)))
    }

    pub(in crate::view) fn word_range_at(
        &self,
        pos: ScreenPos,
        vp_offset: u32,
    ) -> (ScreenPos, ScreenPos) {
        super::super::viewport::word_range_in_viewport(
            pos,
            &self.session,
            &self.state.viewport_lines,
            vp_offset,
        )
    }

    pub(in crate::view) fn line_range_at(
        &self,
        pos: ScreenPos,
        vp_offset: u32,
    ) -> (ScreenPos, ScreenPos) {
        super::super::viewport::line_range_in_viewport(
            pos,
            &self.session,
            &self.state.viewport_lines,
            vp_offset,
        )
    }

    pub(in crate::view) fn mouse_position_to_cell(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<(u16, u16)> {
        let cols = self.session.cols();
        let rows = self.session.rows();

        let position = self.mouse_position_to_local(position);
        let layout = self.cell_layout(window)?;
        let x = f32::from(position.x);
        let y = f32::from(position.y);

        let mut col = (x / layout.cell_width).floor() as i32 + 1;
        let mut row = (y / layout.line_height).floor() as i32 + 1;

        if col < 1 {
            col = 1;
        }
        if row < 1 {
            row = 1;
        }
        if col > cols as i32 {
            col = cols as i32;
        }
        if row > rows as i32 {
            row = rows as i32;
        }

        Some((col as u16, row as u16))
    }

    pub(in crate::view) fn mouse_position_to_local(
        &self,
        position: gpui::Point<Pixels>,
    ) -> gpui::Point<Pixels> {
        window_position_to_local(
            self.state.last_bounds,
            position,
            self.state.inset_x,
            self.state.inset_y,
        )
    }
}
