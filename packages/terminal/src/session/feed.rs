//! Feeding PTY bytes into ghostty and mirroring scrolled-out rows into the
//! line buffer.

use super::*;

impl TerminalSession {
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.feed_internal(bytes, |_| {})
    }

    pub fn feed_with_pty_responses(
        &mut self,
        bytes: &[u8],
        send: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        self.feed_internal(bytes, send)
    }

    fn feed_internal(&mut self, bytes: &[u8], send: impl FnMut(&[u8])) -> Result<(), Error> {
        let was_alt = self.alt_screen;
        self.update_state_from_output(bytes);
        let entering_alt = !was_alt && self.alt_screen;
        if entering_alt {
            // ghostty_vt is still on the primary screen here — \x1b[3J
            // clears primary-screen scrollback before feed_incremental()
            // processes \x1b[?1049h and switches buffers.
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        let result = self.feed_incremental(bytes, send);
        if std::mem::take(&mut self.pending_clear_scrollback) {
            self.apply_clear_scrollback();
        }
        if entering_alt {
            self.on_enter_alt_screen();
        } else {
            self.sync_after_ghostty_scrollback_shrink();
            self.capture_scrolled_out();
        }
        result
    }

    /// Consolidated handler for the `OSC 1337 ; ClearScrollback`
    /// post-feed wipe. Drains after `feed_incremental` so the chunk's
    /// own pre-clear output (cursor home, status line redraw, …) is
    /// still in the visible viewport when `\x1b[3J` wipes the rows
    /// above it.
    ///
    /// LineBuffer mirrors ghostty's scrollback; clearing one without the
    /// other would desync the dispatcher and leave `last_captured_lb_abs`
    /// pointing past the (now-reset) `line_buffer.next_append_abs()`,
    /// silently breaking every subsequent capture for the rest of the
    /// session.
    fn apply_clear_scrollback(&mut self) {
        // SILENT-OK: ERASE_SCROLLBACK is a fixed constant sequence; a feed
        // error on a known-valid byte sequence indicates a terminal in a
        // broken state that will surface through subsequent feeds with
        // proper error reporting.
        let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        self.clear_line_buffer_and_drop_history_marks();
    }

    /// Snapshot every row that has scrolled off the active area since the
    /// last call into `line_buffer`. Wrap-continuations (DECAWM soft-wrap)
    /// merge into the same logical line so the scrollback stores joined text
    /// irrespective of the original physical row width.
    ///
    /// The newly-scrolled row count comes from `Terminal::take_scrolled_rows`
    /// (a monotonic tracked-pin delta), so capture keeps advancing even after
    /// ghostty's bounded scrollback ring saturates; `LineBuffer` grows to its
    /// own `max_scrollback`. `last_captured_lb_abs` records the LineBuffer
    /// abs index a fresh append would claim *after* this capture; it acts
    /// as the bounded marker `sync_after_ghostty_scrollback_shrink`
    /// inspects to detect a `\x1b[3J` / RIS wipe.
    fn capture_scrolled_out(&mut self) {
        // Capture is meaningless while ghostty is rendering into the
        // alt-screen buffer — those rows are by definition transient
        // UI, not scrollback content. We only sample primary-screen
        // rows as they scroll off the top.
        if self.alt_screen {
            return;
        }
        let viewport_top = self.terminal.viewport_row_offset();
        // Rows that scrolled off the active area since the previous capture.
        // Backed by a tracked pin in ghostty, so this stays accurate after the
        // transient ring saturates — where `viewport_row_offset` plateaus and
        // the old `last_captured + 1` cursor froze, capping real scrollback at
        // the ring size instead of `max_scrollback`.
        let scrolled = self.terminal.take_scrolled_rows();
        let start = viewport_top.saturating_sub(scrolled);
        if start >= viewport_top {
            return;
        }
        // Append the scrolled-out rows. The shared helper anchors the loop
        // in LineBuffer abs space so a producer-side glitch — phantom
        // `take_scrolled_rows` spike, stale tracked pin — is caught at the
        // first mismatched row instead of silently re-appending.
        self.append_ghostty_rows(start..viewport_top, "session.capture.desync");
        self.last_captured_lb_abs = Some(self.line_buffer.next_append_abs());
        // Rebind viewport-resident marks whose row has just been
        // captured into LineBuffer. A mark registered with
        // `LineCoord::Viewport { abs_y }` while the row was live must
        // be rewritten to `LineCoord::Buffered` once the row enters
        // scrollback — otherwise the `Ord` rule (Buffered < Viewport)
        // makes every subsequent range query miss it. The rebind math
        // lives on `LineBuffer::rebind_viewport_abs`; this loop just
        // walks single-line marks and forwards.
        {
            let cols = self.dims.cols;
            // Collect first: `iter()` borrows `&self`, `update_payload_range`
            // needs `&mut self`.
            let to_rebind: Vec<(MarkId, LineRange)> = self
                .interval_tree
                .iter()
                .filter_map(|m| match (m.range.start, m.range.end) {
                    // Marks are single-line today (start == end). Multi-line
                    // marks are out of scope.
                    (LineCoord::Viewport { abs_y: s }, LineCoord::Viewport { abs_y: e })
                        if s == e =>
                    {
                        let pos = self.line_buffer.rebind_viewport_abs(s, cols)?;
                        let new_coord = LineCoord::Buffered(pos);
                        Some((m.id, LineRange::new(new_coord, new_coord)))
                    }
                    _ => None,
                })
                .collect();

            for (id, new_range) in to_rebind {
                self.interval_tree.update_payload_range(id, new_range);
            }
        }
        // Drop interval-tree marks whose range fell off the bottom of
        // the live scrollback window. `min_position` returns `None` on
        // an empty buffer (nothing to evict against).
        if let Some(min) = self.line_buffer.min_position() {
            self.interval_tree.evict_below(LineCoord::Buffered(min));
        }
    }

    /// Detect a `\x1b[3J` / RIS ghostty-side scrollback wipe and mirror it
    /// into `line_buffer`. We don't parse the CSI ED directly — we infer the
    /// wipe from ghostty's scrollback depth (`viewport_row_offset`) dropping
    /// to zero while the active grid keeps content. A clear is the only thing
    /// that empties scrollback; a routine page-prune (ghostty drops whole
    /// pages, not single rows, so the depth sawtooths by ~one page) never
    /// reaches zero, so gating on zero avoids the false wipe that otherwise
    /// capped real scrollback at the ring/page size.
    ///
    /// The `last_captured_lb_abs` guard skips the check right after a reset
    /// (alt-screen entry / explicit clear set it to `None`): on the matching
    /// alt-screen exit the primary scrollback is legitimately empty, and we
    /// must not mistake that for a wipe of the preserved `line_buffer`.
    fn sync_after_ghostty_scrollback_shrink(&mut self) {
        if self.last_captured_lb_abs.is_none() {
            return;
        }
        if self.terminal.viewport_row_offset() == 0 && !self.line_buffer.is_empty() {
            self.clear_line_buffer_and_drop_history_marks();
        }
    }

    /// Shared post-wipe bookkeeping: clear `LineBuffer`, reset the
    /// capture cursor and scroll offset, and drop any prompt marks
    /// whose `abs_y` addressed a line inside the wiped history.
    ///
    /// `PromptMark.abs_y` is logical-line abs. `LineBuffer::clear`
    /// bumps `overflow` by `lines.len()`, so every wiped logical line
    /// is absorbed into `overflow` automatically — surviving marks
    /// above the wipe boundary stay valid against the new overflow
    /// without any shift. The wipe is line-symmetric: line-granular
    /// state in, line-granular state out, no visual-row correction
    /// needed (and applying one would over-shift on wrap-inflated
    /// buffers).
    ///
    /// The retention floor is `overflow + lines.len()` — the post-LB
    /// logical boundary just before the wipe. Marks at or above it
    /// were viewport-resident (or beyond the LB tail) and survive;
    /// marks below it pointed into the wiped logical lines and are
    /// dropped to avoid aliasing onto unrelated rows.
    fn clear_line_buffer_and_drop_history_marks(&mut self) {
        let logical_top =
            LogicalLineAbs(self.line_buffer.overflow() + self.line_buffer.len() as u64);
        self.line_buffer.clear();
        self.last_captured_lb_abs = None;
        // `scroll_offset` indexes into the now-empty `line_buffer`;
        // leaving it set would pin the viewport above the live grid.
        self.scroll_offset = 0;
        self.prompt_marks.retain(|m| m.abs_y >= logical_top);
    }

    /// Alt-screen entry handler. Seals any partial tail (the next time
    /// we re-enter primary screen and start capturing, a soft tail
    /// shouldn't grow into an unrelated row) and resets the capture
    /// cursor — ghostty's screen-row indices are no longer meaningful
    /// once the buffer is switched out.
    ///
    /// Asymmetric by design: entry seals + resets, but exit
    /// (alt → primary) has no counterpart. When the primary screen
    /// is restored, ghostty repaints it from row 0; the next
    /// `capture_scrolled_out` pass naturally re-anchors with
    /// `last_captured_lb_abs == None`, so there's nothing to reset.
    fn on_enter_alt_screen(&mut self) {
        // Seal a partial tail so a re-entry into primary screen does
        // not append into a now-stale soft-wrap continuation. We do
        // NOT clear `line_buffer` — daruda's persistent scrollback
        // survives alt-screen cycles (iTerm2 / Alacritty parity).
        // Ghostty's transient ring is wiped via the `\x1b[3J`
        // injection in `feed`, but that's a separate buffer used
        // only as a capture window.
        self.line_buffer.seal_partial();
        self.last_captured_lb_abs = None;
        // Alt-screen apps own the full grid; any user-initiated scroll
        // into history made before the switch is no longer meaningful.
        self.scroll_offset = 0;
    }

    /// Feed `bytes` to ghostty_vt in segments split at points where we
    /// need to observe or respond to state:
    ///   * DSR / OSC color queries — synthesize a response.
    ///   * OSC 133 FTCS marks — capture cursor position *after* the
    ///     segment ending with the terminator so the captured `abs_y`
    ///     reflects the row the shell intended.
    fn feed_incremental(&mut self, bytes: &[u8], mut send: impl FnMut(&[u8])) -> Result<(), Error> {
        let mut seg_start = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            let dsr = self.dsr_state.advance(b);
            let osc_query = self.osc_query_state.advance(b);
            let osc133 = self.osc133_scanner.advance(b);
            let cap = self.capability_scanner.advance(b);
            let xtgettcap = self.xtgettcap_scanner.advance(b);
            if dsr.is_none()
                && osc_query.is_none()
                && osc133.is_none()
                && cap.is_none()
                && xtgettcap.is_none()
            {
                continue;
            }

            self.terminal.feed(&bytes[seg_start..=i])?;
            seg_start = i + 1;

            if let Some(query) = dsr {
                match query {
                    TerminalQuery::DeviceStatus => send(ansi::DSR_OK),
                    TerminalQuery::CursorPosition => {
                        let (col, row) = self.cursor_position().unwrap_or((1, 1));
                        send(ansi::cpr_reply(row, col).as_bytes());
                    }
                }
            }

            if let Some(query) = osc_query {
                let (ps, rgb) = match query {
                    OscQuery::ForegroundColor => {
                        let fg = self.config.default_fg;
                        (OSC_DEFAULT_FG, (fg.r, fg.g, fg.b))
                    }
                    OscQuery::BackgroundColor => {
                        let bg = self.config.default_bg;
                        (OSC_DEFAULT_BG, (bg.r, bg.g, bg.b))
                    }
                };
                send(ansi::osc_color_reply(ps, rgb).as_bytes());
            }

            if let Some((kind, exit_code)) = osc133 {
                if self.alt_screen {
                    // No known shell emits OSC 133 from inside an
                    // alt-screen application, but if one does, the
                    // mark's abs_y would be an unrooted baseline
                    // (`overflow + lb.len()` per `current_abs_y_at_cursor`'s
                    // alt-screen path) — disconnected from the
                    // primary-screen LineBuffer geometry. Surface
                    // the anomaly through the error pipeline so it
                    // shows up in NDJSON instead of silently
                    // poisoning a future jump target. The mark is
                    // still pushed (callers may want the seq
                    // identity); the abs_y reflects the unrooted
                    // state and `abs_to_screen_row` will resolve
                    // best-effort.
                    LogWriter::log(
                        ErrorReport::new(
                            "OSC 133 dispatched while on alt-screen — abs_y is unrooted",
                        )
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("session.osc133.alt_screen")
                        .build(),
                    );
                }
                // Seal any partial tail in the LineBuffer before stamping
                // the mark. Without this, the next `capture_scrolled_out`
                // could fold an uncaptured Hard-EOL row into the partial
                // tail (per `append_at_or_after`'s iTerm2-derived
                // `would_extend_partial` rule), producing a `len()`
                // delta smaller than the Hard-EOL count
                // `logical_lines_until_cursor` walked through — i.e.
                // `mark.abs_y` would point at a logical line the
                // LineBuffer never grew to, and `abs_to_screen_row`
                // would project it onto the wrong row. Sealing
                // collapses the partial-fold path so the round-trip
                // identity holds. Shells emit OSC 133 only after
                // finishing the previous command output, so any
                // surviving partial tail at this point is a
                // mid-output artifact — sealing it is the right
                // semantic (the prompt is on a fresh logical line).
                self.line_buffer.seal_partial();
                let cursor = self.terminal.cursor_position().unwrap_or((1, 1));
                let abs_y = self.current_abs_y_at_cursor();
                self.push_prompt_mark(PromptMarkInit {
                    kind,
                    abs_y,
                    screen_col: cursor.0,
                    exit_code,
                    timestamp: std::time::SystemTime::now(),
                });
            }

            if let Some(query) = cap {
                match query {
                    CapabilityQuery::PrimaryDa => send(ansi::PRIMARY_DA),
                    CapabilityQuery::SecondaryDa => send(ansi::SECONDARY_DA),
                    CapabilityQuery::TertiaryDa => send(ansi::TERTIARY_DA),
                    CapabilityQuery::XtVersion => send(ansi::XTVERSION),
                    CapabilityQuery::KittyKeyboard => send(ansi::KITTY_KEYBOARD_RESPONSE),
                    CapabilityQuery::Decrqm(ps) => {
                        let pm = self.decrqm_pm(ps);
                        send(ansi::decrqm_reply(ps, pm).as_bytes());
                    }
                }
            }

            if let Some(body) = xtgettcap {
                let resp = build_xtgettcap_response(&body);
                if !resp.is_empty() {
                    send(&resp);
                }
            }
        }

        if seg_start < bytes.len() {
            self.terminal.feed(&bytes[seg_start..])?;
        }

        Ok(())
    }
}
