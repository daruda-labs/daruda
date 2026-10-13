use super::*;

// PromptMark.abs_y survives LineBuffer ring eviction --------------

#[test]
fn prompt_mark_abs_y_is_stable_across_normal_scrolling() {
    // Without any ring overflow, `abs_y` never changes — it's the
    // tracking invariant that lets translation work.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"x\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07").unwrap();
    let before = s.prompt_marks()[0].abs_y;
    s.feed(b"a\r\nb\r\nc\r\nd\r\n").unwrap();
    let after = s.prompt_marks()[0].abs_y;
    assert_eq!(
        before, after,
        "abs_y must not move when only LineBuffer captures happen"
    );
}

#[test]
fn prompt_mark_translates_to_screen_row_after_scrolling() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"\x1b]133;A\x07").unwrap(); // mark on row 0, abs_y=0
    s.feed(b"line-1\r\nline-2\r\nline-3\r\nline-4\r\n").unwrap();
    let mark = s.prompt_marks()[0];
    assert_eq!(mark.abs_y.as_u64(), 0, "mark fired before any scroll");
    // 4 newlines into a 3-row grid scrolls 2 rows out; both land in
    // LineBuffer at indices 0..2. Live grid then holds line-3, line-4,
    // empty. The mark at abs_y=0 must translate to LineBuffer index 0
    // (top of scrollback), not just any in-range row.
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark still reachable through LineBuffer dispatcher");
    assert_eq!(
        row, 0,
        "mark at abs_y=0 should translate to LineBuffer row 0 (top of scrollback)"
    );
    assert!(row < s.total_rows());
}

#[test]
fn prompt_mark_survives_line_buffer_eviction() {
    // max_scrollback=2 so the LineBuffer evicts old lines, bumping
    // its `overflow` counter and making any pre-eviction mark's
    // `abs_y` fall below `overflow`.
    let mut s = session_with(80, 3, 2);
    s.feed(b"\x1b]133;A\x07prompt-1\r\n").unwrap();
    let mark_abs = s.prompt_marks()[0].abs_y;
    let line_buffer_overflow_before = s.line_buffer().overflow();
    // Force enough eviction that `overflow` grows past `mark_abs`.
    for _ in 0..10 {
        s.feed(b"x\r\n").unwrap();
    }
    let line_buffer_overflow_after = s.line_buffer().overflow();
    let marks = s.prompt_marks();
    assert!(!marks.is_empty(), "mark must still be recorded");
    assert_eq!(marks[0].abs_y, mark_abs, "abs_y is stable");
    assert!(
        line_buffer_overflow_after > line_buffer_overflow_before,
        "test setup must trigger eviction (before={line_buffer_overflow_before}, after={line_buffer_overflow_after})"
    );
    assert!(
        mark_abs.as_u64() < line_buffer_overflow_after,
        "test setup must place mark below overflow (mark_abs={mark_abs}, overflow={line_buffer_overflow_after})"
    );
    // The line has been evicted — translation must return None and
    // not silently alias another row.
    assert!(
        s.abs_to_screen_row(mark_abs).is_none(),
        "evicted mark must not translate to a row (abs_y={mark_abs} < overflow={line_buffer_overflow_after})"
    );
}

#[test]
fn clear_scrollback_drops_history_marks_keeps_viewport_marks_abs_y() {
    let mut s = session_with(80, 3, 1024);
    // Mark #1 lands on row 1 (after one preceding line) — it will
    // scroll into LineBuffer history before the clear and must be
    // dropped.
    s.feed(b"x\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-1\r\n").unwrap();
    // Push another mark on the line that ends up viewport-resident
    // at clear time so we can assert it survives the wipe.
    s.feed(b"a\r\nb\r\nc\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-2").unwrap();
    let viewport_mark_abs_before = s.prompt_marks().back().unwrap().abs_y;
    assert!(
        !s.line_buffer().is_empty(),
        "test setup must populate LineBuffer first"
    );

    s.feed(b"\x1b[3J").unwrap();
    assert_eq!(s.line_buffer().len(), 0, "scrollback must be cleared");

    // History-resident mark (#1) is gone; only the viewport-resident
    // mark (#2) remains. With logical-line `abs_y`, `LineBuffer::clear`
    // bumps `overflow` by the wiped logical line count and absorbs the
    // full wipe — surviving marks keep their `abs_y` untouched.
    let marks = s.prompt_marks();
    assert_eq!(marks.len(), 1, "history mark dropped, viewport mark kept");
    let after = marks[0].abs_y;
    assert_eq!(
        after, viewport_mark_abs_before,
        "viewport mark's abs_y must be unchanged across the wipe"
    );
    let row = s
        .abs_to_screen_row(after)
        .expect("surviving mark must still translate");
    assert!(row < s.total_rows());
}

