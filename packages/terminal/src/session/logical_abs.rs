//! Logical-line absolute coordinates: mapping between abs lines, screen
//! rows, and interval-tree marks.

use super::*;

impl TerminalSession {
    /// Total visible rows across the union (`line_buffer` scrollback +
    /// the live viewport). Stays in sync as captures land and rows
    /// scroll off the top of the grid.
    pub fn total_rows(&self) -> u32 {
        let cols = self.dims.cols;
        self.line_buffer
            .wrapped_row_count(cols)
            .saturating_add(self.dims.rows as u32)
    }

    /// Compute the ever-increasing absolute Y of the **logical line**
    /// containing the cursor right now. Captured when an OSC 133 mark
    /// fires so the mark survives `LineBuffer` ring eviction — translate
    /// back to a current-frame screen row via [`Self::abs_to_screen_row`].
    ///
    /// **Units are logical lines, not visual rows.** The value increments
    /// once per Hard-EOL line boundary, not once per wrapped visual row,
    /// so it is invariant under resize-driven reflow: re-flowing a single
    /// "echo foo" line from 3 wrapped rows to 1 changes its visual row
    /// position but not its logical-line abs. Mirrors iTerm2's
    /// `cumulativeScrollbackOverflow + numberOfScrollbackLines + cursorY`
    /// accounting, which is also wrap-blind at the abs layer (`LineBuffer`
    /// is iTerm2's `LineBlock` analogue; wrap-awareness lives in the
    /// projection back to screen rows).
    ///
    /// Formula: `line_buffer.overflow() + line_buffer.len() +
    /// logical_lines_until_cursor()`. The helper walks ghostty rows
    /// across both the uncaptured-scrolled region and the live grid up
    /// to (but not including) the cursor's row, counting Hard EOLs —
    /// so the call site does not need a separate `uncaptured_logical`
    /// term.
    ///
    /// **Alt-screen caveat.** `peek_scrolled_rows` and `row_wrap_kind`
    /// read ghostty's currently-active page list. While `self.alt_screen`
    /// is true that list is the alt-screen's, not the primary's that
    /// backs `line_buffer`. A mark dispatched in alt-screen would
    /// therefore mix two coordinate spaces and cannot resolve back to a
    /// primary-screen row. Returning the unrooted logical baseline
    /// `overflow + lb.len()` (no grid/uncaptured term) keeps the mark
    /// from poisoning the primary-screen translation path; the caller
    /// — `push_prompt_mark` — could additionally suppress the mark, but
    /// no shell we know of emits OSC 133 from inside an alt-screen app,
    /// so the surface area of this branch is purely defensive.
    pub(super) fn current_abs_y_at_cursor(&self) -> LogicalLineAbs {
        let base = self
            .line_buffer
            .overflow()
            .saturating_add(self.line_buffer.len() as u64);
        if self.alt_screen {
            // See doc: skip the grid/uncaptured term so the returned
            // value at least stays anchored at the primary-screen
            // logical baseline the caller will compare against.
            return LogicalLineAbs(base);
        }
        // `logical_lines_until_cursor` already walks the entire
        // post-LineBuffer region (uncaptured scrolled rows + grid above
        // the cursor) in one pass, counting Hard EOLs — so we add it
        // exactly once instead of summing a separate `uncaptured_logical`
        // term that would double-count the uncaptured Hard EOLs.
        LogicalLineAbs(base.saturating_add(self.logical_lines_until_cursor()))
    }

