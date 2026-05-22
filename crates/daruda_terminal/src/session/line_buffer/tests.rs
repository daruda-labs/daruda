use super::*;

#[test]
fn append_partial_extends_in_place() {
    let mut b = LineBuffer::new(1024);
    b.append("hello", &[], EolKind::Soft);
    b.append(" world", &[], EolKind::Hard);
    assert_eq!(b.len(), 1);
    assert_eq!(b.get(0).unwrap().text, "hello world");
    assert_eq!(b.get(0).unwrap().eol, EolKind::Hard);
    assert_eq!(b.get(0).unwrap().cells.len(), 11); // "hello world" = 11 cells
}

#[test]
fn wrap_visible_handles_cjk_two_cells() {
    let mut b = LineBuffer::new(1024);
    b.append("가나다라마", &[], EolKind::Hard); // 5 chars * 2 cells = 10 cells
    let rows = b.wrap_visible(0, 5, 4); // wrap at 4 cells
    assert_eq!(rows, vec!["가나", "다라", "마"]);
}

#[test]
fn dwc_eol_skips_right_edge_when_dwc_straddles() {
    let mut b = LineBuffer::new(1024);
    b.append("abc", &[], EolKind::Dwc); // right edge had DWC_SKIP
    b.append("가", &[], EolKind::Hard);
    // One logical line "abc가" (Dwc extends in place); wrap at 4 produces
    // two visual rows.
    let rows = b.wrap_visible(0, 4, 4);
    assert_eq!(rows, vec!["abc", "가"]);
}

#[test]
fn ring_evicts_oldest_increments_overflow() {
    let mut b = LineBuffer::new(2);
    b.append("a", &[], EolKind::Hard);
    b.append("b", &[], EolKind::Hard);
    b.append("c", &[], EolKind::Hard);
    assert_eq!(b.len(), 2);
    assert_eq!(b.overflow(), 1); // 1 line dropped
    assert_eq!(b.get(0).unwrap().text, "b");
}

#[test]
fn line_buffer_position_survives_eviction() {
    let mut b = LineBuffer::new(3);
    b.append("a", &[], EolKind::Hard);
    let pos = b.position_at(0).unwrap(); // points at "a"
    b.append("b", &[], EolKind::Hard);
    b.append("c", &[], EolKind::Hard);
    b.append("d", &[], EolKind::Hard); // "a" evicted
    assert!(b.deref(&pos).is_none()); // dropped — caller handles
}

#[test]
fn empty_hard_eol_after_partial_starts_new_logical_line() {
    let mut b = LineBuffer::new(1024);
    b.append("partial", &[], EolKind::Soft);
    b.append("", &[], EolKind::Hard);
    assert_eq!(b.len(), 2, "empty hard line is a separate logical line");
    assert_eq!(b.get(0).unwrap().text, "partial");
    assert_eq!(b.get(0).unwrap().eol, EolKind::Soft);
    assert_eq!(b.get(1).unwrap().text, "");
    assert_eq!(b.get(1).unwrap().eol, EolKind::Hard);
}

#[test]
fn append_empty_string_pushes_empty_line() {
    let mut b = LineBuffer::new(1024);
    b.append("", &[], EolKind::Hard);
    assert_eq!(b.len(), 1);
    assert_eq!(b.get(0).unwrap().text, "");
    assert_eq!(b.get(0).unwrap().cells.len(), 0);
}

#[test]
fn position_at_empty_buffer_returns_none() {
    let b = LineBuffer::new(1024);
    assert!(b.position_at(0).is_none());
}

#[test]
fn clear_preserves_overflow_and_invalidates_old_positions() {
    let mut b = LineBuffer::new(2);
    b.append("a", &[], EolKind::Hard);
    b.append("b", &[], EolKind::Hard);
    b.append("c", &[], EolKind::Hard); // overflow=1
    let pos = b.position_at(0).unwrap(); // points at "b"
    b.clear();
    assert_eq!(b.overflow(), 1);
    assert!(b.deref(&pos).is_none()); // evicted by clear
}

#[test]
fn seal_partial_is_idempotent() {
    let mut b = LineBuffer::new(1024);
    b.append("x", &[], EolKind::Soft);
    b.seal_partial();
    b.seal_partial(); // no-op
    assert_eq!(b.get(0).unwrap().eol, EolKind::Hard);
}