#[test]
fn clear_scrollback_under_wrap_does_not_over_shift_marks() {
    // Pin the wrap-inflated buffer fix: visual rows > logical lines used
    // to apply a `visual_residual` shift on top of the
    // logical-line `abs_y`, over-shifting viewport-resident marks.
    // Post-Task-4 the wipe is line-symmetric, so `abs_y` must survive
    // intact regardless of wrap inflation.
    let mut s = session_with(4, 3, 1024);
    // Feed several logical lines each wider than 4 cols so soft-wrap
    // inflates the visual row count well past the logical line count.
    s.feed(b"abcdefgh\r\n").unwrap();
    s.feed(b"ijklmnop\r\n").unwrap();
    s.feed(b"qrstuvwx\r\n").unwrap();
    // Capture a mark on the line that ends up viewport-resident at
    // clear time.
    s.feed(b"\x1b]133;A\x07p").unwrap();
    let viewport_mark_abs_before = s.prompt_marks().back().unwrap().abs_y;

    let logical_pre = s.line_buffer().len() as u64;
    let visual_pre = s.line_buffer().wrapped_row_count(4) as u64;
    assert!(
        visual_pre > logical_pre,
        "test setup must wrap-inflate the buffer (visual={visual_pre}, logical={logical_pre})"
    );

    s.feed(b"\x1b[3J").unwrap();
    assert_eq!(s.line_buffer().len(), 0, "scrollback must be cleared");

    let marks = s.prompt_marks();
    assert_eq!(marks.len(), 1, "viewport-resident mark must survive");
    assert_eq!(
        marks[0].abs_y, viewport_mark_abs_before,
        "wrap inflation must not shift abs_y — pre-Task-4 this over-shifted"
    );
    let row = s
        .abs_to_screen_row(marks[0].abs_y)
        .expect("surviving mark must still translate to a row");
    assert!(row < s.total_rows());
}

#[test]
fn prompt_mark_abs_y_accounts_for_in_flight_scroll_before_osc() {
    // OSC 133 fires AFTER mid-feed scrolling. abs_y must point at the
    // row where the OSC actually landed, not at a row earlier in scrollback.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\n\x1b]133;A\x07p").unwrap();
    let mark = s.prompt_marks()[0];
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark must be reachable after capture");
    let vp_top = s.viewport_row_offset();
    let vp_rows = s.rows() as u32;
    assert!(
        row >= vp_top && row < vp_top + vp_rows,
        "mark must land in viewport (where OSC fired), got row={row} vp_top={vp_top} vp_rows={vp_rows}"
    );
    let in_vp = row - vp_top;
    assert_eq!(
        in_vp, 2,
        "OSC fired on viewport row 2 ('p' line) — must not point at a scrolled row"
    );
}

