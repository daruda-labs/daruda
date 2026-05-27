use gpui::Context;

use super::TerminalView;
use super::selection::ScreenPos;
use super::selection_policy::{self, InvalidationReason};
use crate::coords::ViewportRow;
use crate::session::TerminalSession;

pub(super) fn split_viewport_lines(viewport: &str) -> Vec<String> {
    let viewport = viewport.strip_suffix('\n').unwrap_or(viewport);
    if viewport.is_empty() {
        return Vec::new();
    }
    viewport.split('\n').map(|line| line.to_string()).collect()
}

/// Returns the `(start, end)` screen positions that bound the word
/// under `pos`, clamped to the row `pos` is on.  `vp_offset` is
/// `session.viewport_row_offset()` cast to `u32`. `pos` is resolved
/// against `session` so scrollback-resident anchors project to the
/// current frame before the word walk.
pub(crate) fn word_range_in_viewport(
    pos: ScreenPos,
    session: &TerminalSession,
    lines: &[String],
    vp_offset: u32,
) -> (ScreenPos, ScreenPos) {
    let Some((screen_row, byte)) = pos.resolve(session) else {
        return (pos, pos);
    };
    let viewport_row = (screen_row.saturating_sub(vp_offset)) as usize;
    let Some(line) = lines.get(viewport_row) else {
        return (pos, pos);
    };
    if line.is_empty() {
        return (pos, pos);
    }
    let local = byte.min(line.len().saturating_sub(1));
    // Walk back to the nearest valid UTF-8 char boundary (floor_char_boundary
    // is only stable since 1.91, so we replicate the logic inline).
    let local = (0..=local)
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);

    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == '/'
    }

    let text = &line[local..];
    let prefix = &line[..local];

    let mut start = local;
    for ch in prefix.chars().rev() {
        if is_word_char(ch) {
            start -= ch.len_utf8();
        } else {
            break;
        }
    }

    let mut end = local;
    for ch in text.chars() {
        if is_word_char(ch) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }

    if start == end
        && end < line.len()
        && let Some(ch) = line[end..].chars().next()
    {
        end += ch.len_utf8();
    }

    (
        ScreenPos::viewport(screen_row, start),
        ScreenPos::viewport(screen_row, end),
    )
}

/// Returns `(start, end)` screen positions spanning the entire row
/// `pos` is on, including the virtual newline byte (`byte = line.len() + 1`).
/// `vp_offset` is `session.viewport_row_offset()` as `u32`.
/// `pos` is resolved against `session` so scrollback-resident anchors
/// project to the current frame before slicing.
pub(crate) fn line_range_in_viewport(
    pos: ScreenPos,
    session: &TerminalSession,
    lines: &[String],
    vp_offset: u32,
) -> (ScreenPos, ScreenPos) {
    let Some((screen_row, _)) = pos.resolve(session) else {
        return (pos, pos);
    };
    let viewport_row = (screen_row.saturating_sub(vp_offset)) as usize;
    let line_len = lines.get(viewport_row).map(|l| l.len()).unwrap_or(0);
    (
        ScreenPos::viewport(screen_row, 0),
        // +1 to include the virtual newline so triple-click pastes with a
        // line break, matching the original line_range_in_viewport behaviour.
        ScreenPos::viewport(screen_row, line_len + 1),
    )
}

impl TerminalView {
    pub(super) fn feed_output_bytes_to_session(&mut self, bytes: &[u8]) {
        if let Some(input) = self.input.as_ref() {
            let _ = self
                .session
                .feed_with_pty_responses(bytes, |resp| input.send(resp));
        } else {
            let _ = self.session.feed(bytes);
        }
        self.apply_screen_change();
    }

    /// Responds to an alt-screen state transition signalled by the session
    /// parser:
    ///
    /// - **Entered** alt-screen: the session already injected `\033[3J` into
    ///   ghostty while it was still on the primary screen (before the buffer
    ///   switch). Refresh the viewport here to show the blank alt-screen
    ///   immediately.
    /// - **Exited** alt-screen: ghostty has restored the primary screen.
    ///   Inject `\033[3J\033[2J\033[H` to erase both the scrollback buffer
    ///   and the visible viewport so the shell prompt appears on a clean
    ///   slate.
    fn apply_screen_change(&mut self) {
        match self.session.take_screen_changed() {
            Some(true) => {
                // Entered alt-screen. ghostty has already switched and cleared
                // the alt-screen (dirty.clear = true). Dumping it now shows
                // blank content, preventing old primary-screen output from
                // bleeding through while the TUI draws its first frame.
                self.refresh_viewport();
            }
            Some(false) => {
                // Exited alt-screen. Distinct from the `pending_clear_scrollback`
                // path in session: that fires on explicit OSC 1337 ClearScrollback
                // (also \x1b[3J in some shells). This branch is the alt-screen-exit
                // erase-and-home that gives apps like vim/htop a clean primary
                // surface to return to.
                //
                // ghostty has restored the primary screen. Clear scrollback first,
                // then erase the visible viewport so old terminal history cannot
                // be scrolled back to.
                let _ = self.session.feed(crate::ansi::ERASE_SCROLLBACK);
                let _ = self.session.feed(crate::ansi::ERASE_DISPLAY_AND_HOME);
                self.state.pending_refresh = true;
                // Explicitly clear selection: any ScreenPos anchors held from
                // the primary screen are now invalid after the buffer switch.
                // pending_refresh_keep_selection must be false so the deferred
                // refresh calls refresh_viewport() (clears selection) rather
                // than refresh_viewport_preserving_selection().
                self.state.pending_refresh_keep_selection = false;
            }
            None => {}
        }
    }

