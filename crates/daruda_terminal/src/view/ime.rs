use gpui::{Context, SharedString};
use std::ops::Range;

use super::TerminalView;

impl TerminalView {
    pub(super) fn utf16_len(s: &str) -> usize {
        s.chars().map(|ch| ch.len_utf16()).sum()
    }

    pub(super) fn utf16_range_to_utf8(s: &str, range_utf16: Range<usize>) -> Option<Range<usize>> {
        let mut utf16_count = 0usize;
        let mut start_utf8: Option<usize> = None;
        let mut end_utf8: Option<usize> = None;

        if range_utf16.start == 0 {
            start_utf8 = Some(0);
        }
        if range_utf16.end == 0 {
            end_utf8 = Some(0);
        }

        for (utf8_index, ch) in s.char_indices() {
            if start_utf8.is_none() && utf16_count >= range_utf16.start {
                start_utf8 = Some(utf8_index);
            }
            if end_utf8.is_none() && utf16_count >= range_utf16.end {
                end_utf8 = Some(utf8_index);
            }

            utf16_count = utf16_count.saturating_add(ch.len_utf16());
        }

        if start_utf8.is_none() && utf16_count >= range_utf16.start {
            start_utf8 = Some(s.len());
        }
        if end_utf8.is_none() && utf16_count >= range_utf16.end {
            end_utf8 = Some(s.len());
        }

        Some(start_utf8?..end_utf8?)
    }

    pub(super) fn cell_offset_for_utf16(text: &str, utf16_offset: usize) -> usize {
        use unicode_width::UnicodeWidthChar as _;

        let mut cells = 0usize;
        let mut utf16_count = 0usize;
        for ch in text.chars() {
            if utf16_count >= utf16_offset {
                break;
            }

            let len_utf16 = ch.len_utf16();
            if utf16_count.saturating_add(len_utf16) > utf16_offset {
                break;
            }
            utf16_count = utf16_count.saturating_add(len_utf16);

            let width = ch.width().unwrap_or(0);
            if width > 0 {
                cells = cells.saturating_add(width);
            }
        }
        cells
    }

    pub(super) fn clear_marked_text(&mut self, cx: &mut Context<Self>) {
        self.state.marked_text = None;
        self.state.marked_selected_range_utf16 = 0..0;
        cx.notify();
    }

    pub(super) fn set_marked_text(
        &mut self,
        text: String,
        selected_range_utf16: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() {
            self.clear_marked_text(cx);
            return;
        }

        let total_utf16 = Self::utf16_len(&text);
        let selected = selected_range_utf16.unwrap_or(total_utf16..total_utf16);
        let selected = selected.start.min(total_utf16)..selected.end.min(total_utf16);

        self.state.marked_text = Some(SharedString::from(text));
        self.state.marked_selected_range_utf16 = selected;
        cx.notify();
    }

    pub(super) fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }

        if self.state.search_overlay {
            self.on_search_append(text, cx);
            return;
        }

        self.send_input_parts(&[text.as_bytes()], cx);
    }

    /// Release any in-progress Hangul composition to the PTY. Call before any
    /// non-IME key event that leaves the input region so a partial syllable is
    /// not stranded inside the composer.
    pub(super) fn flush_hangul(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = self.state.hangul_composer.flush() {
            self.commit_text(&s, cx);
        }
    }

    /// Send user-originated bytes to the PTY (keystrokes, IME commit, paste).
    /// Does not alter viewport scroll state — the user's scrollback position
    /// is preserved. Sets `pending_refresh` so `viewport_lines` is rebuilt on
    /// the next render; without this, PTY echo dirty-row updates land on the
    /// wrong rows when the viewport is pinned above the bottom.
    ///
    /// `sync_viewport_scroll_tracking` is intentionally absent: this function
    /// does not move the viewport, so there is no scroll delta to flush; the
    /// pending delta from the user's last wheel scroll is preserved for the
    /// output path.
    pub(super) fn send_input_parts(&mut self, parts: &[&[u8]], cx: &mut Context<Self>) {
        if parts.is_empty() {
            return;
        }

        self.snap_to_bottom();
        self.state.pending_refresh = true;
        self.state.pending_refresh_keep_selection = true;
        self.dispatch_parts_to_pty(parts, cx);
    }

    /// Send terminal-protocol bytes that were not triggered by user input
    /// (focus reporting, mouse reporting replies, status responses). Must not
    /// touch viewport scroll state — the user's scrollback position is
    /// preserved across focus changes and other protocol traffic.
    pub(super) fn send_protocol_parts(&mut self, parts: &[&[u8]], cx: &mut Context<Self>) {
        if parts.is_empty() {
            return;
        }

        self.dispatch_parts_to_pty(parts, cx);
    }

    fn dispatch_parts_to_pty(&mut self, parts: &[&[u8]], cx: &mut Context<Self>) {
        cx.notify();

        if let Some(input) = self.input.as_ref() {
            for bytes in parts {
                input.send(bytes);
            }
            return;
        }

        for bytes in parts {
            let _ = self.session.feed(bytes);
        }
        self.apply_side_effects(cx);
        self.schedule_viewport_refresh(cx);
    }
}
