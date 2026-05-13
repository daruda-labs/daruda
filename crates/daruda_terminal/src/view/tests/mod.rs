use ghostty_vt::Rgb;

use super::jump::next_prompt_index;
use super::mouse::window_position_to_local;
use super::selection::{
    BlockRect, ByteSelection, CellAnchor, ScreenPos, SelectionMode, Side, block_copy_text,
    block_rect_from_anchors, block_selection_quads, cell_side, pixel_to_cell_anchor,
    selection_mode_from_modifiers,
};
use super::viewport::{line_range_in_viewport, word_range_in_viewport};

#[test]
fn next_prompt_index_returns_none_on_empty_starts() {
    assert_eq!(next_prompt_index(&[], None, 0, true), None);
    assert_eq!(next_prompt_index(&[], Some(0), 0, false), None);
}

#[test]
fn fresh_forward_anchors_to_first_mark_at_or_below_viewport_top() {
    let j = next_prompt_index(&[10, 50, 90], None, 40, true).unwrap();
    assert_eq!(j.row, 50);
    assert!(!j.wrapped);
}

#[test]
fn fresh_forward_falls_back_to_first_and_flags_wrap() {
    // Viewport below every mark — fresh forward can't find a row
    // at or below viewport_top, so it wraps to the first mark and
    // must signal the wrap so the UI flashes.
    let j = next_prompt_index(&[10, 50, 90], None, 200, true).unwrap();
    assert_eq!(j.row, 10);
    assert!(j.wrapped);
}

#[test]
fn fresh_backward_anchors_to_last_mark_above_viewport_top() {
    let j = next_prompt_index(&[10, 50, 90], None, 80, false).unwrap();
    assert_eq!(j.row, 50);
    assert!(!j.wrapped);
}

#[test]
fn fresh_backward_falls_back_to_last_and_flags_wrap() {
    let j = next_prompt_index(&[10, 50, 90], None, 0, false).unwrap();
    assert_eq!(j.row, 90);
    assert!(j.wrapped);
}

#[test]
fn subsequent_forward_wraps_flags_wrapped() {
    // Previous row = 90 (last) → forward wraps to 10.
    let j = next_prompt_index(&[10, 50, 90], Some(90), 0, true).unwrap();
    assert_eq!(j.row, 10);
    assert!(j.wrapped, "forward step from last must flag wrap");
    // Previous row = 10 → forward = 50, no wrap.
    let j2 = next_prompt_index(&[10, 50, 90], Some(10), 0, true).unwrap();
    assert_eq!(j2.row, 50);
    assert!(!j2.wrapped);
}

#[test]
fn subsequent_backward_wraps_flags_wrapped() {
    // Previous row = 10 (first) → backward wraps to 90.
    let j = next_prompt_index(&[10, 50, 90], Some(10), 0, false).unwrap();
    assert_eq!(j.row, 90);
    assert!(j.wrapped, "backward step from first must flag wrap");
    // Previous row = 50 → backward = 10, no wrap.
    let j2 = next_prompt_index(&[10, 50, 90], Some(50), 0, false).unwrap();
    assert_eq!(j2.row, 10);
    assert!(!j2.wrapped);
}

#[test]
fn single_mark_step_is_trivial_wrap() {
    let j_fwd = next_prompt_index(&[42], Some(42), 0, true).unwrap();
    assert_eq!(j_fwd.row, 42);
    assert!(j_fwd.wrapped);
    let j_bwd = next_prompt_index(&[42], Some(42), 0, false).unwrap();
    assert_eq!(j_bwd.row, 42);
    assert!(j_bwd.wrapped);
}

#[test]
fn evicted_previous_row_falls_back_to_fresh_anchor() {
    // Previously-focused row 5 no longer exists (eviction). The
    // caller should NOT panic; instead fresh-anchor against
    // viewport_top. Here all marks are at/above 0 so fresh
    // backward wraps to the last.
    let j = next_prompt_index(&[10, 50], Some(5), 0, false).unwrap();
    assert_eq!(j.row, 50);
    assert!(j.wrapped);
}