    pub(super) fn sync_viewport_scroll_tracking(&mut self) {
        let _ = self.session.take_viewport_scroll_delta();
    }

    pub(super) fn reconcile_dirty_viewport_after_output(&mut self) {
        let delta = self.session.take_viewport_scroll_delta();
        let dirty = self.session.take_dirty_viewport_rows();
        let grid_events = self.session.take_grid_events();
        let viewport_height = self.session.rows();

        // iTerm2-style selection policy: selection survives partial dirty
        // repaints (it lives in absolute screen coordinates). Clear only on
        // full-viewport repaint, alt-screen toggle, or RIS.
        let reason = selection_policy::invalidation_reason(&dirty, viewport_height, &grid_events);
        if reason != InvalidationReason::None {
            self.state.selection = None;
            if matches!(
                reason,
                InvalidationReason::AltScreenToggle | InvalidationReason::Ris,
            ) {
                self.state.viewport_lock.unlock();
            }
        }

        // Alacritty/iTerm2 convention: any viewport scroll (IND, RI, SU, SD)
        // conservatively invalidates the whole frame. Partial rotate + dirty
        // row merge is fragile because ghostty_vt's dirty indices may refer to
        // pre- or post-scroll coordinates, causing ghost lines on vi `o`/`O`
        // at the last line.
        if delta != 0 {
            self.refresh_viewport_preserving_selection();
            return;
        }

        if dirty.is_empty() {
            return;
        }

        // Heuristic: large dirty sets typically mean IL/DL or region scroll.
        // Fall back to a full refresh instead of per-row dump to avoid stale
        // cells below the cursor (vi insert-mode Enter case).
        let rows = viewport_height as usize;
        if rows > 0 && dirty.len() * 2 >= rows {
            self.refresh_viewport_preserving_selection();
            return;
        }

        if !self.apply_dirty_viewport_rows(&dirty) {
            self.state.pending_refresh = true;
        }
    }

    pub(super) fn with_refreshed_viewport(mut self) -> Self {
        self.refresh_viewport();
        self
    }

    /// Full viewport rebuild.  Clears the selection because the viewport
    /// content may have changed completely (resize, alt-screen toggle).
    pub(super) fn refresh_viewport(&mut self) {
        self.refresh_viewport_impl(true);
    }

    /// Full viewport rebuild that preserves a linear selection whose
    /// endpoints are in absolute screen coordinates.  Used when the
    /// viewport window shifts (user scroll, ghostty internal scroll)
    /// but the underlying content at the selected rows is unchanged.
    pub(super) fn refresh_viewport_preserving_selection(&mut self) {
        self.refresh_viewport_impl(false);
    }

    fn refresh_viewport_impl(&mut self, clear_selection: bool) {
        let viewport = self.session.dump_viewport().unwrap_or_default();
        let mut lines = split_viewport_lines(&viewport);
        // ghostty's dump_viewport returns only non-empty trailing rows in
        // compact form.  Pad to session.rows() so viewport_lines.len() always
        // equals session.rows() — the mismatch check in apply_dirty_viewport_rows
        // uses this invariant to detect resize vs. normal partial content.
        let expected_rows = self.session.rows() as usize;
        if lines.len() < expected_rows {
            lines.resize(expected_rows, String::new());
        }
        self.state.viewport_lines = lines;
        self.state.viewport_line_offsets =
            Self::compute_viewport_line_offsets(&self.state.viewport_lines);
        self.state.viewport_total_len =
            Self::compute_viewport_total_len(&self.state.viewport_lines);
        self.state.viewport_style_runs = (0..self.session.rows())
            .map(|row| {
                self.session
                    .dump_viewport_row_style_runs(ViewportRow::new(row))
                    .unwrap_or_default()
            })
            .collect();
        self.line_layouts.clear();
        self.line_layout_key = None;
        if clear_selection {
            self.state.selection = None;
        }
        if !self.state.search.query.is_empty() {
            self.recompute_search_matches_with(false);
        }
    }

