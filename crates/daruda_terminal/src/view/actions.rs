use gpui::{ClipboardItem, Context, Window};

use super::TerminalView;
use super::selection::ScreenPos;
use super::text_edit::{step_char_left, step_char_right};
use super::{
    ByteSelection, ClearBuffer, ClearScrollback, CommandJumpNext, CommandJumpPrev, Copy,
    CopyLastCommandOutput, Paste, PromptJumpNext, PromptJumpPrev, ResetZoom, ScrollToBottom,
    SearchBackspace, SearchClearQuery, SearchClose, SearchCursorEnd, SearchCursorHome,
    SearchCursorLeft, SearchCursorRight, SearchDeleteForward, SearchNext, SearchOpen, SearchPrev,
    SearchToggleRegex, SelectAll, ToggleFullscreen, ZoomIn, ZoomOut,
};

impl TerminalView {
    pub(super) fn on_paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        self.paste_text(&text, cx);
    }

    pub(super) fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self.selection_text(cx) else {
            return;
        };

        let item = ClipboardItem::new_string(text);
        cx.write_to_clipboard(item.clone());
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        cx.write_to_primary(item);
    }

    pub(super) fn on_select_all(
        &mut self,
        _: &SelectAll,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let vp_offset = self.session.viewport_row_offset();
        let last_row = vp_offset + self.state.viewport_lines.len().saturating_sub(1) as u32;
        let last_byte = self
            .state
            .viewport_lines
            .last()
            .map(|l| l.len())
            .unwrap_or(0);
        // SelectAll covers the live viewport only — both endpoints are
        // viewport-resident so the simpler `Viewport` anchor is enough.
        // TODO(scrollback-selectall): Cmd+A currently selects only the
        // live viewport, unlike iTerm2 / most terminals which extend
        // through the full LineBuffer scrollback. Extending would need
        // a Scrollback PosAnchor at `lb.position_at(0)` for `anchor`
        // and a Viewport anchor at the last live cell for `active`.
        self.state.selection = Some(ByteSelection::linear(
            ScreenPos::viewport(vp_offset, 0),
            ScreenPos::viewport(last_row, last_byte),
        ));
        self.on_copy(&Copy, window, cx);
        cx.notify();
    }

    /// Apply a delta to the terminal body font size, clamp to the
    /// iTerm2-style sane range, and invalidate the shape cache.
    /// Mirrors `iTermAdjustFontSizeHelper.m::adjustFontSizeBy:`:
    /// the profile font is the source of truth, zoom edits it in
    /// place rather than scaling a global `rem_size`.
    fn adjust_font_size(&mut self, delta: f32) {
        let next = (self.state.font_size + delta).clamp(crate::FONT_SIZE_MIN, crate::FONT_SIZE_MAX);
        if next == self.state.font_size {
            return;
        }
        self.state.font_size = next;
        self.line_layouts.clear();
        self.line_layout_key = None;
    }

    pub(super) fn on_zoom_in(&mut self, _: &ZoomIn, _window: &mut Window, cx: &mut Context<Self>) {
        // Flush before the shape cache is invalidated — otherwise
        // the next prepaint would re-shape the preedit at the new
        // size while bounds_for_range still reports old metrics,
        // briefly misaligning the IME candidate window.
        self.flush_hangul(cx);
        self.adjust_font_size(2.0);
        cx.notify();
    }

    pub(super) fn on_zoom_out(
        &mut self,
        _: &ZoomOut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_hangul(cx);
        self.adjust_font_size(-2.0);
        cx.notify();
    }

    pub(super) fn on_reset_zoom(
        &mut self,
        _: &ResetZoom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flush_hangul(cx);
        self.state.font_size = self.session.font_size();
        self.line_layouts.clear();
        self.line_layout_key = None;
        cx.notify();
    }

    pub(super) fn on_toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    /// Wipe everything above the current prompt plus the scrollback history,
    /// leaving the prompt at the top of the screen. Mirrors iTerm2's ⌘K
    /// (`clearBufferSavingPrompt:`) and zed's `InternalEvent::Clear`, both of
    /// which preserve the prompt line.
    ///
    /// The clear is local to the VT model — the shell is never told — so
    /// erasing the prompt row would simply lose it: nothing would repaint,
    /// and the shell's line editor would keep computing redraws from a cursor
    /// position the grid no longer agrees with. Scrolling the prompt to the
    /// top instead keeps the cursor's offset *within* that line intact, which
    /// is the only relationship the shell tracks.
    pub(super) fn on_clear_buffer(
        &mut self,
        _: &ClearBuffer,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Unlock before erasing so the viewport snaps to the live screen
        // immediately rather than waiting for anchor eviction on the next
        // PTY output tick.
        self.snap_to_bottom();
        // A full-screen app owns the alt screen and repaints on its own
        // schedule, so shifting its rows would leave the display disagreeing
        // with the app's model until the next full redraw. Drop the primary
        // screen's history — the only part of the buffer the user can still
        // reach — and leave the display to its owner.
        if !self.session.is_alt_screen() {
            let above = self.rows_above_prompt();
            let _ = self.session.feed(&crate::ansi::lift_rows_to_top(above));
        }
        let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
        self.line_layouts.clear();
        self.line_layout_key = None;
        self.schedule_viewport_refresh(cx);
    }

    /// Viewport rows sitting above the region the clear must keep.
    ///
    /// The kept region runs from the current prompt's first row through the
    /// cursor, so a multi-line prompt or a typed command that wrapped stays
    /// whole. Without shell integration there is no prompt mark to consult
    /// and only the cursor's own row is kept — the same fallback iTerm2's
    /// `numberOfLinesToPreserveWhenClearingScreen` uses.
    ///
    /// Both inputs are normalised to **0-indexed viewport rows** first: the
    /// cursor is a 1-indexed grid row, a mark projects to an absolute screen
    /// row, and `SU` counts viewport rows. Mixing the three is the coordinate
    /// hazard this file's module docs call out.
    fn rows_above_prompt(&self) -> u16 {
        let Some((_, cursor_grid_row)) = self.session.cursor_position() else {
            return 0;
        };
        let cursor_viewport_row = cursor_grid_row.saturating_sub(1);
        rows_above_kept_region(cursor_viewport_row, self.prompt_viewport_row())
    }

    /// Current prompt's first row in 0-indexed viewport space, if shell
    /// integration reported one, it has not been executed yet, and it is
    /// still on screen.
    fn prompt_viewport_row(&self) -> Option<u16> {
        let viewport_top = self.session.viewport_row_offset();
        let viewport_rows = u32::from(self.session.rows());
        let screen_row = current_prompt_start(self.session.prompt_marks().iter())
            .and_then(|mark| self.session.abs_to_screen_row(mark.abs_y))?;
        super::overlay::screen_row_to_visible(screen_row, viewport_top, viewport_rows)
            .and_then(|row| u16::try_from(row).ok())
    }

    /// Drop scrollback history but keep the current viewport intact —
    /// iTerm2's ⇧⌘K behaviour.
    pub(super) fn on_clear_scrollback(
        &mut self,
        _: &ClearScrollback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Scrollback is gone — any saved anchor is now invalid. Unlock
        // immediately so future output is followed normally.
        self.snap_to_bottom();
        let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
        self.schedule_viewport_refresh(cx);
    }

    /// Unlock the viewport and snap to the bottom of the scrollback.
    /// This is the explicit user-facing fallback for shells without OSC 133.
    pub(super) fn on_scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Snap to bottom discards the viewport window but not the selection —
        // absolute ScreenPos coordinates remain valid until the rows are evicted.
        self.snap_to_bottom_and_refresh(cx);
    }

    pub(super) fn on_search_open(
        &mut self,
        _: &SearchOpen,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.search_overlay = true;
        // Re-opening on top of a query keeps the cursor at the end
        // for further refinement.
        self.state.search.cursor_byte = self.state.search.query.len();
        cx.notify();
    }

    pub(super) fn on_search_close(
        &mut self,
        _: &SearchClose,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.state.search_overlay = false;
        self.clear_search(cx);
    }

    pub(super) fn on_search_cursor_left(
        &mut self,
        _: &SearchCursorLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        // Stop propagation so the on_key_down terminal handler does
        // not also encode the arrow key as an ANSI escape and send
        // it to the PTY — without this, every Left in the search
        // overlay would simultaneously edit the query AND poke the
        // shell. Same rationale for the other search input actions
        // below.
        cx.stop_propagation();
        let next = step_char_left(&self.state.search.query, self.state.search.cursor_byte);
        if next == self.state.search.cursor_byte {
            return;
        }
        self.state.search.cursor_byte = next;
        cx.notify();
    }

    pub(super) fn on_search_cursor_right(
        &mut self,
        _: &SearchCursorRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        let next = step_char_right(&self.state.search.query, self.state.search.cursor_byte);
        if next == self.state.search.cursor_byte {
            return;
        }
        self.state.search.cursor_byte = next;
        cx.notify();
    }

    pub(super) fn on_search_cursor_home(
        &mut self,
        _: &SearchCursorHome,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        self.state.search.cursor_byte = 0;
        cx.notify();
    }

    pub(super) fn on_search_cursor_end(
        &mut self,
        _: &SearchCursorEnd,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        self.state.search.cursor_byte = self.state.search.query.len();
        cx.notify();
    }

    pub(super) fn on_search_toggle_regex(
        &mut self,
        _: &SearchToggleRegex,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        let new_is_regex = !self.state.search.is_regex();
        let q = self.state.search.query.clone();
        let case = self.state.search.case_insensitive;
        self.set_search_query(&q, case, new_is_regex, cx);
    }

    pub(super) fn on_search_clear_query(
        &mut self,
        _: &SearchClearQuery,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        let case_insensitive = self.state.search.case_insensitive;
        let is_regex = self.state.search.is_regex();
        self.set_search_query("", case_insensitive, is_regex, cx);
        self.state.search.cursor_byte = 0;
    }

    pub(super) fn on_search_delete_forward(
        &mut self,
        _: &SearchDeleteForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        let mut q = self.state.search.query.clone();
        let cur = self.state.search.cursor_byte.min(q.len());
        let next = step_char_right(&q, cur);
        if next == cur {
            return;
        }
        q.replace_range(cur..next, "");
        let case_insensitive = self.state.search.case_insensitive;
        let is_regex = self.state.search.is_regex();
        self.set_search_query(&q, case_insensitive, is_regex, cx);
        self.state.search.cursor_byte = cur;
    }

    pub(super) fn on_search_next(
        &mut self,
        _: &SearchNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.search_step(true, cx);
    }

    pub(super) fn on_search_prev(
        &mut self,
        _: &SearchPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.search_step(false, cx);
    }

    pub(super) fn on_search_backspace(
        &mut self,
        _: &SearchBackspace,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.search_overlay {
            return;
        }
        cx.stop_propagation();
        let mut q = self.state.search.query.clone();
        let cur = self.state.search.cursor_byte.min(q.len());
        let prev = step_char_left(&q, cur);
        if prev == cur {
            return;
        }
        q.replace_range(prev..cur, "");
        let case_insensitive = self.state.search.case_insensitive;
        let is_regex = self.state.search.is_regex();
        self.set_search_query(&q, case_insensitive, is_regex, cx);
        self.state.search.cursor_byte = prev;
    }

    pub(super) fn on_prompt_jump_prev(
        &mut self,
        _: &PromptJumpPrev,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_prompt(false, window, cx);
    }

    pub(super) fn on_prompt_jump_next(
        &mut self,
        _: &PromptJumpNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_prompt(true, window, cx);
    }

    pub(super) fn on_command_jump_prev(
        &mut self,
        _: &CommandJumpPrev,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_command(false, window, cx);
    }

    pub(super) fn on_command_jump_next(
        &mut self,
        _: &CommandJumpNext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_command(true, window, cx);
    }

    /// Copy the most recent shell command's output (FTCS E/F pair, or
    /// C/D pair when the shell only emits the coarse boundaries) to
    /// the clipboard. No-op when no closed pair has been recorded —
    /// e.g. the very first prompt of a fresh session.
    pub(super) fn on_copy_last_command_output(
        &mut self,
        _: &CopyLastCommandOutput,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(range) = self.session.last_command_output_rows() else {
            return;
        };
        let mut out = String::new();
        for y in range {
            if let Ok(line) = self.session.dump_screen_row(y) {
                let trimmed = line.strip_suffix('\n').unwrap_or(line.as_str());
                out.push_str(trimmed.trim_end());
                out.push('\n');
            }
        }
        if out.is_empty() {
            return;
        }
        let item = ClipboardItem::new_string(out);
        cx.write_to_clipboard(item.clone());
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        cx.write_to_primary(item);
    }

    /// Insert `text` at the current cursor position in the search
    /// query. Used by both the IME-driven keystroke path and the
    /// paste path.
    pub(super) fn on_search_append(&mut self, text: &str, cx: &mut Context<Self>) {
        if !self.state.search_overlay || text.is_empty() {
            return;
        }
        let mut q = self.state.search.query.clone();
        let cur = self.state.search.cursor_byte.min(q.len());
        q.insert_str(cur, text);
        let case_insensitive = self.state.search.case_insensitive;
        let is_regex = self.state.search.is_regex();
        self.set_search_query(&q, case_insensitive, is_regex, cx);
        self.state.search.cursor_byte = cur + text.len();
    }
}