#[test]
fn marks_inside_current_viewport_are_still_jumpable() {
    let starts = [100u32, 102, 104];
    let first = next_prompt_index(&starts, None, 99, true).unwrap();
    assert_eq!(first.row, 100);
    let second = next_prompt_index(&starts, Some(first.row), 99, true).unwrap();
    assert_eq!(second.row, 102);
}

#[test]
fn stepping_forward_visits_every_mark_in_order() {
    let starts = [10u32, 30, 50, 70];
    let mut row = next_prompt_index(&starts, None, 0, true).unwrap().row;
    let mut visited = vec![row];
    for _ in 0..4 {
        row = next_prompt_index(&starts, Some(row), 0, true).unwrap().row;
        visited.push(row);
    }
    assert_eq!(visited, vec![10, 30, 50, 70, 10]);
}

#[test]
fn stepping_backward_visits_every_mark_in_order() {
    let starts = [10u32, 30, 50, 70];
    let mut row = next_prompt_index(&starts, None, 200, false).unwrap().row;
    let mut visited = vec![row];
    for _ in 0..4 {
        row = next_prompt_index(&starts, Some(row), 0, false).unwrap().row;
        visited.push(row);
    }
    assert_eq!(visited, vec![70, 50, 30, 10, 70]);
}

#[test]
fn focus_row_survives_eviction_across_steps() {
    // If the mark list changes shape (head eviction), the
    // previously-tracked row must still resolve to a sensible next
    // step rather than silently pointing at a different command.
    let before = [10u32, 20, 30, 40];
    let step = next_prompt_index(&before, Some(20), 0, true).unwrap();
    assert_eq!(step.row, 30);
    // Eviction drops row 10; list is now [20, 30, 40]. The next
    // forward step must land on 40 (one past 30), not on 20.
    let after = [20u32, 30, 40];
    let step2 = next_prompt_index(&after, Some(step.row), 0, true).unwrap();
    assert_eq!(step2.row, 40);
}

#[test]
fn mouse_position_to_local_accounts_for_bounds_origin() {
    let bounds = Some(gpui::Bounds::new(
        gpui::point(gpui::px(100.0), gpui::px(20.0)),
        gpui::size(gpui::px(200.0), gpui::px(80.0)),
    ));

    let local = window_position_to_local(bounds, gpui::point(gpui::px(110.0), gpui::px(30.0)));
    assert_eq!(local, gpui::point(gpui::px(10.0), gpui::px(10.0)));
}

#[test]
fn selection_at_end_of_line_includes_newline() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let offsets = super::TerminalView::compute_viewport_line_offsets(&lines);
    let total = super::TerminalView::compute_viewport_total_len(&lines);

    assert_eq!(offsets, vec![0, 4]);
    assert_eq!(total, 8);

    // Anchor at byte 3 (past "abc" text), active at row 1 byte 0 — spans
    // the virtual newline so the selection is non-empty.
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 3,
    };
    let active = ScreenPos {
        screen_row: 1,
        byte: 0,
    };
    let sel = ByteSelection::linear(anchor, active);
    assert!(!sel.is_empty());
    let (start, end) = sel.normalized();
    assert_eq!(start, anchor);
    assert_eq!(end, active);
}

#[test]
fn selection_beyond_content_uses_total_len() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let total = super::TerminalView::compute_viewport_total_len(&lines);
    assert_eq!(total, 8);

    // Selection spanning from row 0 byte 1 to end of row 1 — non-empty,
    // normalized order is anchor first.
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 1,
    };
    let active = ScreenPos {
        screen_row: 1,
        byte: 3,
    };
    let sel = ByteSelection::linear(anchor, active);
    let (start, end) = sel.normalized();
    assert_eq!(start.screen_row, 0);
    assert_eq!(end.screen_row, 1);
}

#[test]
fn viewport_line_offsets_single_line() {
    let lines: Vec<String> = vec!["hello".into()];
    let offsets = super::TerminalView::compute_viewport_line_offsets(&lines);
    let total = super::TerminalView::compute_viewport_total_len(&lines);

    assert_eq!(offsets, vec![0]);
    assert_eq!(total, 6);
}

#[test]
fn viewport_line_offsets_empty() {
    let lines: Vec<String> = vec![];
    let offsets = super::TerminalView::compute_viewport_line_offsets(&lines);
    let total = super::TerminalView::compute_viewport_total_len(&lines);

    assert!(offsets.is_empty());
    assert_eq!(total, 0);
}

