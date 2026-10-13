//! Viewport dumps, cursor state, scroll position, and per-frame viewport
//! drains.

use super::*;

impl TerminalSession {
    /// Dump the visible viewport. When `scroll_offset > 0` the viewport
    /// has been scrolled into `line_buffer` history, so compose the
    /// output from the dispatcher (`dump_screen_row`) instead of
    /// ghostty's live grid — otherwise PageUp / scrollbar would be a
    /// visual no-op. Output shape matches ghostty's `dump_viewport`:
    /// rows joined by `\n`, no trailing newline (see
    /// `view::viewport::split_viewport_lines`).
    pub fn dump_viewport(&self) -> Result<String, Error> {
        if self.scroll_offset == 0 {
            return self.terminal.dump_viewport();
        }
        let rows = self.dims.rows as u32;
        let top = self.viewport_row_offset();
        let mut out = String::new();
        for i in 0..rows {
            let y = top + i;
            let row = self.dump_screen_row(y).unwrap_or_default();
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&row);
        }
        Ok(out)
    }

    /// Viewport-relative row text. Keeps the original `0..rows`
    /// addressing the live grid uses (dirty-row repaint). When
    /// `scroll_offset > 0` the row addresses a position inside
    /// `line_buffer` history, so dispatch through [`Self::dump_screen_row`]
    /// to stay consistent with [`Self::dump_viewport`] and
    /// [`Self::dump_viewport_row_style_runs`]. Without this, the dirty-row
    /// fast path paints live-grid rows over scrolled-back content.
    pub fn dump_viewport_row(&self, row: ViewportRow) -> Result<String, Error> {
        let row = row.get();
        if self.scroll_offset == 0 {
            return self.terminal.dump_viewport_row(row);
        }
        let top = self.viewport_row_offset();
        let y = top + row as u32;
        self.dump_screen_row(y)
    }

    /// Dump a single row from the `screen` coordinate space. `y`
    /// ranges over `[0, total_rows())`, covering scrollback and the
    /// active viewport.
    ///
    /// Dispatcher: when `y` addresses a row inside `line_buffer`'s
    /// wrapped range, the row text comes from there; rows beyond that
    /// land on ghostty's live viewport.
    pub fn dump_screen_row(&self, y: u32) -> Result<String, Error> {
        let cols = self.dims.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if y < lb_rows {
            self.line_buffer
                .visual_row(y, cols)
                .ok_or(Error::DumpFailed)
        } else {
            let vp_row = (y - lb_rows) as u16;
            self.terminal.dump_viewport_row(vp_row)
        }
    }

    /// Style runs for any absolute screen row. Dispatcher mirror of
    /// [`Self::dump_screen_row`]: scrollback rows come from
    /// `line_buffer`, viewport rows from ghostty. Output columns are
    /// 1-indexed inclusive (StyleRun convention).
    pub fn dump_screen_row_styles(&self, y: u32) -> Result<Vec<ghostty_vt::StyleRun>, Error> {
        let cols = self.dims.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if y < lb_rows {
            Ok(self
                .line_buffer
                .visual_row_with_styles(y, cols)
                .map(|(_, r)| r)
                .unwrap_or_default())
        } else {
            let vp_row = (y - lb_rows) as u16;
            self.terminal.dump_viewport_row_style_runs(vp_row)
        }
    }

    /// Viewport-relative style runs. Keeps the original 0..rows
    /// addressing the live grid uses (background quads, dirty-row
    /// repaint). When `scroll_offset > 0` the row addresses a position
    /// inside `line_buffer` history, so dispatch through
    /// [`Self::dump_screen_row_styles`] to stay consistent with
    /// [`Self::dump_viewport`]. Pure-scrollback callers (no viewport
    /// translation) should reach for [`Self::dump_screen_row_styles`]
    /// directly.
    pub fn dump_viewport_row_style_runs(
        &self,
        row: ViewportRow,
    ) -> Result<Vec<ghostty_vt::StyleRun>, Error> {
        let row = row.get();
        if self.scroll_offset == 0 {
            return self.terminal.dump_viewport_row_style_runs(row);
        }
        let top = self.viewport_row_offset();
        let y = top + row as u32;
        self.dump_screen_row_styles(y)
    }

    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        self.terminal.cursor_position()
    }

    pub fn cursor_style(&self) -> u8 {
        self.terminal.cursor_style()
    }

    pub fn cursor_visible(&self) -> bool {
        self.terminal.cursor_visible()
    }

    pub fn take_bell(&mut self) -> bool {
        self.terminal.take_bell()
    }

    /// Absolute row index at the top of the visible viewport.
    /// `scroll_offset == 0` pins the viewport to the bottom; positive
    /// `scroll_offset` walks back into `line_buffer` scrollback.
    pub fn viewport_row_offset(&self) -> u32 {
        self.total_rows()
            .saturating_sub(self.dims.rows as u32)
            .saturating_sub(self.scroll_offset)
    }

    /// Absolute **visual-row** index at the topmost visible row.
    ///
    /// Expressed as `line_buffer.overflow() + viewport_row_offset()`,
    /// where `viewport_row_offset` is in visual-row (`total_rows`) space.
    /// Capture this when the user scrolls and pass it to [`ViewportLock`]
    /// as the anchor. During subsequent IND / SU grid scrolls,
    /// `viewport_row_offset` increases as new rows are captured into
    /// `LineBuffer`, so the stored anchor no longer matches the
    /// top-of-viewport — `restore_pinned_viewport` detects the drift via
    /// [`Self::viewport_anchor_to_screen_row`] and re-seeks. Conversely,
    /// scrolling up (increasing `scroll_offset`) decreases this value, so
    /// it is not monotone.
    ///
    /// **Distinct space from `PromptMark.abs_y`.** This value lives in
    /// visual-row space (`overflow + screen_row`); [`PromptMark::abs_y`]
    /// lives in logical-line space. The two are NOT interchangeable —
    /// translate the viewport anchor via
    /// [`Self::viewport_anchor_to_screen_row`] (visual-row math) and the
    /// mark abs via [`Self::abs_to_screen_row`] (logical-line projection).
    /// Mixing them would put the viewport pin on the wrong row at any
    /// wrap-inflated width.
    pub(crate) fn viewport_top_abs_y(&self) -> u64 {
        self.line_buffer
            .overflow()
            .saturating_add(self.viewport_row_offset() as u64)
    }

    /// Translate a viewport anchor (captured via
    /// [`Self::viewport_top_abs_y`], visual-row space) back to a
    /// current screen-frame row. Returns `None` when the anchor row has
    /// been evicted from `LineBuffer` or pushed past the live frame.
    ///
    /// **Why this exists alongside [`Self::abs_to_screen_row`].** The
    /// mark-side translator now takes logical-line abs; the viewport
    /// anchor stays in visual-row space because the user may park the
    /// viewport top on a wrap-continuation row (which has no
    /// logical-line abs of its own). A 1-row precision loss on
    /// continuation rows would make `restore_pinned_viewport`'s round
    /// trip silently snap up to the head of the logical line on every
    /// IND / SU, so the two spaces remain separated by call path.
    pub(crate) fn viewport_anchor_to_screen_row(&self, anchor: u64) -> Option<u32> {
        let overflow = self.line_buffer.overflow();
        if anchor < overflow {
            return None;
        }
        let rel = anchor - overflow;
        let total = self.total_rows() as u64;
        if rel >= total {
            return None;
        }
        u32::try_from(rel).ok()
    }

    /// Current scroll offset in rows back from the bottom of the
    /// unified frame. `0` pins the viewport to the live grid; positive
    /// values walk back into `line_buffer` history. Exposed for tests
    /// that need to assert exact scroll positions; production callers
    /// should reach for [`Self::viewport_row_offset`] instead.
    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> u32 {
        self.scroll_offset
    }

    /// Move the viewport `delta_lines` rows within the unified frame.
    /// `delta < 0` scrolls UP into history (increases `scroll_offset`),
    /// matching existing call-site convention (`scroll_viewport(-step)`
    /// for PageUp / wheel-up). Clamps to the available scrollback so
    /// `scroll_offset` never exceeds `line_buffer.wrapped_row_count`.
    pub fn scroll_viewport(&mut self, delta_lines: i32) -> Result<(), Error> {
        let max_scroll = self.line_buffer.wrapped_row_count(self.dims.cols);
        let cur = self.scroll_offset as i32;
        let new_offset = (cur - delta_lines).clamp(0, max_scroll as i32);
        self.scroll_offset = new_offset as u32;
        Ok(())
    }

    /// Pin the viewport to the top of the unified frame (oldest
    /// `line_buffer` row at the top of the visible area).
    pub fn scroll_viewport_top(&mut self) -> Result<(), Error> {
        self.scroll_offset = self.line_buffer.wrapped_row_count(self.dims.cols);
        Ok(())
    }

    /// Pin the viewport to the live grid (`scroll_offset = 0`).
    pub fn scroll_viewport_bottom(&mut self) -> Result<(), Error> {
        self.scroll_offset = 0;
        Ok(())
    }

    pub fn take_dirty_viewport_rows(&mut self) -> Vec<u16> {
        self.terminal
            .take_dirty_viewport_rows(self.dims.rows)
            .unwrap_or_default()
    }

    pub fn take_viewport_scroll_delta(&mut self) -> i32 {
        self.terminal.take_viewport_scroll_delta()
    }

    /// Drain one-shot grid events (alt-screen toggle, RIS) recorded by
    /// ghostty since the last call. Used by the viewport reconcile path
    /// to decide whether to invalidate selection.
    pub fn take_grid_events(&mut self) -> Vec<ghostty_vt::GridEvent> {
        self.terminal.take_grid_events()
    }
}