#[test]
fn locate_visual_row_maps_flat_y_to_line_and_subrow() {
    let mut b = LineBuffer::new(1024);
    b.append("abcdef", &[], EolKind::Hard); // 6 cells → 2 rows at width 3
    b.append("ghi", &[], EolKind::Hard); // 3 cells → 1 row at width 3
    // wrapped layout at cell_cols=3:
    //   y=0 → line 0 sub 0 ("abc")
    //   y=1 → line 0 sub 1 ("def")
    //   y=2 → line 1 sub 0 ("ghi")
    assert_eq!(b.locate_visual_row(0, 3), Some((0, 0)));
    assert_eq!(b.locate_visual_row(1, 3), Some((0, 1)));
    assert_eq!(b.locate_visual_row(2, 3), Some((1, 0)));
    assert_eq!(b.locate_visual_row(3, 3), None);
}

#[test]
fn locate_visual_row_counts_empty_line_as_one_row() {
    let mut b = LineBuffer::new(1024);
    b.append("", &[], EolKind::Hard); // empty line still occupies 1 row
    b.append("xy", &[], EolKind::Hard);
    assert_eq!(b.locate_visual_row(0, 4), Some((0, 0)));
    assert_eq!(b.locate_visual_row(1, 4), Some((1, 0)));
    assert_eq!(b.locate_visual_row(2, 4), None);
}

#[test]
fn locate_visual_row_zero_cols_returns_none() {
    let mut b = LineBuffer::new(1024);
    b.append("x", &[], EolKind::Hard);
    assert!(b.locate_visual_row(0, 0).is_none());
}

#[test]
fn wrap_visible_with_styles_emits_one_indexed_runs() {
    let mut b = LineBuffer::new(1024);
    // Input runs are 1-indexed (cell 1 = 'a'). The whole line is one
    // run with non-default fg/bg so the run survives the merge.
    let runs = vec![StyleRun {
        start_col: 1,
        end_col: 3,
        fg: Rgb { r: 1, g: 2, b: 3 },
        bg: Rgb { r: 4, g: 5, b: 6 },
        flags: 0,
    }];
    b.append("abc", &runs, EolKind::Hard);
    let rows = b.wrap_visible_with_styles(0, 1, 10);
    assert_eq!(rows.len(), 1);
    let (text, out_runs) = &rows[0];
    assert_eq!(text, "abc");
    assert_eq!(out_runs.len(), 1, "expected one merged run");
    // Output is 1-indexed inclusive: cell 1..=3 covers 'a', 'b', 'c'.
    assert_eq!(out_runs[0].start_col, 1);
    assert_eq!(out_runs[0].end_col, 3);
    assert_eq!(out_runs[0].fg, Rgb { r: 1, g: 2, b: 3 });
    assert_eq!(out_runs[0].bg, Rgb { r: 4, g: 5, b: 6 });
}

#[test]
fn wrap_visible_with_styles_resets_columns_per_row() {
    // 6 cells split into two rows of width 3. Each row's run should
    // restart its column numbering at 1.
    let mut b = LineBuffer::new(1024);
    let runs = vec![StyleRun {
        start_col: 1,
        end_col: 6,
        fg: Rgb { r: 9, g: 9, b: 9 },
        bg: Rgb { r: 0, g: 0, b: 0 },
        flags: 0,
    }];
    b.append("abcdef", &runs, EolKind::Hard);
    let rows = b.wrap_visible_with_styles(0, 2, 3);
    assert_eq!(rows.len(), 2);
    for (_, row_runs) in &rows {
        assert_eq!(row_runs.len(), 1);
        assert_eq!(row_runs[0].start_col, 1);
        assert_eq!(row_runs[0].end_col, 3);
    }
}