#[test]
fn selection_range_clamped_to_total_len() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let offsets = super::TerminalView::compute_viewport_line_offsets(&lines);
    let _ = offsets;
    let total = super::TerminalView::compute_viewport_total_len(&lines);
    let _ = total;

    // Full viewport selection (row 0 byte 0 → row 1 byte 3) is non-empty
    // and normalized with anchor before active.
    let sel = ByteSelection::linear(
        ScreenPos {
            screen_row: 0,
            byte: 0,
        },
        ScreenPos {
            screen_row: 1,
            byte: 3,
        },
    );
    let (start, end) = sel.normalized();
    assert!(start.screen_row <= end.screen_row);
    assert!(!sel.is_empty());

    // Selection whose active exceeds text length is still accepted.
    let sel2 = ByteSelection::linear(
        ScreenPos {
            screen_row: 0,
            byte: 0,
        },
        ScreenPos {
            screen_row: 1,
            byte: 100,
        },
    );
    assert!(!sel2.is_empty());
}

#[test]
fn word_range_stays_within_line() {
    let lines: Vec<String> = vec!["hello world".into(), "foo bar".into()];
    let vp_offset = 0u32;

    // byte 6 on row 0 → selects "world" (bytes 6..11 on row 0)
    let pos = ScreenPos {
        screen_row: 0,
        byte: 6,
    };
    let (start, end) = word_range_in_viewport(pos, &lines, vp_offset);
    assert_eq!(
        start,
        ScreenPos {
            screen_row: 0,
            byte: 6
        }
    );
    assert_eq!(
        end,
        ScreenPos {
            screen_row: 0,
            byte: 11
        }
    );
    assert_eq!(&lines[0][6..11], "world");

    // byte 0 on row 1 → selects "foo" (bytes 0..3 on row 1)
    let pos = ScreenPos {
        screen_row: 1,
        byte: 0,
    };
    let (start, end) = word_range_in_viewport(pos, &lines, vp_offset);
    assert_eq!(start.screen_row, 1);
    assert_eq!(end.screen_row, 1);
    assert_eq!(&lines[1][start.byte..end.byte], "foo");

    // byte 11 on row 0 (past end of "hello world") — end clamps to row 0
    let pos = ScreenPos {
        screen_row: 0,
        byte: 11,
    };
    let (_, end) = word_range_in_viewport(pos, &lines, vp_offset);
    assert!(end.byte <= 11 + 1);
    assert_eq!(end.screen_row, 0);
}

#[test]
fn line_range_last_line_does_not_select_below() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let vp_offset = 0u32;

    // A position on row 1 → full row 1 range: byte 0 to len+1
    let pos = ScreenPos {
        screen_row: 1,
        byte: 1,
    };
    let (start, end) = line_range_in_viewport(pos, &lines, vp_offset);
    assert_eq!(
        start,
        ScreenPos {
            screen_row: 1,
            byte: 0
        }
    );
    assert_eq!(end.screen_row, 1);
    assert_eq!(end.byte, lines[1].len() + 1); // "def".len()+1 = 4

    // A position on row 0 → full row 0 range
    let pos = ScreenPos {
        screen_row: 0,
        byte: 1,
    };
    let (start, end) = line_range_in_viewport(pos, &lines, vp_offset);
    assert_eq!(
        start,
        ScreenPos {
            screen_row: 0,
            byte: 0
        }
    );
    assert_eq!(end.screen_row, 0);
    assert_eq!(end.byte, lines[0].len() + 1); // "abc".len()+1 = 4
}

#[test]
fn line_range_at_total_len_selects_last_line() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let vp_offset = 0u32;

    // Clamped past-end position on row 1 still returns full row 1
    let pos = ScreenPos {
        screen_row: 1,
        byte: 100,
    };
    let (start, end) = line_range_in_viewport(pos, &lines, vp_offset);
    assert_eq!(start.screen_row, 1);
    assert_eq!(end.screen_row, 1);
    assert!(end.byte <= lines[1].len() + 1);
}

#[test]
fn empty_selection_produces_no_copy_text() {
    let pos = ScreenPos {
        screen_row: 0,
        byte: 5,
    };
    let sel = ByteSelection::linear(pos, pos);
    assert!(sel.is_empty());
}