    pub(crate) fn compute_viewport_line_offsets(lines: &[String]) -> Vec<usize> {
        let mut offsets = Vec::with_capacity(lines.len());
        let mut offset = 0usize;
        for line in lines {
            offsets.push(offset);
            offset = offset.saturating_add(line.len() + 1);
        }
        offsets
    }

    pub(crate) fn compute_viewport_total_len(lines: &[String]) -> usize {
        lines
            .iter()
            .fold(0usize, |acc, line| acc.saturating_add(line.len() + 1))
    }

    /// Copy the text covered by a linear selection.  Rows within the
    /// current viewport are read from `viewport_lines`; rows that have
    /// scrolled out of view are fetched via `session.dump_screen_row`.
    pub(super) fn viewport_slice_screen(&self, start: ScreenPos, end: ScreenPos) -> String {
        let Some((start_row, start_byte)) = start.resolve(&self.session) else {
            return String::new();
        };
        let Some((end_row, end_byte)) = end.resolve(&self.session) else {
            return String::new();
        };
        if (start_row, start_byte) == (end_row, end_byte) {
            return String::new();
        }
        let vp_offset = self.session.viewport_row_offset();
        let vp_rows = self.session.rows() as u32;
        let mut out = String::new();

        for screen_row in start_row..=end_row {
            let row_start_byte = if screen_row == start_row {
                start_byte
            } else {
                0
            };
            // End byte for this row: for intermediate rows use line.len()+1 so
            // the virtual newline is included in the copy payload.
            let row_end_byte = if screen_row == end_row {
                end_byte
            } else {
                usize::MAX
            };

            let line_opt = if screen_row >= vp_offset && screen_row < vp_offset + vp_rows {
                let vp_row = (screen_row - vp_offset) as usize;
                self.state.viewport_lines.get(vp_row).cloned()
            } else {
                // Row is in scrollback outside the current viewport window.
                self.session.dump_screen_row(screen_row).ok()
            };

            let Some(line) = line_opt else { continue };
            let line = line.strip_suffix('\n').unwrap_or(&line).to_owned();

            let seg_start = row_start_byte.min(line.len());
            let text_end = row_end_byte.min(line.len());
            let includes_newline = row_end_byte > line.len();

            if seg_start < text_end {
                out.push_str(&line[seg_start..text_end]);
            }
            if includes_newline {
                out.push('\n');
            }
        }
        out
    }

    pub(super) fn apply_dirty_viewport_rows(&mut self, dirty_rows: &[u16]) -> bool {
        if dirty_rows.is_empty() {
            return false;
        }

        let expected_rows = self.session.rows() as usize;
        if self.state.viewport_lines.len() != expected_rows {
            // Row count changed (resize) — full refresh + clear selection.
            self.refresh_viewport();
            return true;
        }
        if self.state.viewport_style_runs.len() != expected_rows {
            self.refresh_viewport();
            return true;
        }

        for &row in dirty_rows {
            let row = row as usize;
            if row >= self.state.viewport_lines.len() {
                continue;
            }

            let line = match self.session.dump_viewport_row(ViewportRow::new(row as u16)) {
                Ok(s) => s,
                Err(_) => {
                    self.refresh_viewport();
                    return true;
                }
            };

            let line = line.strip_suffix('\n').unwrap_or(line.as_str());
            self.state.viewport_lines[row].clear();
            self.state.viewport_lines[row].push_str(line);
            self.state.viewport_style_runs[row] = self
                .session
                .dump_viewport_row_style_runs(ViewportRow::new(row as u16))
                .unwrap_or_default();
            if row < self.line_layouts.len() {
                self.line_layouts[row] = None;
            }
        }

        self.state.viewport_line_offsets =
            Self::compute_viewport_line_offsets(&self.state.viewport_lines);
        self.state.viewport_total_len =
            Self::compute_viewport_total_len(&self.state.viewport_lines);

        // Selection invalidation is decided centrally in
        // `reconcile_dirty_viewport_after_output` via the iTerm2 policy
        // (full-viewport / alt-screen / RIS). Partial dirty preserves the
        // selection here — its absolute screen coordinates remain valid.

        if !self.state.search.query.is_empty() {
            self.recompute_search_matches_with(false);
        }
        true
    }

    pub(super) fn schedule_viewport_refresh(&mut self, cx: &mut Context<Self>) {
        self.state.focused_prompt_row = None;
        self.state.focused_command_row = None;
        self.state.pending_refresh = true;
        // Selection uses absolute ScreenPos coordinates — it survives a
        // viewport-window shift (scroll, search navigation).
        self.state.pending_refresh_keep_selection = true;
        cx.notify();
    }