#[test]
fn wrap_visible_at_cell_cols_one_breaks_cjk_to_skipped_row() {
    // CJK chars are width 2; cell_cols=1 means each CJK char doesn't fit
    // on any row of width 1. The current implementation places the wide
    // char on its own row (overflowing the nominal width by 1) rather
    // than dropping it — see `wrap_visible` rustdoc. We assert the
    // surrounding single-cell rows are emitted; the wide-char row is
    // implementation-defined overflow.
    let mut b = LineBuffer::new(1024);
    b.append("a가b", &[], EolKind::Hard);
    let rows = b.wrap_visible(0, 5, 1);
    assert!(rows.iter().any(|r| r == "a"));
    assert!(rows.iter().any(|r| r == "b"));
}

#[test]
fn wrap_visible_cjk_at_start_with_narrow_width_does_not_emit_empty_row() {
    let mut b = LineBuffer::new(1024);
    b.append("가", &[], EolKind::Hard);
    let rows = b.wrap_visible(0, 5, 1);
    // The 2-cell char gets its own row (overflowing nominal width by 1),
    // but no leading empty row is emitted.
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], "가");
}

#[test]
fn attach_url_ids_to_tail_sets_cell_url_id_for_nonzero_entries() {
    let mut b = LineBuffer::new(1024);
    b.append("abcd", &[], EolKind::Hard);
    // Cells 0..3 = a/b/c/d. Mark cells 1 and 2 with url_id=7, leave 0 and 3 unset.
    b.attach_url_ids_to_tail(&[0, 7, 7, 0]);
    let line = b.get(0).unwrap();
    assert_eq!(line.cells[0].url_id, None);
    assert_eq!(line.cells[1].url_id, NonZeroU16::new(7));
    assert_eq!(line.cells[2].url_id, NonZeroU16::new(7));
    assert_eq!(line.cells[3].url_id, None);
}

#[test]
fn attach_url_ids_to_tail_aligns_with_trailing_cells_when_shorter() {
    // When the slice is shorter than the line's cells (e.g. an extend
    // path appended only a tail segment), the IDs land on the trailing
    // cells — matching the segment that was just appended.
    let mut b = LineBuffer::new(1024);
    b.append("abc", &[], EolKind::Soft);
    b.append("de", &[], EolKind::Hard); // extends; final cells = a/b/c/d/e
    b.attach_url_ids_to_tail(&[9, 9]); // only the just-appended segment
    let line = b.get(0).unwrap();
    assert_eq!(line.cells[0].url_id, None);
    assert_eq!(line.cells[1].url_id, None);
    assert_eq!(line.cells[2].url_id, None);
    assert_eq!(line.cells[3].url_id, NonZeroU16::new(9));
    assert_eq!(line.cells[4].url_id, NonZeroU16::new(9));
}

#[test]
fn find_matches_projects_single_line_match_to_one_row() {
    let mut b = LineBuffer::new(1024);
    b.append("hello world", &[], EolKind::Hard);
    let opts = FindOptions::default();
    let matches = b.find_matches("world", opts, 80);
    assert_eq!(matches.len(), 1);
    // "world" starts at cell column 7 (1-indexed), ends at column 11.
    assert_eq!(matches[0], (0, 7, 11));
}

#[test]
fn find_matches_emits_one_entry_per_visual_row_for_wrap_spanning_match() {
    let mut b = LineBuffer::new(1024);
    // 8-char line wrapping at cell_cols=4 → rows: "abcd" / "efgh"
    b.append("abcdefgh", &[], EolKind::Hard);
    let opts = FindOptions::default();
    let matches = b.find_matches("cdef", opts, 4);
    // Match spans last 2 cols of row 0 and first 2 cols of row 1.
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0], (0, 3, 4)); // "cd" on row 0
    assert_eq!(matches[1], (1, 1, 2)); // "ef" on row 1
}

#[test]
fn find_matches_spans_two_logical_lines() {
    let mut b = LineBuffer::new(1024);
    b.append("hello wor", &[], EolKind::Hard);
    b.append("ld there", &[], EolKind::Hard);
    let opts = FindOptions::default();
    let matches = b.find_matches("world", opts, 80);
    // No wrap; one row per logical line spanned.
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].0, 0);
    assert_eq!(matches[1].0, 1);
    assert_eq!(matches[0], (0, 7, 9));
    assert_eq!(matches[1], (1, 1, 2));
}