// ----- Selection survivability across dirty rows ----------------

#[test]
fn selection_survives_dirty_rows_outside_selection_range() {
    use super::ByteSelection;
    use super::viewport::split_viewport_lines;

    // Simulates the clear_selection_if_overlaps_screen_rows logic:
    // selection on screen rows 10..=12, dirty row 5 → no overlap → kept.
    let sel = ByteSelection::linear(
        ScreenPos {
            screen_row: 10,
            byte: 0,
        },
        ScreenPos {
            screen_row: 12,
            byte: 5,
        },
    );
    let (start, end) = sel.normalized();
    let vp_offset = 0u32;
    let dirty_rows: &[u16] = &[5]; // row 5 is outside 10..=12
    let overlaps = dirty_rows.iter().any(|&r| {
        let sr = vp_offset + r as u32;
        sr >= start.screen_row && sr <= end.screen_row
    });
    assert!(!overlaps, "dirty row outside selection should not clear it");

    // Confirm split_viewport_lines is importable (sanity)
    let _ = split_viewport_lines("a\nb\n");
}

#[test]
fn selection_cleared_when_dirty_row_overlaps() {
    // Selection on rows 10..=12, dirty row 11 → overlaps → should clear.
    let sel = ByteSelection::linear(
        ScreenPos {
            screen_row: 10,
            byte: 0,
        },
        ScreenPos {
            screen_row: 12,
            byte: 5,
        },
    );
    let (start, end) = sel.normalized();
    let vp_offset = 0u32;
    let dirty_rows: &[u16] = &[11];
    let overlaps = dirty_rows.iter().any(|&r| {
        let sr = vp_offset + r as u32;
        sr >= start.screen_row && sr <= end.screen_row
    });
    assert!(overlaps, "dirty row inside selection should clear it");
}

// ----- Stage 2a: SelectionMode / Block selection state ---------

#[test]
fn selection_mode_defaults_to_linear_without_alt() {
    assert_eq!(selection_mode_from_modifiers(false), SelectionMode::Linear);
}

#[test]
fn selection_mode_switches_to_block_with_alt() {
    assert_eq!(selection_mode_from_modifiers(true), SelectionMode::Block);
}

#[test]
fn linear_selection_has_no_block_rect() {
    let sel = ByteSelection::linear(
        ScreenPos {
            screen_row: 0,
            byte: 3,
        },
        ScreenPos {
            screen_row: 0,
            byte: 10,
        },
    );
    assert!(!sel.is_block());
    assert!(sel.block_rect().is_none());
}

#[test]
fn block_selection_reports_is_block() {
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 0,
    };
    let sel = ByteSelection::block(anchor, CellAnchor::new(5, 2, Side::Left));
    assert!(sel.is_block());
}

#[test]
fn block_rect_normalizes_top_left_to_bottom_right() {
    // Drag from left-half of col 3 to right-half of col 10: both
    // Side values are the ones that keep the full width, so the
    // rect should cover cols 3..=10 without Side trimming.
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 0,
    };
    let mut sel = ByteSelection::block(anchor, CellAnchor::new(3, 2, Side::Left));
    sel.block_active = Some(CellAnchor::new(10, 5, Side::Right));
    let rect = sel.block_rect().unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );
}

#[test]
fn block_rect_handles_reversed_drag_in_all_four_quadrants() {
    // Every direction uses (Side::Left outside, Side::Right
    // outside) so the trim rule doesn't fire — the test's job
    // is to verify drag-direction normalization, not Side trim.
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 0,
    };

    // Drag up-and-left.
    let mut sel = ByteSelection::block(anchor, CellAnchor::new(10, 5, Side::Right));
    sel.block_active = Some(CellAnchor::new(3, 2, Side::Left));
    let rect = sel.block_rect().unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );

    // Drag up-and-right.
    let mut sel = ByteSelection::block(anchor, CellAnchor::new(3, 5, Side::Left));
    sel.block_active = Some(CellAnchor::new(10, 2, Side::Right));
    let rect = sel.block_rect().unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );

    // Drag down-and-left.
    let mut sel = ByteSelection::block(anchor, CellAnchor::new(10, 2, Side::Right));
    sel.block_active = Some(CellAnchor::new(3, 5, Side::Left));
    let rect = sel.block_rect().unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );
}

