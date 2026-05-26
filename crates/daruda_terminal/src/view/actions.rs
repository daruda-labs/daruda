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

        // Commit any in-flight Hangul composition before the paste
        // hits the PTY — otherwise the pasted bytes interleave ahead
        // of the pending syllable, garbling the input order.
        self.flush_hangul(cx);

        if self.state.search_overlay {
            // While the search overlay is up, paste feeds the query
            // input rather than the PTY. Strip embedded newlines —
            // they'd break the single-line query model.
            let sanitized: String = text.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
            self.on_search_append(&sanitized, cx);
            return;
        }

        if self.session.bracketed_paste_enabled() {
            self.send_input_parts(
                &[
                    crate::ansi::PTY_BRACKETED_PASTE_START,
                    text.as_bytes(),
                    crate::ansi::PTY_BRACKETED_PASTE_END,
                ],
                cx,
            );
        } else {
            self.send_input_parts(&[text.as_bytes()], cx);
        }
    }

    pub(super) fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        // Commit pending Hangul so the user's last visible syllable
        // is in the grid before we slice viewport_lines for the
        // selection. Without this the composed glyph (drawn as an
        // overlay only) is silently absent from the clipboard.
        self.flush_hangul(cx);

        let text = match self.state.selection {
            // Block mode: rectangular slice, rows joined by `\n`.
            // iTerm2 and Alacritty both copy each block row as its
            // own line so pasting reconstructs the rectangle.
            Some(sel) if sel.is_block() => {
                let Some(rect) = sel.block_rect() else {
                    return;
                };
                super::block_copy_text(&rect, &self.session)
            }
            // Linear mode: screen-position slice, survives scroll and
            // viewport repaints because ScreenPos is absolute. If
            // either endpoint has been evicted from LineBuffer the
            // selection has no live projection — skip the copy.
            Some(sel) => {
                let Some((start, end)) = sel.normalized(&self.session) else {
                    return;
                };
                if start == end {
                    return;
                }
                self.viewport_slice_screen(start, end)
            }
            None => return,
        };
        if text.is_empty() {
            return;
        }

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

    /// Wipe the visible viewport AND scrollback history. Mirrors
    /// iTerm2's ⌘K — feeds CSI `2J` + `H` (erase display, home cursor)
    /// followed by `3J` (erase scrollback) directly into the VT model
    /// so the result is independent of shell behaviour.
    pub(super) fn on_clear_buffer(
        &mut self,
        _: &ClearBuffer,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.session.feed(crate::ansi::ERASE_DISPLAY_AND_HOME);
        let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
        self.line_layouts.clear();
        self.line_layout_key = None;
        self.schedule_viewport_refresh(cx);
        cx.notify();
    }

    /// Drop scrollback history but keep the current viewport intact —
    /// iTerm2's ⇧⌘K behaviour.
    pub(super) fn on_clear_scrollback(
        &mut self,
        _: &ClearScrollback,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
        self.schedule_viewport_refresh(cx);
        cx.notify();
    }

    /// Release the viewport pin and snap to the bottom of the scrollback.
    /// Clears `user_scrolled` so future PTY output is followed normally.
    /// This is the explicit user-facing fallback for shells without OSC 133.
    pub(super) fn on_scroll_to_bottom(
        &mut self,
        _: &ScrollToBottom,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.snap_to_bottom();
        // Snap to bottom discards the viewport window but not the selection —
        // absolute ScreenPos coordinates remain valid until the rows are evicted.
        self.state.pending_refresh_keep_selection = true;
        self.state.pending_refresh = true;
        cx.notify();
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
        let new_is_regex = !self.state.search.is_regex;
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
        let is_regex = self.state.search.is_regex;
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
        let is_regex = self.state.search.is_regex;
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
        let is_regex = self.state.search.is_regex;
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
        let is_regex = self.state.search.is_regex;
        self.set_search_query(&q, case_insensitive, is_regex, cx);
        self.state.search.cursor_byte = cur + text.len();
    }
}