/// Rows above the region a buffer-clear keeps, in 0-indexed viewport space.
///
/// `prompt_row` is the current prompt's first row when shell integration
/// reported one. A mark above the cursor widens the kept region to cover a
/// multi-line prompt or a wrapped command; a mark at or below the cursor is
/// ignored, because the prompt cannot start after the point the user is
/// typing at and a stale mark must not shrink the region to nothing.
fn rows_above_kept_region(cursor_row: u16, prompt_row: Option<u16>) -> u16 {
    match prompt_row {
        Some(prompt) if prompt < cursor_row => prompt,
        _ => cursor_row,
    }
}

/// Latest prompt mark that still describes editable input. `CommandStart`
/// (FTCS B) is part of that prompt/input region, so it does not invalidate
/// the prompt; `CommandExecuted`/`CommandFinished` mean the shell has moved
/// on to command output or completion, and a prior PromptStart is stale.
fn current_prompt_start<'a>(
    marks: impl DoubleEndedIterator<Item = &'a crate::session::PromptMark>,
) -> Option<&'a crate::session::PromptMark> {
    for mark in marks.rev() {
        match mark.kind {
            crate::session::PromptMarkKind::PromptStart => return Some(mark),
            crate::session::PromptMarkKind::CommandExecuted
            | crate::session::PromptMarkKind::CommandFinished => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{current_prompt_start, rows_above_kept_region};
    use crate::session::{LogicalLineAbs, PromptMark, PromptMarkKind};

    fn mark(seq: u64, kind: PromptMarkKind) -> PromptMark {
        PromptMark {
            kind,
            seq,
            abs_y: LogicalLineAbs(seq),
            screen_col: 1,
            exit_code: None,
            timestamp: std::time::SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn without_a_prompt_mark_only_the_cursor_row_is_kept() {
        assert_eq!(rows_above_kept_region(4, None), 4);
    }

    #[test]
    fn a_prompt_above_the_cursor_widens_the_kept_region() {
        // Prompt started two rows up (wrapped command) — keep all three rows.
        assert_eq!(rows_above_kept_region(4, Some(2)), 2);
    }

    #[test]
    fn a_prompt_at_or_below_the_cursor_is_ignored() {
        assert_eq!(rows_above_kept_region(4, Some(4)), 4);
        assert_eq!(rows_above_kept_region(4, Some(9)), 4);
    }

    #[test]
    fn a_cursor_already_at_the_top_scrolls_nothing() {
        assert_eq!(rows_above_kept_region(0, None), 0);
        assert_eq!(rows_above_kept_region(0, Some(0)), 0);
    }

    #[test]
    fn command_start_keeps_the_prompt_current() {
        let marks = [
            mark(1, PromptMarkKind::PromptStart),
            mark(2, PromptMarkKind::CommandStart),
        ];
        assert_eq!(
            current_prompt_start(marks.iter()).map(|mark| mark.seq),
            Some(1)
        );
    }

    #[test]
    fn command_execution_makes_the_prior_prompt_stale() {
        let marks = [
            mark(1, PromptMarkKind::PromptStart),
            mark(2, PromptMarkKind::CommandStart),
            mark(3, PromptMarkKind::CommandExecuted),
        ];
        assert!(current_prompt_start(marks.iter()).is_none());
    }

    #[test]
    fn a_new_prompt_after_command_finish_is_current() {
        let marks = [
            mark(1, PromptMarkKind::PromptStart),
            mark(2, PromptMarkKind::CommandStart),
            mark(3, PromptMarkKind::CommandExecuted),
            mark(4, PromptMarkKind::CommandFinished),
            mark(5, PromptMarkKind::PromptStart),
        ];
        assert_eq!(
            current_prompt_start(marks.iter()).map(|mark| mark.seq),
            Some(5)
        );
    }
}