    /// Lock the viewport to the current top abs line and schedule a repaint.
    /// Call this instead of `schedule_viewport_refresh` whenever a scroll
    /// action should hold the reading position against PTY output.
    pub(super) fn lock_viewport_and_refresh(&mut self, cx: &mut Context<Self>) {
        self.state
            .viewport_lock
            .lock(self.session.viewport_top_abs_y());
        self.schedule_viewport_refresh(cx);
    }

    /// Snap the viewport to the bottom when PTY output arrives.
    ///
    /// Skipped when:
    /// - A search anchor is active (`search_overlay` or non-empty query).
    /// - The viewport is locked (`viewport_lock` is `Pinned`).
    ///   The lock is cleared by `check_prompt_arrived()` when OSC 133 A
    ///   signals that a new shell prompt has appeared.
    ///
    /// When locked, restores the viewport to the anchored absolute line
    /// so grid scrolls (IND / SU) do not drift the reading position.
    pub(super) fn maybe_scroll_to_bottom_on_output(&mut self) {
        if self.state.search_overlay || !self.state.search.query.is_empty() {
            return;
        }
        if self.state.viewport_lock.is_locked() {
            self.restore_pinned_viewport();
            // If restore unlocked (anchor evicted), fall through to the
            // bottom-snap check so the viewport follows output again.
            if self.state.viewport_lock.is_locked() {
                return;
            }
        }
        let offset = self.session.viewport_row_offset();
        let rows = self.session.rows() as u32;
        let total = self.session.total_rows();
        if offset + rows < total {
            let _ = self.session.scroll_viewport_bottom();
            self.sync_viewport_scroll_tracking();
        }
    }

    /// If the viewport is locked to an absolute line, scroll to keep that
    /// line at the viewport top. Handles grid scrolls (IND / SU) that push
    /// `viewport_row_offset` without changing `scroll_offset`.
    fn restore_pinned_viewport(&mut self) {
        let Some(anchor) = self.state.viewport_lock.anchor() else {
            // Live state — nothing to restore.
            return;
        };
        let Some(screen_row) = self.session.abs_to_screen_row(anchor) else {
            // Anchor evicted from LineBuffer — unlock so future PTY
            // output snaps to bottom again.
            self.state.viewport_lock.unlock();
            return;
        };
        let current = self.session.viewport_row_offset();
        if current == screen_row {
            return;
        }
        let delta = screen_row as i32 - current as i32;
        let _ = self.session.scroll_viewport(delta);
        self.sync_viewport_scroll_tracking();
    }

    /// Unlock the viewport and snap to the bottom of the scrollback.
    /// Shared by `check_prompt_arrived` (OSC 133 A), `on_scroll_to_bottom`
    /// (Cmd+End), and all PTY input paths.
    pub(super) fn snap_to_bottom(&mut self) {
        self.state.viewport_lock.unlock();
        let _ = self.session.scroll_viewport_bottom();
        self.sync_viewport_scroll_tracking();
    }

    /// `snap_to_bottom` + set the momentum-scroll suppression window.
    /// Call this from every PTY-input path (keyboard, IME, paste) so
    /// trackpad inertia from a prior scroll-then-type gesture cannot
    /// re-lock the viewport immediately after the input snaps it back.
    pub(super) fn snap_to_bottom_on_pty_input(&mut self) {
        self.snap_to_bottom();
        self.state.suppress_scroll_lock_until = Some(
            std::time::Instant::now() + std::time::Duration::from_millis(300),
        );
    }

    /// Check whether a PromptStart (OSC 133 A) has arrived.  If so, unlock
    /// the viewport and snap to bottom so the new shell prompt is always
    /// visible after a command completes.  Search anchors are still
    /// respected.
    pub(super) fn check_prompt_arrived(&mut self) {
        if !self.session.take_prompt_arrived() {
            return;
        }
        if self.state.search_overlay || !self.state.search.query.is_empty() {
            return;
        }
        self.snap_to_bottom();
    }

    pub(super) fn apply_side_effects(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.session.take_clipboard_write() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        if let Some(kind) = self.session.take_attention_request() {
            cx.emit(super::TerminalViewEvent::AttentionRequested(kind));
        }
        if let Some(req) = self.session.take_notification_request() {
            cx.emit(super::TerminalViewEvent::NotificationRequested(req));
        }
        if let Some(elapsed) = self.session.take_finished_command_elapsed() {
            cx.emit(super::TerminalViewEvent::CommandFinishedAfter { elapsed });
        }
    }
}