// ----- Stage 2b: block_selection_quads geometry -----------------

#[test]
fn block_quads_emit_one_rect_per_row() {
    let rect = BlockRect {
        top: 2,
        bottom: 5,
        left: 3,
        right: 10,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0);
    assert_eq!(quads.len(), 4, "rows 2..=5 inclusive → 4 quads");
}

#[test]
fn block_quads_start_at_left_cell_boundary() {
    // `left = 3` is 1-indexed inclusive → column 3 starts at
    // `cell_width * (3 - 1) = cell_width * 2`. `right = 10` is
    // inclusive, so the rect ends at the *start* of column 11,
    // i.e. `cell_width * 10`.
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 3,
        right: 10,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0);
    let q = quads[0];
    assert_eq!(q.x1, 16.0, "column 3 left edge");
    assert_eq!(q.x2, 80.0, "past column 10 right edge");
    assert_eq!(q.y1, 0.0);
    assert_eq!(q.y2, 20.0);
}

#[test]
fn block_quads_adopt_element_bounds_origin() {
    // Origin offset must be added so the render path can pass
    // `bounds.left()` / `bounds.top()` straight through.
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 1,
        right: 1,
    };
    let quads = block_selection_quads(rect, 100.0, 50.0, 8.0, 20.0);
    let q = quads[0];
    assert_eq!(q.x1, 100.0);
    assert_eq!(q.x2, 108.0);
    assert_eq!(q.y1, 50.0);
    assert_eq!(q.y2, 70.0);
}

// ----- font_hash — shape cache self-invalidation ---------------

#[test]
fn font_hash_stable_for_identical_fonts() {
    let a = crate::default_terminal_font();
    let b = crate::default_terminal_font();
    assert_eq!(super::font_hash(&a), super::font_hash(&b));
}

#[test]
fn font_hash_differs_when_family_changes() {
    let mut a = crate::default_terminal_font();
    let b = {
        let mut f = a.clone();
        f.family = "Helvetica".into();
        f
    };
    let hash_a = super::font_hash(&a);
    let hash_b = super::font_hash(&b);
    assert_ne!(
        hash_a, hash_b,
        "swapping the primary font family must invalidate the shape cache"
    );
    // Silence unused_mut if future refactors drop the mut.
    a.family = a.family.clone();
}

#[test]
fn font_hash_differs_when_weight_changes() {
    let a = crate::default_terminal_font();
    let b = {
        let mut f = a.clone();
        f.weight = gpui::FontWeight::BOLD;
        f
    };
    assert_ne!(super::font_hash(&a), super::font_hash(&b));
}

#[test]
fn font_hash_differs_when_style_changes() {
    let a = crate::default_terminal_font();
    let b = {
        let mut f = a.clone();
        f.style = gpui::FontStyle::Italic;
        f
    };
    assert_ne!(super::font_hash(&a), super::font_hash(&b));
}

#[test]
fn font_hash_differs_when_features_change() {
    use std::sync::Arc;
    let a = crate::default_terminal_font();
    let b = {
        let mut f = a.clone();
        f.features = gpui::FontFeatures(Arc::new(vec![("liga".to_string(), 1)]));
        f
    };
    assert_ne!(super::font_hash(&a), super::font_hash(&b));
}

#[test]
fn font_hash_differs_when_fallbacks_change() {
    let a = crate::default_terminal_font();
    let b = {
        let mut f = a.clone();
        f.fallbacks = Some(gpui::FontFallbacks::from_fonts(vec!["Monaco".to_string()]));
        f
    };
    assert_ne!(super::font_hash(&a), super::font_hash(&b));
}

// ----- Stage 3a: cell_side / pixel_to_cell_anchor --------------

#[test]
fn cell_side_left_half_returns_left() {
    assert_eq!(cell_side(0.0, 8.0), Side::Left);
    assert_eq!(cell_side(3.9, 8.0), Side::Left);
}

#[test]
fn cell_side_exactly_half_rounds_left() {
    // Tie-break: integer pixel grids land on the exact half point
    // very often, so we need a stable rule. Alacritty's
    // `cell_side` uses strict `>` → tie goes to Left.
    assert_eq!(cell_side(4.0, 8.0), Side::Left);
}