    /// Logical-line count among ghostty rows that scrolled out but
    /// haven't yet entered `LineBuffer`. Mirrors `peek_scrolled_rows`
    /// — non-consuming, reads the tracked-pin delta — but walks each
    /// row's `row_wrap_kind` and counts only Hard-terminated
    /// boundaries. Wrap-continuation (`Soft`) rows fold into the
    /// preceding logical line on capture, so they must not bump the
    /// logical-line count.
    ///
    /// Returns `0` in alt-screen — `peek_scrolled_rows` reflects the
    /// active page list, which is the alt-screen's there. Mixing that
    /// count with primary-screen `LineBuffer` abs would corrupt the
    /// translation; callers should additionally short-circuit on
    /// alt-screen.
    // Used only by tests pinning the post-feed invariant — production
    // `current_abs_y_at_cursor` and `abs_to_screen_row` walk the
    // uncaptured+grid region in one pass via `logical_lines_until_cursor`
    // / `abs_to_screen_row`'s grid branch, so this helper is intentionally
    // not on a production path. `#[cfg(test)]` excludes it from non-test
    // builds entirely rather than just silencing the warning.
    #[cfg(test)]
    pub(super) fn peek_uncaptured_logical_lines(&self) -> u64 {
        if self.alt_screen {
            return 0;
        }
        // ghostty screen-space row index (the input space `row_wrap_kind`
        // expects). Distinct from `self.viewport_row_offset()` (line ~1660)
        // which sits in the unified line_buffer+grid abs space — the two
        // are same-named on intentionally different receivers.
        let viewport_top = self.terminal.viewport_row_offset();
        let start = viewport_top.saturating_sub(self.terminal.peek_scrolled_rows());
        self.count_hard_eols_in_ghostty_range(start, viewport_top)
    }

    /// Logical-line offset of the cursor's row inside the
    /// post-LineBuffer frame (uncaptured scrolled rows + the live
    /// grid). Walks Hard EOLs from the top of the uncaptured region,
    /// across the grid up to but not including the cursor's own row,
    /// and returns the count. The cursor's own line is *not* counted
    /// — it is the current logical line — so a call site computes the
    /// cursor's logical abs as `line_buffer.overflow() +
    /// line_buffer.len() + helper_result`.
    ///
    /// Returns `0` when the cursor is at row 1 with no preceding Hard
    /// EOL in the uncaptured/grid region (cursor is still on the
    /// continuation of the LineBuffer's tail, or at the very top of a
    /// fresh logical line).
    ///
    /// Returns `0` in alt-screen for the same reasoning as
    /// [`Self::peek_uncaptured_logical_lines`].
    pub(super) fn logical_lines_until_cursor(&self) -> u64 {
        if self.alt_screen {
            return 0;
        }
        // ghostty screen-space — see note in
        // `peek_uncaptured_logical_lines` about the same-named methods
        // living in different coordinate systems.
        let viewport_top = self.terminal.viewport_row_offset();
        let uncaptured_start = viewport_top.saturating_sub(self.terminal.peek_scrolled_rows());
        // `cursor_position` returns a 1-indexed row inside the live
        // grid. The grid-above-cursor region is
        // `[viewport_top, viewport_top + cursor_row - 1)` — the
        // cursor's own row is excluded.
        //
        // A `None` here means ghostty has no cursor (transient VT state
        // during a reset, an alt-screen edge case the alt-screen guard
        // above might not have absorbed). Defaulting to row 1 is
        // conservative — the count walks only the uncaptured region and
        // skips the grid — but the silent fallback can hide a real VT
        // bug. Log it through the error pipeline so the dedup'd report
        // surfaces in NDJSON rather than swallowing it.
        let cursor_row_1 = match self.terminal.cursor_position() {
            Some((_, y)) => u32::from(y),
            None => {
                LogWriter::log(
                    ErrorReport::new("cursor_position unavailable during logical-line walk")
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("session.logical_lines.no_cursor")
                        .build(),
                );
                1
            }
        };
        // `saturating_add`: with `viewport_top` near `u32::MAX` (deep
        // long-lived session) the result clamps; the empty-range guard
        // inside `count_hard_eols_in_ghostty_range` returns 0 in that
        // degenerate case rather than wrapping.
        let cursor_abs = viewport_top.saturating_add(cursor_row_1.saturating_sub(1));
        self.count_hard_eols_in_ghostty_range(uncaptured_start, cursor_abs)
    }