#[test]
fn wrap_cache_returns_same_value_for_same_width() {
    // Sanity check that repeated wrapped_row_count calls at the same width
    // are stable (cache hit path is correct even when nothing mutated).
    let mut b = LineBuffer::new(1024);
    b.append("hello world", &[], EolKind::Hard);
    let n1 = b.wrapped_row_count(4);
    let n2 = b.wrapped_row_count(4);
    let n3 = b.wrapped_row_count(4);
    assert_eq!(n1, n2);
    assert_eq!(n2, n3);
    assert_eq!(n1, 3); // "hell"/"o wo"/"rld"
}

#[test]
fn wrap_cache_invalidates_on_extend() {
    let mut b = LineBuffer::new(1024);
    b.append("hello", &[], EolKind::Soft);
    let n1 = b.wrapped_row_count(3); // "hel"/"lo" → 2
    b.append(" world", &[], EolKind::Hard);
    let n2 = b.wrapped_row_count(3); // now "hel"/"lo "/"wor"/"ld" → 4
    assert_eq!(n1, 2);
    assert_eq!(n2, 4);
}

#[test]
fn visual_row_matches_wrap_visible() {
    let mut b = LineBuffer::new(1024);
    b.append("hello world this is a long line", &[], EolKind::Hard);
    let wrapped = b.wrap_visible(0, 10, 7);
    for (i, expected) in wrapped.iter().enumerate() {
        let direct = b.visual_row(i as u32, 7).expect("row exists");
        assert_eq!(&direct, expected, "row {i} mismatch");
    }
}

#[test]
fn position_for_visual_row_round_trips_basic_ascii() {
    let mut b = LineBuffer::new(1024);
    // 6 cells wrapped at 3 → 2 visual rows for line 0; line 1 = 1 row.
    b.append("abcdef", &[], EolKind::Hard);
    b.append("ghi", &[], EolKind::Hard);

    // y=0 → line 0 sub 0. sub_col_origin = 0 (start of line).
    let (pos, sub_row, origin) = b.position_for_visual_row(0, 3).unwrap();
    assert_eq!(sub_row, 0);
    assert_eq!(origin, 0);
    // Inverse: cumulative_col=0 lands back on (visual_y=0, col_in_row=0).
    assert_eq!(b.coordinate_for_position(&pos, 3, 0), Some((0, 0)));

    // y=1 → line 0 sub 1. sub_col_origin = 3 (cells consumed by sub_row 0).
    let (pos, sub_row, origin) = b.position_for_visual_row(1, 3).unwrap();
    assert_eq!(sub_row, 1);
    assert_eq!(origin, 3);
    // Inverse: cumulative_col=3 → (visual_y=1, col_in_row=0).
    assert_eq!(b.coordinate_for_position(&pos, 3, 3), Some((1, 0)));
    // Mid-row offset.
    assert_eq!(b.coordinate_for_position(&pos, 3, 5), Some((1, 2)));

    // y=2 → line 1 sub 0. sub_col_origin = 0.
    let (pos, sub_row, origin) = b.position_for_visual_row(2, 3).unwrap();
    assert_eq!(sub_row, 0);
    assert_eq!(origin, 0);
    assert_eq!(b.coordinate_for_position(&pos, 3, 1), Some((2, 1)));
}

#[test]
fn position_for_visual_row_survives_width_change() {
    // Anchor at visual row 1 (width 3) lands on line 0 sub_row 1.
    // Widening to 6 fits the whole line on one row; the same cumulative
    // cell column resolves to the new wrap layout.
    let mut b = LineBuffer::new(1024);
    b.append("abcdef", &[], EolKind::Hard);

    let (pos, sub_row, origin) = b.position_for_visual_row(1, 3).unwrap();
    assert_eq!(sub_row, 1);
    assert_eq!(origin, 3);
    // At width 3, cumulative_col 3 → (visual_y=1, col 0).
    assert_eq!(b.coordinate_for_position(&pos, 3, 3), Some((1, 0)));
    // After widening to 6, cumulative_col 3 → (visual_y=0, col 3).
    assert_eq!(b.coordinate_for_position(&pos, 6, 3), Some((0, 3)));
    // After narrowing to 2: "ab"/"cd"/"ef" → cumulative_col 3 lands on
    // the 'd' cell (visual_y=1, col 1).
    assert_eq!(b.coordinate_for_position(&pos, 2, 3), Some((1, 1)));
}