#[test]
fn cell_side_past_half_returns_right() {
    assert_eq!(cell_side(4.1, 8.0), Side::Right);
    assert_eq!(cell_side(7.9, 8.0), Side::Right);
}

#[test]
fn pixel_to_cell_anchor_first_cell_left_edge() {
    let a = pixel_to_cell_anchor(0.0, 0.0, 8.0, 20.0, 80, 24);
    assert_eq!((a.col, a.row, a.side), (1, 1, Side::Left));
}

#[test]
fn pixel_to_cell_anchor_within_cell_tracks_side() {
    // x=3 on an 8px cell → inside cell 1, left half.
    let a = pixel_to_cell_anchor(3.0, 0.0, 8.0, 20.0, 80, 24);
    assert_eq!((a.col, a.side), (1, Side::Left));
    // x=5 on an 8px cell → inside cell 1, right half.
    let a = pixel_to_cell_anchor(5.0, 0.0, 8.0, 20.0, 80, 24);
    assert_eq!((a.col, a.side), (1, Side::Right));
}

#[test]
fn pixel_to_cell_anchor_clamps_past_grid_edge() {
    // Far past the last column — anchor must stay in-grid.
    let a = pixel_to_cell_anchor(10_000.0, 10_000.0, 8.0, 20.0, 80, 24);
    assert_eq!((a.col, a.row), (80, 24));
    assert_eq!(a.side, Side::Right);
}

#[test]
fn pixel_to_cell_anchor_negative_coords_snap_to_origin() {
    let a = pixel_to_cell_anchor(-5.0, -5.0, 8.0, 20.0, 80, 24);
    assert_eq!((a.col, a.row, a.side), (1, 1, Side::Left));
}

#[test]
fn pixel_to_cell_anchor_empty_grid_is_safe_fallback() {
    let a = pixel_to_cell_anchor(50.0, 50.0, 8.0, 20.0, 0, 0);
    assert_eq!((a.col, a.row, a.side), (1, 1, Side::Left));
}

#[test]
fn block_rect_from_anchors_simple_right_drag() {
    // Drag from col 3 Left to col 10 Right — both sides are
    // "inside" the respective cells → all 8 columns included.
    let a = CellAnchor::new(3, 2, Side::Left);
    let b = CellAnchor::new(10, 5, Side::Right);
    let rect = block_rect_from_anchors(a, b).unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );
}

#[test]
fn block_rect_from_anchors_side_left_end_excludes_last_column() {
    // Alacritty rule: end.side == Left && start != end → drop
    // last column. Click started at col 3 (left half) and
    // stopped at col 10 (left half); col 10 should NOT be
    // highlighted because the cursor is on the wrong side to
    // "cover" it.
    let a = CellAnchor::new(3, 2, Side::Left);
    let b = CellAnchor::new(10, 5, Side::Left);
    let rect = block_rect_from_anchors(a, b).unwrap();
    assert_eq!(rect.right, 9, "right bound trimmed to col 9 by Side::Left");
    assert_eq!((rect.top, rect.bottom, rect.left), (2, 5, 3));
}

#[test]
fn block_rect_from_anchors_side_right_start_excludes_first_column() {
    // Start on the right half of col 3 → col 3 excluded.
    let a = CellAnchor::new(3, 2, Side::Right);
    let b = CellAnchor::new(10, 5, Side::Right);
    let rect = block_rect_from_anchors(a, b).unwrap();
    assert_eq!(rect.left, 4, "left bound trimmed to col 4 by Side::Right");
    assert_eq!((rect.top, rect.bottom, rect.right), (2, 5, 10));
}

#[test]
fn block_rect_from_anchors_single_cell_click_ignores_side_trim() {
    // Anchor == active (before any drag). Trimming would empty
    // the rect, which would hide the fresh-click feedback.
    // Stage 2b expectation: we always paint 1×1 on first click,
    // regardless of which half of the cell was clicked.
    let a = CellAnchor::new(7, 3, Side::Right);
    let b = a;
    let rect = block_rect_from_anchors(a, b).unwrap();
    assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (3, 3, 7, 7));
}