    /// Walk ghostty screen-space rows in `[start, end)` and count those
    /// whose [`Terminal::row_wrap_kind`] is [`WrapKind::Hard`]. Empty or
    /// inverted ranges return 0. Shared by [`Self::peek_uncaptured_logical_lines`]
    /// and [`Self::logical_lines_until_cursor`] so the walk lives in
    /// exactly one place — duplicating it across the two callers would
    /// be an N-step sequence inviting silent divergence under future
    /// `WrapKind` additions.
    fn count_hard_eols_in_ghostty_range(&self, start: u32, end: u32) -> u64 {
        if start >= end {
            return 0;
        }
        (start..end)
            .filter(|&y| is_hard_eol(self.terminal.row_wrap_kind(y)))
            .count() as u64
    }

    /// Project a stored logical-line `abs_y` (captured at OSC 133
    /// dispatch via [`Self::current_abs_y_at_cursor`]) onto a
    /// current screen-frame row.
    ///
    /// **Input space:** logical-line abs. `abs_y - overflow` is the
    /// 0-indexed logical-line offset, **not** a visual-row count.
    ///
    /// **Output space:** the unified frame row in
    /// [`Self::dump_screen_row`] coordinates (0 = topmost scrollback
    /// row; `total_rows() - 1` = bottommost grid row).
    ///
    /// **Returns `None` when**
    /// - the line has been evicted from `LineBuffer` (`abs_y < overflow`)
    /// - the logical line sits past the live grid bottom (the caller
    ///   stamped a mark for a row that scrolled out before capture
    ///   reached it — should not happen for a stored OSC 133 mark, but
    ///   guarded to keep the projection total).
    ///
    /// Production callers (paint, jump) treat both `None` cases the
    /// same — skip the mark — so a single return value suffices. A
    /// caller that needs to distinguish the two should compare
    /// `abs_y` against [`LineBuffer::overflow`] directly *before*
    /// calling this method:
    ///
    /// ```text
    /// if abs_y < session.line_buffer().overflow() { /* evicted */ }
    /// else if let Some(row) = session.abs_to_screen_row(abs_y) { /* visible */ }
    /// else { /* past frame — extremely rare, debug-level concern */ }
    /// ```
    ///
    /// **Soft-wrap precision.** The returned row is always the *first*
    /// visual row of the target logical line. When the cursor was on a
    /// soft-wrap continuation row at OSC 133 dispatch, the captured
    /// `abs_y` identifies the logical line correctly but `abs_to_screen_row`
    /// resolves to that line's head row, not the continuation row the
    /// cursor was on. The gutter band painted on the head row is the
    /// intended UX (one band per logical prompt), so this is by design,
    /// not a defect — it mirrors iTerm2's mark gutter, which also
    /// anchors to the head visual row regardless of wrap.
    ///
    /// **Algorithm.** Two branches, inverse of `current_abs_y_at_cursor`'s
    /// `overflow + lb.len() + grid` decomposition (capture sums up to
    /// the cursor; projection walks down from a given abs to the row):
    ///
    /// 1. **LineBuffer branch** (`line_offset < lb.len()`): sum
    ///    `rows_at_width` for the prefix `lines[0..line_offset]`
    ///    (via [`LineBuffer::visual_rows_through`] — which encapsulates
    ///    the otherwise-private `rows_at_width`). That sum is the first
    ///    visual row of the target logical line within the unified frame.
    /// 2. **Grid branch** (`line_offset >= lb.len()`): walk ghostty
    ///    screen-space rows from the top of the uncaptured-scrolled
    ///    region forward, counting Hard EOLs to find the start of the
    ///    `(line_offset - lb.len())`-th logical line. The visual row in
    ///    the unified frame is `lb_visual + (y - frame_start)`.
    ///
    /// The two branches use distinct coordinate spaces — `LineBuffer`
    /// abs vs. ghostty screen-space rows — so the per-branch comments
    /// matter: do not collapse the math even when the lengths line up
    /// numerically in a wrap-free trace.
    pub fn abs_to_screen_row(&self, abs_y: LogicalLineAbs) -> Option<u32> {
        let abs_y = abs_y.as_u64();
        let overflow = self.line_buffer.overflow();
        if abs_y < overflow {
            // Line evicted — caller (e.g. paint loop, jump candidate
            // walker) must skip this mark rather than alias onto an
            // unrelated row.
            return None;
        }
        let line_offset = abs_y - overflow;
        let lb_lines = self.line_buffer.len() as u64;
        let cols = self.dims.cols;
        if line_offset < lb_lines {
            // LineBuffer branch: sum the wrapped row counts of every
            // logical line strictly before `idx` to land on the target's
            // first visual row. `visual_rows_through` mirrors
            // `wrapped_row_count`'s wrap walk so the two stay in sync as
            // `LogicalLine::rows_at_width` evolves.
            let idx = line_offset as usize;
            return Some(self.line_buffer.visual_rows_through(idx, cols));
        }
        // Grid branch — ghostty screen space (NOT LineBuffer abs).
        //
        // The post-LineBuffer region starts at `lb_visual` rows from the
        // top of the unified frame and contains both the uncaptured-
        // scrolled rows (a transient buffer mid-feed) and the live
        // grid. Walk it from `frame_start` looking for the `n`-th Hard
        // EOL boundary; once `count == n` the current `y` is the first
        // ghostty row of the target logical line.
        let n = line_offset - lb_lines;
        let lb_visual = self.line_buffer.wrapped_row_count(cols) as u64;
        let scrolled = self.terminal.peek_scrolled_rows();
        let viewport_top = self.terminal.viewport_row_offset();
        let frame_start = viewport_top.saturating_sub(scrolled);
        let frame_end = viewport_top.saturating_add(self.dims.rows as u32);
        let mut count: u64 = 0;
        let mut y = frame_start;
        while y < frame_end {
            if count == n {
                // The visual row of `y` in the unified frame is the
                // `lb_visual` LineBuffer-side rows plus the offset
                // walked through the post-LineBuffer region.
                let rel = (y - frame_start) as u64;
                return u32::try_from(lb_visual.saturating_add(rel)).ok();
            }
            if is_hard_eol(self.terminal.row_wrap_kind(y)) {
                count = count.saturating_add(1);
            }
            y = y.saturating_add(1);
        }
        // `line_offset` points past the live frame — the mark is in the
        // future (shouldn't normally happen for a stored OSC 133 mark,
        // but guarded so the projection is total).
        None
    }