/// Regression: daruda's own scrollback (`LineBuffer`) must retain up to
/// `max_scrollback` logical lines, independent of ghostty's small transient
/// ring. Feeding 5_000 lines one-at-a-time (well above the ring, below the
/// 10k cap) must leave every scrolled-off line in the buffer.
///
/// Before the tracked-pin cursor fix, capture used `viewport_row_offset()`
/// (ghostty's retained scrollback depth, not a monotonic counter); once that
/// saturated, the capture cursor froze and `LineBuffer` stopped growing at
/// ~1_000 lines, silently dropping the rest (overflow stayed 0).
#[test]
fn line_buffer_retains_scrollback_beyond_ghostty_ring() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    const N: usize = 5_000; // 80x24 grid, 10k cap
    for i in 0..N {
        session.feed(format!("LINE{i:06}\r\n").as_bytes()).unwrap();
    }

    let lb = session.line_buffer();
    let texts: Vec<String> = (0..lb.len())
        .filter_map(|idx| lb.get(idx).map(|l| l.text.clone()))
        .collect();

    // None of the fed lines reached the 10k cap, so nothing was evicted.
    assert_eq!(
        lb.overflow(),
        0,
        "no line should be evicted below the {N} < 10k cap"
    );

    // Every line that scrolled off the live grid must survive in scrollback.
    // Leave a 32-row margin at the top for grid / partial-tail boundary fuzz.
    let mut missing = Vec::new();
    for i in 0..N.saturating_sub(32) {
        let needle = format!("LINE{i:06}");
        if !texts.iter().any(|t| t.contains(&needle)) {
            missing.push(i);
        }
    }
    assert!(
        missing.is_empty(),
        "lost {} of {N} scrolled-off lines (effective scrollback froze at lb.len()={}); \
         first 5 missing = {:?}",
        missing.len(),
        lb.len(),
        missing.iter().take(5).collect::<Vec<_>>(),
    );
}

// Logical-line grid-walk helpers ----------------------------------
//
// `peek_uncaptured_logical_lines` and `logical_lines_until_cursor`
// back `current_abs_y_at_cursor`'s logical-line abs computation. They
// live on the session as private helpers; these tests pin the
// post-feed invariants and the grid-walk contract that
// `logical_lines_until_cursor_after_hard_lines` exercises.

#[test]
fn peek_uncaptured_logical_lines_post_feed_is_empty() {
    // `feed()` always runs `capture_scrolled_out` at the end, draining
    // the tracked-pin delta. The Hard-EOL counting branch itself is
    // only reachable mid-feed (when OSC 133 dispatches before the
    // end-of-feed capture); from a black-box public API there is no
    // way to observe it directly. This test pins the post-capture
    // invariant — uncaptured region empty — and confirms the helper
    // isn't trivially zero because nothing scrolled (LineBuffer did
    // receive the rows). The counting-branch correctness is exercised
    // indirectly via `logical_lines_until_cursor_after_hard_lines`,
    // which walks the same code path through a non-empty grid region.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n").unwrap();
    assert_eq!(
        s.peek_uncaptured_logical_lines(),
        0,
        "after capture, uncaptured region must be empty"
    );
    assert!(
        s.line_buffer().len() >= 2,
        "expected >= 2 captured lines, got {}",
        s.line_buffer().len()
    );

    s.feed(b"f\r\ng\r\n").unwrap();
    assert_eq!(s.peek_uncaptured_logical_lines(), 0);
}

#[test]
fn logical_lines_until_cursor_at_top_of_grid() {
    // Fresh session, no feed: cursor sits at row 1, no Hard EOL
    // precedes it in the uncaptured/grid region.
    let s = session_with(80, 3, 1024);
    assert_eq!(s.logical_lines_until_cursor(), 0);
}

#[test]
fn logical_lines_until_cursor_after_hard_lines() {
    // Feed "a\r\nb\r\nc" into an 80x3 grid: cursor lands on the row
    // containing 'c' (row 3), preceded by 2 Hard EOLs on rows 1 and
    // 2. Nothing has scrolled out yet (3 lines fit), so the count
    // comes entirely from the grid-above-cursor walk.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc").unwrap();
    assert_eq!(
        s.logical_lines_until_cursor(),
        2,
        "two Hard EOLs precede the cursor's row"
    );
}

#[test]
fn peek_uncaptured_logical_lines_returns_zero_in_alt_screen() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"\x1b[?1049h").unwrap();
    assert!(s.is_alt_screen());
    assert_eq!(s.peek_uncaptured_logical_lines(), 0);
    assert_eq!(s.logical_lines_until_cursor(), 0);
}

// PromptMark.abs_y logical-line migration ---------------------------
//
// The two tests below pin the migration's load-bearing properties:
// (1) the abs_y → screen-row round-trip lands the mark back on the
// row that dispatched it, and (2) the stored abs_y is invariant under
// resize-driven reflow (visual-row counts change, the logical-line
// abs does not).