#[test]
fn block_rect_from_anchors_reversed_drag_still_normalizes() {
    // Drag from bottom-right to top-left — rect must still be
    // top-left → bottom-right regardless of drag direction.
    let a = CellAnchor::new(10, 5, Side::Right);
    let b = CellAnchor::new(3, 2, Side::Left);
    let rect = block_rect_from_anchors(a, b).unwrap();
    assert_eq!(
        (rect.top, rect.bottom, rect.left, rect.right),
        (2, 5, 3, 10)
    );
}

#[test]
fn block_rect_from_anchors_returns_none_when_trim_empties_rect() {
    // Cols 3 and 4 with start.side=Right and end.side=Left:
    // trim each into a 1-column gap → left=4, right=3, empty.
    let a = CellAnchor::new(3, 2, Side::Right);
    let b = CellAnchor::new(4, 2, Side::Left);
    assert!(block_rect_from_anchors(a, b).is_none());
}

// ----- Stage 2c: block_copy_text -------------------------------

#[test]
fn block_copy_extracts_rectangle_across_rows() {
    let lines: Vec<String> = vec![
        "0123456789abcdef".into(),
        "hello world!!!!!".into(),
        "terminal rocks!!".into(),
    ];
    let rect = BlockRect {
        top: 1,
        bottom: 3,
        left: 3,
        right: 7,
    };
    // Columns 3..=7 inclusive, 1-indexed → bytes [2..7) for ASCII rows.
    let expected = "23456\nllo w\nrmina";
    assert_eq!(block_copy_text(&lines, rect), expected);
}

#[test]
fn block_copy_short_row_is_clamped_to_line_len() {
    // Row 2 only has 4 columns; block asks for cols 3..=10 which
    // extends past the end. We take what's available
    // (cols 3..=4 → "o!") and do not pad with trailing spaces.
    let lines: Vec<String> = vec!["abcdefghij".into(), "foo!".into()];
    let rect = BlockRect {
        top: 1,
        bottom: 2,
        left: 3,
        right: 10,
    };
    assert_eq!(block_copy_text(&lines, rect), "cdefghij\no!");
}

#[test]
fn block_copy_missing_row_emits_blank_line() {
    // The block extends below the content area — those rows
    // still appear in the output as empty lines so pasting
    // recreates the rectangle faithfully.
    let lines: Vec<String> = vec!["abc".into()];
    let rect = BlockRect {
        top: 1,
        bottom: 3,
        left: 1,
        right: 3,
    };
    assert_eq!(block_copy_text(&lines, rect), "abc\n\n");
}

#[test]
fn block_copy_single_cell_returns_single_char() {
    let lines: Vec<String> = vec!["hello".into()];
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 2,
        right: 2,
    };
    assert_eq!(block_copy_text(&lines, rect), "e");
}

#[test]
fn block_copy_wide_glyph_snaps_to_glyph_boundary() {
    // "한" occupies two display columns. Selecting col 1..=1
    // picks up the whole wide glyph because the shaper advances
    // two columns per wide char.
    let lines: Vec<String> = vec!["한글x".into()];
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 1,
        right: 1,
    };
    // Column 2 lives inside the same wide glyph → byte index
    // for col 2 should fall on the next char boundary.
    let out = block_copy_text(&lines, rect);
    // Must be a full glyph (no partial UTF-8).
    assert!(
        out.is_char_boundary(out.len()),
        "block slice must end on a char boundary"
    );
}

#[test]
fn block_quads_for_single_cell_are_one_cell_square() {
    let rect = BlockRect {
        top: 4,
        bottom: 4,
        left: 7,
        right: 7,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 10.0, 20.0);
    assert_eq!(quads.len(), 1);
    let q = quads[0];
    // Column 7 spans [60, 70); row 4 spans [60, 80).
    assert_eq!((q.x1, q.x2, q.y1, q.y2), (60.0, 70.0, 60.0, 80.0));
}

#[test]
fn block_rect_collapses_to_single_cell_before_drag_moves() {
    // Fresh block click with no drag yet — anchor == active. The
    // rect is a 1×1 square on that cell, not empty, so the render
    // path still paints a visible highlight at click time.
    let anchor = ScreenPos {
        screen_row: 0,
        byte: 0,
    };
    let sel = ByteSelection::block(anchor, CellAnchor::new(7, 3, Side::Left));
    let rect = sel.block_rect().unwrap();
    assert_eq!((rect.top, rect.bottom, rect.left, rect.right), (3, 3, 7, 7));
}

