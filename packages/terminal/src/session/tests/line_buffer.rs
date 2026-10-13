use super::*;

#[test]
fn capture_appends_scrolled_out_rows() {
    let mut s = session_with(80, 3, 1024);
    // Feed five lines into a 3-row viewport so at least the first two
    // scroll off the top and land in the LineBuffer.
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n").unwrap();
    assert!(
        s.line_buffer().len() >= 2,
        "expected >= 2 captured lines, got {}",
        s.line_buffer().len()
    );
    assert!(
        s.line_buffer().get(0).unwrap().text.starts_with('a'),
        "first captured line should start with 'a', got {:?}",
        s.line_buffer().get(0).unwrap().text
    );
}

#[test]
fn live_edge_predicate_tracks_scroll_to_bottom() {
    // Guards the session-level predicate that `reanchor_viewport_lock`
    // (a GPUI-bound `TerminalView` method, not exercised here) feeds on:
    // `viewport_row_offset() + rows >= total_rows()`. The wheel/trackpad
    // path must release the viewport lock when the user scrolls back down
    // to the live edge (matching PageDown and the scrollbar) — otherwise a
    // long streaming run that emits no new OSC 133 prompt stays pinned and
    // never follows output again. The View-level unlock decision itself
    // needs a `Window` and is covered by manual smoke testing.
    let mut s = session_with(80, 5, 1024);
    // 20 lines into a 5-row viewport — ~15 land in the LineBuffer, so
    // there is real scrollback to walk back into.
    for i in 0..20 {
        s.feed(format!("line{i:02}\r\n").as_bytes()).unwrap();
    }
    let rows = s.rows() as u32;
    let total = s.total_rows();

    // At the bottom (scroll_offset == 0) the predicate reports the live
    // edge → reanchor unlocks.
    assert_eq!(s.scroll_offset(), 0);
    assert!(
        s.viewport_row_offset() + rows >= total,
        "fresh session must sit at the live edge"
    );

    // Scroll up into history → no longer at the live edge → reanchor locks.
    s.scroll_viewport(-3).unwrap();
    assert_eq!(s.scroll_offset(), 3);
    assert!(
        s.viewport_row_offset() + rows < total,
        "scrolled-back viewport must not report the live edge"
    );

    // Scroll back down to the bottom → live edge again → reanchor unlocks.
    s.scroll_viewport(3).unwrap();
    assert_eq!(s.scroll_offset(), 0);
    assert!(
        s.viewport_row_offset() + rows >= total,
        "scrolling back to the bottom must re-report the live edge"
    );
}

#[test]
fn capture_merges_soft_wrap_continuation() {
    let mut s = session_with(5, 3, 1024);
    // 10-char string DECAWM-wraps across two physical rows in a 5-col
    // viewport. After the rows scroll off, they should merge into one
    // logical line.
    s.feed(b"abcdefghij\r\nx\r\ny\r\nz\r\n").unwrap();
    assert!(
        !s.line_buffer().is_empty(),
        "expected >= 1 captured line, got {}",
        s.line_buffer().len()
    );
    let first = &s.line_buffer().get(0).unwrap().text;
    assert!(
        first.contains("abcdefghij") || first == "abcdefghij",
        "first captured line should join wrap-continuation, got {first:?}"
    );
    assert_eq!(s.line_buffer().get(0).unwrap().eol, EolKind::Hard);
}

#[test]
fn dump_screen_row_dispatches_between_line_buffer_and_viewport() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n").unwrap(); // 5 lines into 3-row viewport
    // total_rows should at least cover the live viewport.
    let total = s.total_rows();
    assert!(total >= 3, "total_rows must include viewport: {total}");
    // y=0 sits in LineBuffer (rows a, b scrolled out of the 3-row
    // viewport). Assert exact content so the dispatcher is actually
    // exercised — a stub returning empty strings would silently pass
    // a no-panic check.
    let row0 = s
        .dump_screen_row(0)
        .expect("row 0 must dispatch to LineBuffer");
    let row0_trimmed = row0.trim_end_matches([' ', '\n']);
    assert_eq!(
        row0_trimmed, "a",
        "row 0 should be the first scrolled-out line"
    );
    // Last visible row sits inside the live viewport via the dispatcher.
    let _ = s.dump_screen_row(total - 1).unwrap_or_default();
    let _ = s.dump_screen_row(total - 2).unwrap_or_default();
}