    /// Inverse of [`Self::abs_to_screen_row`]: map a unified-frame row
    /// (`dump_screen_row` space) to the wrap-invariant logical-line abs of
    /// the line it sits on. Used at resize entry to convert a width-dependent
    /// `LineCoord::Viewport` anchor into a logical-line abs that survives the
    /// reflow, then re-project it via `abs_to_screen_row` at the new width.
    /// Returns `None` in alt-screen or when the row sits past the live frame.
    ///
    /// Mirrors `abs_to_screen_row`'s two branches inversely — keep the two in
    /// sync: the LineBuffer branch reads a buffer position whose `abs_index`
    /// *is* the logical-line abs; the grid branch counts Hard EOLs in ghostty
    /// screen space (NOT LineBuffer abs).
    ///
    /// NOTE: a soft-wrap *continuation* row maps to its logical line's abs
    /// (no Hard EOL fires before it), so a mark on a continuation row snaps to
    /// the line's head row after re-projection. This matches `abs_to_screen_row`'s
    /// head-row contract and the "one annotation box per logical line" goal.
    pub(super) fn screen_row_to_logical_abs(&self, unified_row: u32) -> Option<LogicalLineAbs> {
        let cols = self.dims.cols;
        let overflow = self.line_buffer.overflow();
        let lb_visual = self.line_buffer.wrapped_row_count(cols);
        if unified_row < lb_visual {
            let (pos, _, _) = self
                .line_buffer
                .position_for_visual_row(unified_row, cols)?;
            return Some(LogicalLineAbs(pos.abs_index));
        }
        if self.alt_screen {
            return None;
        }
        let scrolled = self.terminal.peek_scrolled_rows();
        let viewport_top = self.terminal.viewport_row_offset();
        let frame_start = viewport_top.saturating_sub(scrolled);
        let frame_end = viewport_top.saturating_add(self.dims.rows as u32);
        let target_y = frame_start.saturating_add(unified_row - lb_visual);
        if target_y >= frame_end {
            return None;
        }
        let base = overflow.saturating_add(self.line_buffer.len() as u64);
        let grid_logical = self.count_hard_eols_in_ghostty_range(frame_start, target_y);
        Some(LogicalLineAbs(base.saturating_add(grid_logical)))
    }