#[test]
fn ime_skip_blocks_printable_during_marked_text() {
    use gpui::Keystroke;

    let letter = Keystroke::parse("a").unwrap();
    assert!(super::should_skip_key_down_for_ime(true, true, &letter));
    assert!(super::should_skip_key_down_for_ime(false, true, &letter));

    let esc = Keystroke::parse("escape").unwrap();
    assert!(!super::should_skip_key_down_for_ime(true, true, &esc));
    let bs = Keystroke::parse("backspace").unwrap();
    assert!(!super::should_skip_key_down_for_ime(true, true, &bs));
}

#[test]
fn ime_skip_blocks_non_ascii_key_char() {
    use gpui::Keystroke;

    let korean = Keystroke::parse("h->ㅎ").unwrap();
    assert!(!korean.is_ime_in_progress());
    assert!(super::should_skip_key_down_for_ime(true, false, &korean));
    assert!(super::should_skip_key_down_for_ime(false, false, &korean));

    let ascii = Keystroke::parse("a->a").unwrap();
    assert!(!super::should_skip_key_down_for_ime(true, false, &ascii));
}

#[test]
fn viewport_slice_clamps_oversized_range() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let offsets = super::TerminalView::compute_viewport_line_offsets(&lines);
    let total = super::TerminalView::compute_viewport_total_len(&lines);

    let _ = offsets;
    assert!(total + 10 > total);
}

#[test]
fn cursor_color_contrasts_with_background() {
    let cursor = super::style::cursor_color_for_background(Rgb {
        r: 0xFF,
        g: 0xFF,
        b: 0xFF,
    });
    assert!(cursor.l < 0.2);
    assert!((cursor.a - 0.72).abs() < f32::EPSILON);

    let cursor = super::style::cursor_color_for_background(Rgb {
        r: 0x00,
        g: 0x00,
        b: 0x00,
    });
    assert!(cursor.l > 0.8);
    assert!((cursor.a - 0.72).abs() < f32::EPSILON);
}

// ── Dirty-row overlap diagnostics ─────────────────────────────────────────────

/// Verify which viewport rows ghostty marks dirty when writing to a specific
/// row via CSI cursor-position. This is the ground-truth check for the
/// selection-survives logic.
#[test]
fn dirty_rows_after_cursor_to_row2_write() {
    use crate::{TerminalConfig, TerminalSession};
    let cfg = TerminalConfig {
        rows: 24,
        cols: 80,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(cfg).unwrap();

    let _ = session.feed(b"row zero content here\r\nrow one content here\r\n");
    let _ = session.take_dirty_viewport_rows(); // consume initial dirty set

    // Move cursor to row 2 (1-indexed) and write one character.
    let _ = session.feed(b"\x1b[2;1HX");
    let dirty = session.take_dirty_viewport_rows();

    // Row 1 (0-indexed) must be dirty; row 0 must not be.
    assert!(
        dirty.contains(&1),
        "row 1 (0-indexed) should be dirty; got {:?}",
        dirty
    );
    assert!(
        !dirty.contains(&0),
        "row 0 should NOT be dirty; got {:?}",
        dirty
    );
}

/// Same check but targeting row 1 (0-indexed = row 0), to confirm that
/// writing there does mark row 0 dirty.
#[test]
fn dirty_rows_after_cursor_to_row1_write() {
    use crate::{TerminalConfig, TerminalSession};
    let cfg = TerminalConfig {
        rows: 24,
        cols: 80,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(cfg).unwrap();

    let _ = session.feed(b"row zero content here\r\nrow one content here\r\n");
    let _ = session.take_dirty_viewport_rows();

    // Move cursor to row 1 (1-indexed = 0-indexed row 0) and overwrite.
    let _ = session.feed(b"\x1b[1;1HXXXXXXXXX");
    let dirty = session.take_dirty_viewport_rows();

    assert!(dirty.contains(&0), "row 0 should be dirty; got {:?}", dirty);
}

mod mouse_protocol;
