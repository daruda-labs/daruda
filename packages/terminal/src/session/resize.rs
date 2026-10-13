//! Grid resize, including dedup of lines that re-enter the viewport on a
//! widen reflow.

use super::*;

impl TerminalSession {
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // Snapshot all marks as wrap-invariant logical-line abs at the OLD
        // width, before the reflow moves lines across the LineBuffer↔grid
        // seam. Re-projected at the new width once the LineBuffer settles
        // (see `reproject_marks` below).
        let marks_snapshot = self.snapshot_marks_logical_abs();
        self.dims.cols = cols;
        self.dims.rows = rows;
        let result = self.terminal.resize(cols, rows);
        // Widening un-wraps soft-wrapped rows: ghostty reflows its own pages
        // and pulls history rows back into the active viewport, but those
        // rows are still mirrored in the LineBuffer tail, so the unified
        // frame (LineBuffer rows ++ viewport rows) would render the overlap
        // twice — and re-capture it on the next scroll, leaving a permanent
        // duplicated block in scrollback. Pop the re-entered tail lines,
        // gated on content equality so nothing is lost. Must run before the
        // watermark re-anchor below so it anchors to the post-pop tail.
        if !self.alt_screen && !process_env::NO_SEAM_DEDUP.is_present() {
            self.pop_verified_reentered_lines();
        }
        // Ghostty reflows soft-wrapped rows on resize, so
        // `viewport_row_offset` may drop (widen) or rise (narrow) by an
        // unpredictable amount. Re-anchor the capture cursor against
        // the post-resize ghostty state so:
        //   1. `sync_after_ghostty_scrollback_shrink` does not interpret
        //      a widen-induced drop as a `\x1b[3J` scrollback wipe and
        //      clear LineBuffer.
        //   2. The next `capture_scrolled_out` does not re-walk rows
        //      we've already captured (LineBuffer keeps its contents
        //      as-is; ghostty's pre-viewport rows are treated as
        //      already accounted for).
        // Drain the Zig-side capture watermark so it re-anchors at the
        // post-reflow active-area top. Without this drain, reflow's shift
        // of the active row index would surface in the next
        // `take_scrolled_rows` call as a phantom scroll count, and
        // `capture_scrolled_out` would re-walk rows already in
        // LineBuffer. The return value is intentionally discarded — the
        // capture cursor we hand the rest of the session is the
        // LineBuffer's own `next_append_abs` below; this call exists
        // purely to reset the pin so subsequent peeks/takes see a clean
        // delta.
        let _ = self.terminal.take_scrolled_rows();
        // Re-anchor the LineBuffer-space watermark to the buffer's
        // current tail. The buffer's contents are unchanged by resize
        // (only the wrap cache was invalidated), so `next_append_abs`
        // continues to identify the same logical position; the value
        // lives entirely in LineBuffer abs, never ghostty's screen-row
        // space. An empty buffer drops back to `None` — the invariant
        // sustained everywhere else is "`Some` ⇔ a capture has reached
        // the buffer", and an empty buffer cannot meet that. Leaving an
        // older `Some` here would be the only path to a stale watermark.
        self.last_captured_lb_abs = if self.line_buffer.is_empty() {
            None
        } else {
            Some(self.line_buffer.next_append_abs())
        };
        // Invalidate LineBuffer wrap cache before querying at new width.
        // Width change → all per-line cached wraps become stale. Clearing
        // ensures wrapped_row_count below recalculates from cells without
        // per-line cache hits on potentially outdated (width, rows) pairs.
        self.line_buffer.invalidate_wrap_cache();
        // `wrapped_row_count` depends on the column count; clamp so a
        // narrower viewport doesn't leave `scroll_offset` past the new
        // top of the unified frame.
        let max_scroll = self.line_buffer.wrapped_row_count(cols);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
        // Drag any column-bound mark metadata down to the new width so
        // narrowing does not leave stale start/end column pairs pointing
        // past the right margin.
        self.interval_tree.clamp_payload_cols(cols);
        // Re-anchor marks snapshotted above: the LineBuffer and grid are now
        // in their final post-resize state, so each logical abs re-projects to
        // its correct new row/coord (Viewport↔Buffered as the line moved).
        self.reproject_marks(marks_snapshot);
        result
    }

    /// Content-verified dedup of complete logical lines that re-entered the
    /// live viewport after a widen reflow. ghostty reflows its own pages on
    /// resize, pulling history rows back into the active area, but the
    /// LineBuffer mirror keeps its pre-reflow copy of those rows; without
    /// this they render and re-capture twice (see the call site in
    /// [`Self::resize`]).
    ///
    /// Re-derive the ring's complete logical lines downward from the viewport
    /// top, then pop the longest run of LineBuffer tail lines that equals
    /// them in order (the newest buffered line pairs with the deepest matched
    /// ring line). Every pop is equality-gated — a mismatch stops the walk —
    /// so an unmatched residue is left duplicated but content is never lost.
    /// O(viewport rows); only on resize.
    fn pop_verified_reentered_lines(&mut self) {
        let v1 = self.terminal.viewport_row_offset();
        let end = v1.saturating_add(u32::from(self.dims.rows));
        // When the viewport top sits *mid-line* — the row just above it ended
        // Soft, so `v1` is a wrap continuation — the first run is the tail
        // fragment of a straddling line whose head is below the viewport
        // (still in the LineBuffer as a complete line). That fragment is not
        // a complete logical line; recording it would shift every subsequent
        // line by one and break the alignment with the LineBuffer's complete
        // tail. Skip it.
        let mut skip_fragment = v1 > 0 && !is_hard_eol(self.terminal.row_wrap_kind(v1 - 1));
        let mut ring_lines: Vec<String> = Vec::new();
        let mut y = v1;
        while y < end {
            let mut text = String::new();
            let mut complete = false;
            while y < end {
                text.push_str(&self.terminal.dump_screen_row(y).unwrap_or_default());
                let hard = is_hard_eol(self.terminal.row_wrap_kind(y));
                y = y.saturating_add(1);
                if hard {
                    complete = true;
                    break;
                }
            }
            if !complete {
                break;
            }
            if skip_fragment {
                skip_fragment = false;
                continue;
            }
            ring_lines.push(text);
        }
        // Longest `m` where the LineBuffer's last `m` lines equal the ring's
        // first `m` lines in order. A Hard line must match a complete ring
        // line exactly; the buffer's *newest* line may be a Soft/Dwc partial
        // (only a straddling line's head was captured before its tail
        // scrolled), so it matches when the ring's complete line *starts
        // with* the captured head.
        let lb_len = self.line_buffer.len();
        let max_m = ring_lines.len().min(lb_len);
        let mut m = 0usize;
        for cand in (1..=max_m).rev() {
            let matches = (0..cand).all(|i| {
                let is_newest = i == cand - 1;
                self.line_buffer.get(lb_len - cand + i).is_some_and(|l| {
                    // Trim trailing blanks on both sides: the LineBuffer text
                    // was joined from rows wrapped at the *old* width and the
                    // ring row is padded at the *new* width, so the trailing
                    // padding differs even for identical content.
                    let lt = l.text.trim_end();
                    let rt = ring_lines[i].trim_end();
                    match l.eol {
                        EolKind::Hard => lt == rt,
                        EolKind::Soft | EolKind::Dwc => {
                            is_newest && !lt.is_empty() && rt.starts_with(lt)
                        }
                    }
                })
            });
            if matches {
                m = cand;
                break;
            }
        }
        for _ in 0..m {
            self.pop_reentered_tail_line();
        }

        // Residual straddler heal. When the viewport top sits mid-line, the
        // logical line whose head is below the viewport is held *complete* in
        // the LineBuffer while the viewport re-shows its tail (the fragment
        // skipped above) — so that tail row renders twice. Re-seat the
        // LineBuffer's copy as head-only (Soft), re-derived from ghostty's
        // rows above the viewport, so `LineBuffer(head) + viewport(tail)`
        // render the straddler exactly once. Content-gated.
        if v1 > 0 && !is_hard_eol(self.terminal.row_wrap_kind(v1 - 1)) {
            let mut b = v1;
            while b > 0 && !is_hard_eol(self.terminal.row_wrap_kind(b - 1)) {
                b -= 1;
            }
            let head: String = (b..v1)
                .map(|y| self.terminal.dump_screen_row(y).unwrap_or_default())
                .collect();
            let head_trim = head.trim_end();
            let tail_holds_straddler = !head_trim.is_empty()
                && self
                    .line_buffer
                    .get(self.line_buffer.len().wrapping_sub(1))
                    .is_some_and(|l| {
                        l.eol == EolKind::Hard && l.text.trim_end().starts_with(head_trim)
                    });
            if tail_holds_straddler {
                self.pop_reentered_tail_line();
                self.append_ghostty_rows(b..v1, "session.resize.seam_heal");
            }
        }
    }

    /// Append ghostty screen rows `range` into the LineBuffer, folding
    /// soft-wrapped rows into one logical line. Anchors the loop in
    /// LineBuffer abs space (`append_at_or_after`) so a producer-side glitch
    /// is caught at the first mismatched row rather than silently
    /// re-appended; `site` tags the desync log. Shared by
    /// [`Self::capture_scrolled_out`] and the straddler heal in
    /// [`Self::pop_verified_reentered_lines`].
    pub(super) fn append_ghostty_rows(&mut self, range: std::ops::Range<u32>, site: &'static str) {
        let mut intended = self.line_buffer.next_append_abs();
        for y in range {
            // Empty text on dump failure (e.g., row evicted from ghostty
            // scrollback); preserves line numbering.
            let text = self.terminal.dump_screen_row(y).unwrap_or_default();
            let runs = self
                .terminal
                .dump_screen_row_style_runs(y)
                .unwrap_or_default();
            let url_ids = self.terminal.dump_screen_row_url_ids(y).unwrap_or_default();
            let eol = match self.terminal.row_wrap_kind(y) {
                ghostty_vt::WrapKind::Soft => EolKind::Soft,
                ghostty_vt::WrapKind::Hard => EolKind::Hard,
            };
            match self
                .line_buffer
                .append_at_or_after(intended, &text, &runs, eol)
            {
                Ok(next) => {
                    self.line_buffer.attach_url_ids_to_tail(&url_ids);
                    intended = next;
                }
                Err(err) => {
                    LogWriter::log(
                        ErrorReport::new("LineBuffer append refused")
                            .severity(ErrorSeverity::Warning)
                            .with_context("detail", err.to_string())
                            .at(file!(), line!())
                            .dedup(site)
                            .build(),
                    );
                    // Resync the local cursor to the buffer's own tail so
                    // subsequent rows don't fire the same error repeatedly.
                    intended = self.line_buffer.next_append_abs();
                }
            }
        }
    }

    /// Drop the newest LineBuffer line because it re-entered the live
    /// viewport on a widen reflow. Marks on it are **not** dropped here —
    /// `resize` snapshots every mark's logical abs before the reflow and
    /// `reproject_marks` re-anchors them (Buffered → Viewport for a line that
    /// re-entered the grid) once the buffer settles. Evicting here instead
    /// would silently detach an annotation on a re-entered line.
    fn pop_reentered_tail_line(&mut self) {
        if self.line_buffer.last_abs().is_some() {
            self.line_buffer.pop_tail();
        }
    }
}