#[test]
fn dump_viewport_row_respects_scroll_offset() {
    // Regression: dump_viewport_row (TEXT) used to ignore scroll_offset and
    // always dump ghostty's live grid, unlike its siblings dump_viewport and
    // dump_viewport_row_style_runs. When the user scrolled up during streaming,
    // the dirty-row repaint fast-path (apply_dirty_viewport_rows) then painted
    // live-grid rows over the scrolled-back content — the Claude Code "input
    // box afterimage" overlay. dump_viewport_row must dispatch through the
    // unified frame like the other two dumps.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n").unwrap();
    let max_scroll = s.line_buffer().wrapped_row_count(80);
    assert!(max_scroll > 0, "expected captured scrollback rows");

    // Scroll to the very top of history; the viewport now shows the oldest
    // unified-frame rows, not the live grid.
    s.scroll_viewport(-(max_scroll as i32)).unwrap();
    assert_eq!(s.scroll_offset(), max_scroll);

    let top = s.viewport_row_offset();
    for r in 0..s.rows() {
        let via_row = s.dump_viewport_row(ViewportRow::new(r)).unwrap_or_default();
        let via_row = via_row.trim_end_matches([' ', '\n']);
        let via_frame = s.dump_screen_row(top + r as u32).unwrap_or_default();
        let via_frame = via_frame.trim_end_matches([' ', '\n']);
        assert_eq!(
            via_row,
            via_frame,
            "dump_viewport_row({r}) must match unified-frame row {} when \
             scroll_offset={}",
            top + r as u32,
            s.scroll_offset()
        );
    }

    // Concrete anchor: at the top of history, row 0 is the oldest scrolled-out
    // line ("a"), never a live-grid row.
    let row0 = s.dump_viewport_row(ViewportRow::new(0)).unwrap_or_default();
    assert_eq!(
        row0.trim_end_matches([' ', '\n']),
        "a",
        "top-of-history viewport row 0 should be the first scrolled-out line"
    );
}

#[test]
fn scroll_viewport_moves_offset_into_history() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n").unwrap();
    let max_scroll = s.line_buffer().wrapped_row_count(80);
    assert!(max_scroll > 0, "expected captured scrollback rows");
    // Pinned to bottom initially.
    assert_eq!(s.scroll_offset(), 0);
    // negative delta = scroll UP into history (spec sign convention).
    s.scroll_viewport(-1).unwrap();
    assert_eq!(s.scroll_offset(), 1, "one step up should land at offset 1");
    s.scroll_viewport(-100).unwrap();
    assert_eq!(
        s.scroll_offset(),
        max_scroll,
        "saturating scroll up should clamp to max_scroll"
    );
    // Scroll back down past the live grid clamps to 0.
    s.scroll_viewport(i32::MAX).unwrap();
    assert_eq!(
        s.scroll_offset(),
        0,
        "saturating scroll down should clamp to 0"
    );
}