#[test]
fn position_for_visual_row_handles_cjk_wrap() {
    // "가나다라" = 4 chars * 2 cells = 8 cells. At cell_cols=4 each
    // CJK pair fills the row exactly → 2 visual rows.
    let mut b = LineBuffer::new(1024);
    b.append("가나다라", &[], EolKind::Hard);

    let (_pos0, sub0, origin0) = b.position_for_visual_row(0, 4).unwrap();
    let (pos1, sub1, origin1) = b.position_for_visual_row(1, 4).unwrap();
    assert_eq!((sub0, origin0), (0, 0));
    assert_eq!((sub1, origin1), (1, 4));
    // cumulative_col=4 (start of sub_row 1) → visual_y=1, col 0.
    assert_eq!(b.coordinate_for_position(&pos1, 4, 4), Some((1, 0)));
    // cumulative_col=6 lands inside the second CJK pair on sub_row 1 →
    // col_in_row=2 (left half of "라").
    assert_eq!(b.coordinate_for_position(&pos1, 4, 6), Some((1, 2)));
}

#[test]
fn coordinate_for_position_returns_none_for_evicted() {
    let mut b = LineBuffer::new(2);
    b.append("a", &[], EolKind::Hard);
    let pos = b.position_at(0).unwrap();
    b.append("b", &[], EolKind::Hard);
    b.append("c", &[], EolKind::Hard); // "a" evicted → overflow = 1
    assert!(b.coordinate_for_position(&pos, 80, 0).is_none());
}

#[test]
fn rows_at_width_handles_cjk_odd_divisor() {
    // Three 2-cell CJK chars (6 cells total). At width 3 each char
    // overflows the 1-cell slack, so the wrap walk produces 3 rows —
    // div_ceil(6, 3) = 2 would be wrong.
    let mut b = LineBuffer::new(1024);
    b.append("가나다", &[], EolKind::Hard);
    assert_eq!(b.wrapped_row_count(3), 3);
    // At width 4 two 2-cell chars fit per row → 2 rows.
    assert_eq!(b.wrapped_row_count(4), 2);
    // At width 2 each 2-cell char fills the row → 3 rows.
    assert_eq!(b.wrapped_row_count(2), 3);
}

#[test]
fn position_for_visual_row_handles_cjk_odd_divisor() {
    let mut b = LineBuffer::new(1024);
    b.append("가나다", &[], EolKind::Hard);

    // y=0 → line 0 sub 0, origin = 0.
    let (pos0, sub0, origin0) = b.position_for_visual_row(0, 3).unwrap();
    assert_eq!((sub0, origin0), (0, 0));
    // Inverse: cumulative_col=0 round-trips to (visual_y=0, col_in_row=0).
    assert_eq!(b.coordinate_for_position(&pos0, 3, 0), Some((0, 0)));

    // y=1 → second CJK char on its own row; cumulative col = 2.
    let (_pos1, sub1, origin1) = b.position_for_visual_row(1, 3).unwrap();
    assert_eq!((sub1, origin1), (1, 2));

    // y=2 → third CJK char; cumulative col = 4.
    let (_pos2, sub2, origin2) = b.position_for_visual_row(2, 3).unwrap();
    assert_eq!((sub2, origin2), (2, 4));

    // y=3 is past the wrapped row count.
    assert!(b.position_for_visual_row(3, 3).is_none());
}

#[test]
fn coordinate_for_position_handles_cjk_odd_divisor() {
    let mut b = LineBuffer::new(1024);
    b.append("가나다", &[], EolKind::Hard);
    let pos = b.position_at(0).unwrap();
    // cumulative_col=4 (start of third CJK char) → row 2, col 0.
    assert_eq!(b.coordinate_for_position(&pos, 3, 4), Some((2, 0)));
}

#[test]
fn position_for_visual_row_zero_cols_returns_none() {
    let mut b = LineBuffer::new(1024);
    b.append("x", &[], EolKind::Hard);
    assert!(b.position_for_visual_row(0, 0).is_none());
    let pos = b.position_at(0).unwrap();
    assert!(b.coordinate_for_position(&pos, 0, 0).is_none());
}