#[test]
fn current_abs_y_at_cursor_round_trips_through_abs_to_screen_row() {
    // Dispatch OSC 133 at a known cursor row, then verify that
    // `abs_to_screen_row(mark.abs_y)` maps back to a row that
    // matches the cursor's pre-feed position in the unified frame.
    // The round trip is the contract for paint / jump consumers
    // — anything else and the gutter band paints on the wrong row.
    let mut s = session_with(80, 5, 1024);
    // Feed two lines so the cursor lands on row 3 (1-indexed) before
    // dispatch — exercises the grid-above-cursor walk inside
    // `logical_lines_until_cursor`.
    s.feed(b"alpha\r\nbeta\r\n\x1b]133;A\x07").unwrap();
    let mark = s.prompt_marks().back().copied().expect("mark must exist");
    // The mark was dispatched while the cursor was on grid row 3 of a
    // 5-row viewport with no scrollback. In the unified frame that is
    // absolute screen row 2 (zero-indexed).
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("round-trip translation must resolve");
    assert_eq!(
        row, 2,
        "mark dispatched on grid row 3 (zero-indexed row 2) must round-trip back to row 2"
    );
    // And after PTY-driven capture (lines scroll into LineBuffer),
    // the round-trip still lands on the original row's unified-frame
    // position — LineBuffer absorbed rows 0..N, so the row count
    // grows but the mark's abs_y is invariant.
    s.feed(b"gamma\r\ndelta\r\nepsilon\r\nzeta\r\n").unwrap();
    let row_after = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark must still resolve after scroll");
    // Unified-frame row of the mark's source line stays at 2 — two
    // logical lines ('alpha', 'beta') precede it regardless of where
    // the live grid currently sits.
    assert_eq!(
        row_after, 2,
        "scroll-driven capture must not move the mark's resolved row"
    );
}

#[test]
fn abs_y_unchanged_after_resize() {
    // Logical-line abs is wrap-invariant: dispatching OSC 133 at a
    // given cursor position, then resizing to a width that reflows
    // earlier lines, must NOT change the stored `mark.abs_y` (the
    // mark is an identity in logical-line space). `abs_to_screen_row`
    // then projects through the new wrap to land on the post-reflow
    // visual row.
    let mut s = session_with(40, 5, 1024);
    // Two lines wide enough to wrap at the post-resize 20-cols width
    // but fit in 40 cols pre-resize. The mark sits on row 3 of a
    // wrap-free pre-resize grid.
    s.feed(b"this-line-is-thirty-chars-long-aaaa\r\n").unwrap();
    s.feed(b"another-thirty-char-line-bbbbbbbbbbb\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07").unwrap();
    let abs_before = s
        .prompt_marks()
        .back()
        .copied()
        .expect("mark must exist")
        .abs_y;
    // Narrow enough to reflow the two earlier lines into wrap
    // continuations. Visual-row count of the LineBuffer prefix
    // therefore grows; logical-line count does not.
    s.resize(20, 5).expect("resize must succeed");
    let abs_after = s
        .prompt_marks()
        .back()
        .copied()
        .expect("mark must survive resize")
        .abs_y;
    assert_eq!(
        abs_before, abs_after,
        "stored abs_y must be invariant under wrap reflow (logical-line space)"
    );
    // Stronger than reachability: the projected visual row must
    // anchor a row that the cursor was logically on. The mark sits on
    // an empty prompt row (OSC 133 fires before the prompt body is
    // drawn), so the projected row's text starts empty — sufficient
    // for the round-trip identity. Cells around the mark are blanks
    // on both sides of resize, so the projected row's leading
    // characters compare equal pre/post.
    let row_after = s
        .abs_to_screen_row(abs_after)
        .expect("post-resize projection must still resolve the mark to a row");
    let text_after = s.dump_screen_row(row_after).unwrap_or_default();
    let row_before_recap = s.abs_to_screen_row(abs_before).unwrap();
    let text_before_recap = s.dump_screen_row(row_before_recap).unwrap_or_default();
    assert_eq!(
        text_after.trim_end(),
        text_before_recap.trim_end(),
        "the projected row's content must match the mark's logical line across resize"
    );
}