#[test]
fn line_buffer_survives_resize_widen() {
    let mut s = session_with(40, 3, 1024);
    // Long soft-wrapped line plus a few hard rows so the LineBuffer
    // captures something the resize might otherwise wipe.
    s.feed(b"a long line that will wrap at 40 cols and again somewhere\r\nx\r\ny\r\nz\r\n")
        .unwrap();
    let before_len = s.line_buffer().len();
    assert!(before_len > 0, "test setup must populate LineBuffer first");
    let before_first = s.line_buffer().get(0).unwrap().text.clone();
    s.resize(120, 3).unwrap();
    // A no-op feed re-enters `sync_after_ghostty_scrollback_shrink`
    // (the false-positive wipe path) without scrolling new rows out
    // of the viewport, so any change in LineBuffer length must come
    // from the bug being patched here.
    s.feed(b"").unwrap();
    assert!(
        s.line_buffer().len() >= before_len,
        "widen resize must not wipe LineBuffer (was {before_len}, now {})",
        s.line_buffer().len()
    );
    assert_eq!(
        s.line_buffer().get(0).unwrap().text,
        before_first,
        "first logical line must survive widen resize"
    );
}

/// Count how many times each `MKnn` marker appears across the whole
/// unified frame.
fn marker_counts(s: &TerminalSession) -> std::collections::HashMap<String, u32> {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for y in 0..s.total_rows() {
        let text = s.dump_screen_row(y).unwrap_or_default();
        if let Some(mk) = text.get(0..4).filter(|mk| mk.starts_with("MK")) {
            *seen.entry(mk.to_string()).or_default() += 1;
        }
    }
    seen
}