    /// Snapshot every single-line mark as its wrap-invariant logical-line
    /// abs, taken at the **old** width before a resize reflow moves lines
    /// across the LineBuffer↔grid seam (a narrow pushes grid lines into the
    /// buffer; a widen pulls buffer lines back into the grid). A `Viewport`
    /// coord (width-dependent visual-row) converts via
    /// `screen_row_to_logical_abs`; a `Buffered` coord on a real buffer line
    /// already holds the logical-line abs in its `abs_index`. Paired with
    /// [`Self::reproject_marks`], which re-anchors them once the buffer
    /// settles — capturing both sides is what lets a mark migrate
    /// Viewport↔Buffered in either resize direction without being lost.
    pub(super) fn snapshot_marks_logical_abs(&self) -> Vec<(MarkId, LogicalLineAbs)> {
        if self.alt_screen {
            return Vec::new();
        }
        let overflow = self.line_buffer.overflow();
        self.interval_tree
            .iter()
            .filter_map(|m| match (m.range.start, m.range.end) {
                _ if m.range.start != m.range.end => None,
                (LineCoord::Viewport { abs_y: s }, _) => {
                    let unified_row = u32::try_from(s.checked_sub(overflow)?).ok()?;
                    Some((m.id, self.screen_row_to_logical_abs(unified_row)?))
                }
                (LineCoord::Buffered(pos), _) => {
                    // Only snapshot marks on a real buffer line. A degenerate
                    // abs at/past the tail (or on an empty buffer) is not a
                    // live line and must not be reinterpreted as a grid row;
                    // leave it untouched.
                    let k = pos.abs_index.checked_sub(overflow)?;
                    (k < self.line_buffer.len() as u64)
                        .then_some((m.id, LogicalLineAbs(pos.abs_index)))
                }
            })
            .collect()
    }

    /// Re-project marks snapshotted by [`Self::snapshot_marks_logical_abs`]
    /// onto their post-reflow coords at the **new** width. A line now in the
    /// LineBuffer becomes `Buffered`; one now in the live grid becomes
    /// `Viewport` at its new visual row; one evicted or past the frame is
    /// dropped. Run after the LineBuffer is in its final post-resize state.
    pub(super) fn reproject_marks(&mut self, snapshot: Vec<(MarkId, LogicalLineAbs)>) {
        for (id, abs) in snapshot {
            match self
                .abs_to_screen_row(abs)
                .and_then(|row| self.screen_row_to_line_coord(row))
            {
                Some(coord) => {
                    // Skip the update when the coord is unchanged — a mark
                    // whose line did not cross the seam re-projects to its
                    // existing position, and a no-op `update_payload_range`
                    // would still churn the tree and emit a record.
                    let new_range = LineRange::new(coord, coord);
                    if self.interval_tree.range_of(id) != Some(new_range) {
                        self.interval_tree.update_payload_range(id, new_range);
                    }
                }
                None => {
                    self.interval_tree.remove_payload(id);
                }
            }
        }
    }
}