/// Trimmed text of every unified-frame row, top to bottom.
fn frame_lines(s: &TerminalSession) -> Vec<String> {
    (0..s.total_rows())
        .map(|y| {
            s.dump_screen_row(y)
                .unwrap_or_default()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn annotation_survives_widen_reentry_into_viewport() {
    // The widen mirror of the narrow-detach bug. A buffer line carrying an
    // annotation (Buffered coord) that a widen reflow un-wraps and pulls back
    // into the live viewport must keep its annotation — re-anchored from
    // Buffered to Viewport. Previously `pop_verified_reentered_lines` popped
    // the re-entered tail and EVICTED its marks instead of re-promoting them,
    // so the annotation silently vanished.
    let mut s = session_with(80, 6, 100_000);
    let mut feed = Vec::new();
    for i in 1..=30u32 {
        // 155 chars: two rows at width 80, one row at width 200.
        feed.extend_from_slice(format!("MK{i:02}-{}\r\n", "a".repeat(150)).as_bytes());
    }
    s.feed(&feed).unwrap();

    // Annotate the NEWEST buffer line (the one nearest the seam, which a widen
    // pulls back into the viewport first). It is a `LineCoord::Buffered` anchor.
    let lb_rows = {
        // First live row's screen index = wrapped-row count of the LineBuffer.
        s.total_rows() - u32::from(s.rows())
    };
    // Walk up from the seam to the last buffer row that starts a logical line.
    let (target_row, annotated_text) = (0..lb_rows)
        .rev()
        .find_map(|y| {
            let t = s.dump_screen_row(y).unwrap_or_default();
            t.starts_with("MK")
                .then(|| (y, t.trim_end().chars().take(4).collect::<String>()))
        })
        .expect("a buffer row starts a logical line");
    let coord = s
        .screen_row_to_line_coord(target_row)
        .expect("buffer row resolves to a coord");
    assert!(
        matches!(coord, LineCoord::Buffered(_)),
        "setup: annotated row must start as a Buffered coord, got {coord:?}"
    );
    let id = s
        .add_annotation(LineRange::new(coord, coord), "note".into())
        .expect("annotation adds");

    s.resize(200, 6).unwrap();

    // The annotated line is now back in the live viewport. It must still carry
    // its annotation (re-anchored to a Viewport coord).
    let row = (0..s.total_rows())
        .find(|&y| {
            s.dump_screen_row(y)
                .unwrap_or_default()
                .starts_with(&annotated_text)
        })
        .expect("annotated line still present after widen");
    let new_coord = s
        .screen_row_to_line_coord(row)
        .expect("row resolves after widen");
    let hit = s.annotation_at_point(new_coord, 0);
    assert_eq!(
        hit.map(|(mid, _)| mid),
        Some(id),
        "annotation must survive a widen that pulls its line back into the \
         viewport; new_coord={new_coord:?}"
    );
}

#[test]
fn logical_abs_and_screen_row_are_inverses() {
    // `screen_row_to_logical_abs` is the inverse of `abs_to_screen_row`; mark
    // re-anchoring across reflow depends on the round-trip holding. Every
    // resolvable unified row must map to a logical abs that projects back to a
    // row resolving to the same abs (continuation rows snap to their head).
    let mut s = session_with(80, 6, 100_000);
    let mut feed = Vec::new();
    for i in 1..=20u32 {
        feed.extend_from_slice(format!("MK{i:02}-{}\r\n", "a".repeat(150)).as_bytes());
    }
    s.feed(&feed).unwrap();

    for row in 0..s.total_rows() {
        let Some(abs) = s.screen_row_to_logical_abs(row) else {
            continue;
        };
        let head = s
            .abs_to_screen_row(abs)
            .expect("a resolvable abs must project to a row");
        let abs2 = s
            .screen_row_to_logical_abs(head)
            .expect("the projected head row must resolve to an abs");
        assert_eq!(abs, abs2, "round-trip broke at row {row}");
    }
}

#[test]
fn annotation_survives_narrow_then_widen_roundtrip() {
    // The live-drag loop shape: a mark migrates Viewport -> Buffered on the
    // narrow, then Buffered -> Viewport on the widen back. The full cycle must
    // keep it attached — the most likely regression vector for the migration.
    let mut s = session_with(200, 6, 100_000);
    let mut feed = Vec::new();
    for i in 1..=30u32 {
        feed.extend_from_slice(format!("MK{i:02}-{}\r\n", "a".repeat(150)).as_bytes());
    }
    s.feed(&feed).unwrap();

    let top_live_row = s.total_rows() - u32::from(s.rows());
    let coord = s
        .screen_row_to_line_coord(top_live_row)
        .expect("top live row resolves");
    let annotated_text = s
        .dump_screen_row(top_live_row)
        .unwrap_or_default()
        .trim_end()
        .chars()
        .take(4)
        .collect::<String>();
    let id = s
        .add_annotation(LineRange::new(coord, coord), "note".into())
        .expect("annotation adds");

    s.resize(80, 6).unwrap(); // -> pushed into scrollback (Buffered)
    s.resize(200, 6).unwrap(); // -> pulled back into the viewport (Viewport)

    let row = (0..s.total_rows())
        .find(|&y| {
            s.dump_screen_row(y)
                .unwrap_or_default()
                .starts_with(&annotated_text)
        })
        .expect("annotated line present after roundtrip");
    let new_coord = s.screen_row_to_line_coord(row).expect("row resolves");
    assert_eq!(
        s.annotation_at_point(new_coord, 0).map(|(mid, _)| mid),
        Some(id),
        "annotation must survive a narrow->widen roundtrip; new_coord={new_coord:?}"
    );
}

#[test]
fn widen_does_not_duplicate_reentered_scrollback_rows() {
    // The originally-reported bug: an app emits soft-wrapped output and the
    // window is then widened. At a narrow width, long lines soft-wrap and
    // scroll off into the LineBuffer. Widening un-wraps them: ghostty
    // reflows, the active viewport reaches further back into history, so
    // lines captured into the LineBuffer re-enter the live viewport. Without
    // the resize-time dedup, the unified frame (LineBuffer ++ viewport)
    // renders them twice.
    let mut s = session_with(20, 4, 1024);
    let mut feed = Vec::new();
    for i in 1..=8u32 {
        feed.extend_from_slice(format!("MK{i:02}-aaaaaaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
    }
    s.feed(&feed).unwrap();
    assert!(
        !s.line_buffer().is_empty(),
        "setup must scroll some lines into the LineBuffer"
    );

    s.resize(80, 4).unwrap();

    let seen = marker_counts(&s);
    let duplicated: Vec<_> = seen.iter().filter(|&(_, &c)| c > 1).collect();
    assert!(
        duplicated.is_empty(),
        "widen must not duplicate re-entered scrollback rows; duplicated: {duplicated:?}"
    );
    for i in 1..=8u32 {
        let key = format!("MK{i:02}");
        assert_eq!(seen.get(&key), Some(&1), "{key} must appear exactly once");
    }
}

#[test]
fn grow_rows_at_narrow_width_does_not_duplicate_complete_lines() {
    // Growing the viewport height at the *same* wrapping width pulls
    // scrollback rows back into the active grid — a re-entry with no column
    // reflow, where the viewport top lands mid-line (a straddler). The
    // `skip_fragment` guard realigns the comparison so every fully
    // re-entered complete logical line is popped.
    let mut s = session_with(20, 4, 1024);
    let mut feed = Vec::new();
    for i in 1..=12u32 {
        feed.extend_from_slice(format!("MK{i:02}-aaaaaaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
    }
    s.feed(&feed).unwrap();
    assert!(
        !s.line_buffer().is_empty(),
        "setup must populate LineBuffer"
    );

    s.resize(20, 16).unwrap(); // grow rows only

    let seen = marker_counts(&s);
    let dups: Vec<_> = seen.iter().filter(|&(_, &c)| c > 1).collect();
    assert!(
        dups.is_empty(),
        "growing rows must not duplicate re-entered complete lines; duplicated: {dups:?}"
    );
}

#[test]
fn grow_rows_straddler_tail_row_no_residual_duplicate() {
    // Each line carries a unique tail token (`ENDnn`) so the straddler's
    // tail *row* — not just its line start — is detectable. The straddler
    // heal re-seats the LineBuffer's complete copy as head-only so the
    // viewport's re-shown tail row is not a second copy.
    let mut s = session_with(20, 4, 1024);
    let mut feed = Vec::new();
    for i in 1..=12u32 {
        feed.extend_from_slice(format!("L{i:02}{}END{i:02}\r\n", "x".repeat(20)).as_bytes());
    }
    s.feed(&feed).unwrap();

    s.resize(20, 16).unwrap();

    let mut counts: std::collections::HashMap<u32, u32> = Default::default();
    for y in 0..s.total_rows() {
        let t = s.dump_screen_row(y).unwrap_or_default();
        for i in 1..=12u32 {
            if t.contains(&format!("END{i:02}")) {
                *counts.entry(i).or_default() += 1;
            }
        }
    }
    let dups: Vec<_> = counts.iter().filter(|&(_, &c)| c > 1).collect();
    assert!(
        dups.is_empty(),
        "straddler tail row must not duplicate; duplicated: {dups:?}"
    );
}

#[test]
fn resize_dedup_preserves_all_lines_across_narrow_and_repeated_widen() {
    // Guards against over-popping: narrowing (rows leave the viewport, none
    // re-enter) and repeated widening (idempotent) must each leave every
    // unique line present exactly once.
    let mut s = session_with(20, 4, 1024);
    let mut feed = Vec::new();
    for i in 1..=8u32 {
        feed.extend_from_slice(format!("MK{i:02}-aaaaaaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
    }
    s.feed(&feed).unwrap();

    for &(c, r) in &[(80u16, 4u16), (30, 4), (80, 4)] {
        s.resize(c, r).unwrap();
        let seen = marker_counts(&s);
        for i in 1..=8u32 {
            let key = format!("MK{i:02}");
            assert_eq!(
                seen.get(&key),
                Some(&1),
                "{key} must appear exactly once after resize to {c}x{r}; counts={seen:?}"
            );
        }
    }
}

#[test]
fn shrink_then_widen_roundtrip_preserves_lines() {
    // Shrink (narrow) then grow back (widen) at a fixed height. The narrow
    // reflows lines to wrap; the widen un-wraps and re-enters them. Every
    // line must survive exactly once: no duplicate from the widen, no loss
    // from the narrow.
    let mut s = session_with(80, 4, 1024);
    let mut feed = Vec::new();
    for i in 1..=10u32 {
        feed.extend_from_slice(format!("MK{i:02}-aaaaaaaaaaaaaaaaaaaaaaaa\r\n").as_bytes());
    }
    s.feed(&feed).unwrap();

    s.resize(20, 4).unwrap();
    s.resize(80, 4).unwrap();

    let seen = marker_counts(&s);
    for i in 1..=10u32 {
        let key = format!("MK{i:02}");
        assert_eq!(
            seen.get(&key),
            Some(&1),
            "{key} must appear exactly once after shrink→widen; counts={seen:?}"
        );
    }
}

#[test]
fn widen_dedup_handles_cjk_wide_char_straddle() {
    // CJK syllables are 2 cells wide, so one straddles the right margin and
    // ghostty marks that row's eol `Dwc`. The captured LineBuffer tail is a
    // Dwc partial, which the dedup must treat like a Soft head (prefix match)
    // on widen — without duplicating, losing, or corrupting the wide content.
    let mut s = session_with(20, 4, 1024);
    let suffix = "가나다라마바사아자차가나";
    let mut feed = Vec::new();
    for i in 1..=8u32 {
        feed.extend_from_slice(format!("MK{i:02}X{suffix}\r\n").as_bytes());
    }
    s.feed(&feed).unwrap();
    assert!(
        !s.line_buffer().is_empty(),
        "setup must populate LineBuffer"
    );

    s.resize(80, 4).unwrap();

    let seen = marker_counts(&s);
    for i in 1..=8u32 {
        let key = format!("MK{i:02}");
        assert_eq!(seen.get(&key), Some(&1), "{key} must appear exactly once");
    }
    let expected = format!("MK03X{suffix}");
    let hits = frame_lines(&s)
        .into_iter()
        .filter(|l| *l == expected)
        .count();
    assert_eq!(
        hits, 1,
        "MK03 wide-char line must reflow intact exactly once"
    );
}

#[test]
fn alt_screen_seals_partial_tail() {
    let mut s = session_with(80, 3, 1024);
    // First chunk lacks a trailing newline so the row, once it scrolls
    // out, would normally land with a Soft eol. We then feed enough
    // hard-newline rows to push it off the top and observe the eol
    // before alt-screen entry.
    s.feed(b"partial-no-newline").unwrap();
    s.feed(b"\r\nx\r\ny\r\nz\r\n").unwrap();
    s.feed(b"\x1b[?1049h").unwrap();
    // The test is meaningless unless we actually exercised the capture
    // path; an empty LineBuffer would let the assertion vacuously pass.
    let len = s.line_buffer().len();
    assert!(
        len > 0,
        "expected at least one captured row before alt-screen entry"
    );
    let last = s.line_buffer().get(len - 1).unwrap();
    assert_eq!(
        last.eol,
        EolKind::Hard,
        "alt-screen entry should have sealed partial tail to Hard"
    );
}

#[test]
fn capture_attaches_osc8_url_id_to_correct_char_after_scrollout() {
    // A wide CJK char followed by an ASCII char that alone carries the
    // OSC 8 link, then enough newlines to scroll the row into scrollback.
    // The captured LineBuffer cells must carry the link on the ASCII char
    // and not on the CJK char — the per-physical-cell id dump used to
    // shift it onto the wrong cell (or drop it) on wide-char rows.
    let mut s = session_with(80, 3, 1024);
    s.feed("\u{AC00}".as_bytes()).unwrap();
    s.feed(b"\x1b]8;;https://example.com\x07x\x1b]8;;\x07\r\n")
        .unwrap();
    s.feed(b"b\r\nc\r\nd\r\ne\r\n").unwrap();

    let line = (0..s.line_buffer().len())
        .map(|i| s.line_buffer().get(i).unwrap())
        .find(|l| l.text.starts_with('\u{AC00}'))
        .expect("the CJK+link row must be captured into scrollback");
    assert_eq!(line.text, "\u{AC00}x");
    assert_eq!(line.cells.len(), 2, "one cell per char");
    assert!(
        line.cells[0].url_id.is_none(),
        "the CJK char must carry no link"
    );
    assert!(
        line.cells[1].url_id.is_some(),
        "the link must land on the ASCII char"
    );
}

#[test]
fn diag_scroll_after_same_size_resize() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\n").unwrap();
    assert!(s.line_buffer().wrapped_row_count(80) > 0);
    // lane-switch tail: pending_resize fires resize with identical dims
    s.resize(80, 3).unwrap();
    // next output tick after activation
    s.feed(b"").unwrap();
    let max_scroll = s.line_buffer().wrapped_row_count(80);
    assert!(max_scroll > 0, "scrollback wiped by same-size resize");
    let before = s.viewport_row_offset();
    s.scroll_viewport(-1).unwrap();
    assert_ne!(
        s.viewport_row_offset(),
        before,
        "scroll dead after same-size resize"
    );
}

/// Clear Buffer must leave the prompt on screen. `TerminalView::on_clear_buffer`
/// feeds exactly these bytes; erasing the whole display instead (the old
/// `ED 2` + `CUP` path) dropped the prompt row, and since the clear never
/// reaches the shell nothing repainted it.
#[test]
fn clear_buffer_sequence_keeps_the_prompt_and_the_cursor_column() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]133;A\x07user% \x1b]133;B\x07echo one\r\none\r\n")
        .unwrap();
    session
        .feed(b"\x1b]133;A\x07user% \x1b]133;B\x07echo two\r\ntwo\r\n")
        .unwrap();
    session.feed(b"\x1b]133;A\x07user% \x1b]133;B\x07").unwrap();

    let (col_before, row_before) = session.cursor_position().expect("cursor on screen");
    assert!(row_before > 1, "the prompt must start below the top row");

    // Mirrors `on_clear_buffer`: lift the cursor's row to the top, drop history.
    let above = row_before - 1;
    session.feed(&crate::ansi::lift_rows_to_top(above)).unwrap();
    session.feed(crate::ansi::ERASE_SCROLLBACK).unwrap();

    assert_eq!(
        session.dump_screen_row(0).unwrap(),
        "user% ",
        "the prompt survives at the top of the screen"
    );
    assert_eq!(
        session.cursor_position(),
        Some((col_before, 1)),
        "the cursor keeps its column within the prompt line and moves to row 1"
    );
    assert_eq!(
        session.dump_screen_row(1).unwrap(),
        "",
        "everything the prompt scrolled over is gone"
    );
}

/// On the alt screen the running app owns the display, so a buffer clear must
/// only drop the primary screen's history. Scrolling the alt rows would leave
/// the grid offset from what the app believes it drew.
#[test]
fn clear_buffer_leaves_the_alt_screen_untouched() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"scrollback line\r\n").unwrap();
    session.feed(b"\x1b[?1049h").unwrap();
    session.feed(b"TUI row one\r\nTUI row two").unwrap();
    assert!(session.is_alt_screen());

    let rows_before: Vec<String> = (0..session.total_rows())
        .map(|y| session.dump_screen_row(y).unwrap())
        .collect();
    let cursor_before = session.cursor_position();
    assert!(
        rows_before.iter().any(|row| row == "TUI row one"),
        "the alt screen holds the app's output before the clear"
    );

    // `on_clear_buffer` skips the lift while the alt screen is up and feeds
    // only this.
    session.feed(crate::ansi::ERASE_SCROLLBACK).unwrap();

    let rows_after: Vec<String> = (0..session.total_rows())
        .map(|y| session.dump_screen_row(y).unwrap())
        .collect();
    assert_eq!(rows_after, rows_before, "the alt display is left alone");
    assert_eq!(session.cursor_position(), cursor_before);
}
