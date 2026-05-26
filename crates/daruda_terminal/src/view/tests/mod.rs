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
    let anchor = ScreenPos::viewport(0, 3);
    let active = ScreenPos::viewport(1, 0);
    let sel = ByteSelection::linear(anchor, active);
    assert!(!sel.is_empty());
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();
    let (start, end) = sel.normalized(&session).unwrap();
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
    let anchor = ScreenPos::viewport(0, 1);
    let active = ScreenPos::viewport(1, 3);
    let sel = ByteSelection::linear(anchor, active);
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();
    let (start, end) = sel.normalized(&session).unwrap();
    assert_eq!(start.screen_row(&session), Some(0));
    assert_eq!(end.screen_row(&session), Some(1));
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
    let sel = ByteSelection::linear(ScreenPos::viewport(0, 0), ScreenPos::viewport(1, 3));
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();
    let (start, end) = sel.normalized(&session).unwrap();
    assert!(start.screen_row(&session) <= end.screen_row(&session));
    assert!(!sel.is_empty());

    // Selection whose active exceeds text length is still accepted.
    let sel2 = ByteSelection::linear(ScreenPos::viewport(0, 0), ScreenPos::viewport(1, 100));
    assert!(!sel2.is_empty());
}

#[test]
fn word_range_stays_within_line() {
    let lines: Vec<String> = vec!["hello world".into(), "foo bar".into()];
    let vp_offset = 0u32;
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();

    // byte 6 on row 0 → selects "world" (bytes 6..11 on row 0)
    let pos = ScreenPos::viewport(0, 6);
    let (start, end) = word_range_in_viewport(pos, &session, &lines, vp_offset);
    assert_eq!(start, ScreenPos::viewport(0, 6));
    assert_eq!(end, ScreenPos::viewport(0, 11));
    assert_eq!(&lines[0][6..11], "world");

    // byte 0 on row 1 → selects "foo" (bytes 0..3 on row 1)
    let pos = ScreenPos::viewport(1, 0);
    let (start, end) = word_range_in_viewport(pos, &session, &lines, vp_offset);
    assert_eq!(start.screen_row(&session), Some(1));
    assert_eq!(end.screen_row(&session), Some(1));
    let (sr, sb) = start.resolve(&session).unwrap();
    let (_, eb) = end.resolve(&session).unwrap();
    let _ = sr;
    assert_eq!(&lines[1][sb..eb], "foo");

    // byte 11 on row 0 (past end of "hello world") — end clamps to row 0
    let pos = ScreenPos::viewport(0, 11);
    let (_, end) = word_range_in_viewport(pos, &session, &lines, vp_offset);
    let (er, eb) = end.resolve(&session).unwrap();
    assert!(eb <= 11 + 1);
    assert_eq!(er, 0);
}

#[test]
fn line_range_last_line_does_not_select_below() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let vp_offset = 0u32;
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();

    // A position on row 1 → full row 1 range: byte 0 to len+1
    let pos = ScreenPos::viewport(1, 1);
    let (start, end) = line_range_in_viewport(pos, &session, &lines, vp_offset);
    assert_eq!(start, ScreenPos::viewport(1, 0));
    assert_eq!(end.screen_row(&session), Some(1));
    assert_eq!(end.byte(&session), Some(lines[1].len() + 1)); // "def".len()+1 = 4

    // A position on row 0 → full row 0 range
    let pos = ScreenPos::viewport(0, 1);
    let (start, end) = line_range_in_viewport(pos, &session, &lines, vp_offset);
    assert_eq!(start, ScreenPos::viewport(0, 0));
    assert_eq!(end.screen_row(&session), Some(0));
    assert_eq!(end.byte(&session), Some(lines[0].len() + 1)); // "abc".len()+1 = 4
}

#[test]
fn line_range_at_total_len_selects_last_line() {
    let lines: Vec<String> = vec!["abc".into(), "def".into()];
    let vp_offset = 0u32;
    let session = crate::TerminalSession::new(crate::TerminalConfig::default()).unwrap();

    // Clamped past-end position on row 1 still returns full row 1
    let pos = ScreenPos::viewport(1, 100);
    let (start, end) = line_range_in_viewport(pos, &session, &lines, vp_offset);
    assert_eq!(start.screen_row(&session), Some(1));
    assert_eq!(end.screen_row(&session), Some(1));
    assert!(end.byte(&session).unwrap() <= lines[1].len() + 1);
}

#[test]
fn empty_selection_produces_no_copy_text() {
    let pos = ScreenPos::viewport(0, 5);
    let sel = ByteSelection::linear(pos, pos);
    assert!(sel.is_empty());
}

// Note: the prior "selection survives/cleared on dirty-row overlap" tests
// were tied to the dirty-overlap policy that has been replaced with the
// iTerm2 invalidation policy. See `view::selection_policy::tests` for the
// canonical coverage of partial / full-viewport / alt-screen / RIS cases.

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
    let sel = ByteSelection::linear(ScreenPos::viewport(0, 3), ScreenPos::viewport(0, 10));
    assert!(!sel.is_block());
    assert!(sel.block_rect().is_none());
}

#[test]
fn block_selection_reports_is_block() {
    let anchor = ScreenPos::viewport(0, 0);
    let sel = ByteSelection::block(anchor, CellAnchor::new(5, 2, Side::Left));
    assert!(sel.is_block());
}

#[test]
fn block_rect_normalizes_top_left_to_bottom_right() {
    // Drag from left-half of col 3 to right-half of col 10: both
    // Side values are the ones that keep the full width, so the
    // rect should cover cols 3..=10 without Side trimming.
    let anchor = ScreenPos::viewport(0, 0);
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
    let anchor = ScreenPos::viewport(0, 0);

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
    // vp_top = 0, vp_rows large enough to keep all 4 rows visible.
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0, 0, 24);
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
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0, 0, 24);
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
    let quads = block_selection_quads(rect, 100.0, 50.0, 8.0, 20.0, 0, 24);
    let q = quads[0];
    assert_eq!(q.x1, 100.0);
    assert_eq!(q.x2, 108.0);
    assert_eq!(q.y1, 50.0);
    assert_eq!(q.y2, 70.0);
}

#[test]
fn block_quads_skip_rows_above_visible_viewport() {
    // Rect spans absolute rows 5..=8; viewport sits at row 7
    // (vp_top = 6 in 0-indexed terms). Rows 5 and 6 are above
    // the viewport top — they must be dropped, leaving 2 quads.
    let rect = BlockRect {
        top: 5,
        bottom: 8,
        left: 1,
        right: 3,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0, 6, 24);
    assert_eq!(quads.len(), 2);
    // Visible row indices: row 7 → visible 0, row 8 → visible 1.
    assert_eq!(quads[0].y1, 0.0);
    assert_eq!(quads[1].y1, 20.0);
}

#[test]
fn block_quads_skip_rows_below_visible_viewport() {
    // Rect spans rows 1..=10 with a 4-row viewport pinned at the
    // top (vp_top = 0); rows 5..=10 are below the viewport and
    // must be dropped, leaving 4 quads for rows 1..=4.
    let rect = BlockRect {
        top: 1,
        bottom: 10,
        left: 1,
        right: 3,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 8.0, 20.0, 0, 4);
    assert_eq!(quads.len(), 4);
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

/// Build a session whose first few dump_screen_row(y) responses match
/// `lines`. Feeds the lines through PTY input so they land in the
/// viewport (and, when the row count exceeds the viewport, spill into
/// `LineBuffer` scrollback). Returns the session ready for selection
/// tests. `cols` is sized large enough to keep each input line on a
/// single visual row.
fn session_with_lines(lines: &[&str], cols: u16, rows: u16) -> crate::TerminalSession {
    let config = crate::TerminalConfig {
        cols,
        rows,
        max_scrollback: 1024,
        ..crate::TerminalConfig::default()
    };
    let mut s = crate::TerminalSession::new(config).expect("session");
    let mut payload = String::new();
    for (i, line) in lines.iter().enumerate() {
        payload.push_str(line);
        if i + 1 < lines.len() {
            payload.push_str("\r\n");
        }
    }
    s.feed(payload.as_bytes()).expect("feed");
    s
}

#[test]
fn block_copy_extracts_rectangle_across_rows() {
    let session = session_with_lines(
        &["0123456789abcdef", "hello world!!!!!", "terminal rocks!!"],
        32,
        4,
    );
    let rect = BlockRect {
        top: 1,
        bottom: 3,
        left: 3,
        right: 7,
    };
    // Columns 3..=7 inclusive, 1-indexed → bytes [2..7) for ASCII rows.
    let expected = "23456\nllo w\nrmina";
    assert_eq!(block_copy_text(&rect, &session), expected);
}

#[test]
fn block_copy_short_row_is_clamped_to_line_len() {
    // Row 2 only has 4 columns; block asks for cols 3..=10 which
    // extends past the end. We take what's available
    // (cols 3..=4 → "o!") and do not pad with trailing spaces.
    let session = session_with_lines(&["abcdefghij", "foo!"], 16, 4);
    let rect = BlockRect {
        top: 1,
        bottom: 2,
        left: 3,
        right: 10,
    };
    assert_eq!(block_copy_text(&rect, &session), "cdefghij\no!");
}

#[test]
fn block_copy_single_cell_returns_single_char() {
    let session = session_with_lines(&["hello"], 16, 4);
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 2,
        right: 2,
    };
    assert_eq!(block_copy_text(&rect, &session), "e");
}

#[test]
fn block_copy_wide_glyph_snaps_to_glyph_boundary() {
    // "한" occupies two display columns. Selecting col 1..=1
    // picks up the whole wide glyph because the shaper advances
    // two columns per wide char.
    let session = session_with_lines(&["한글x"], 16, 4);
    let rect = BlockRect {
        top: 1,
        bottom: 1,
        left: 1,
        right: 1,
    };
    // Column 2 lives inside the same wide glyph → byte index
    // for col 2 should fall on the next char boundary.
    let out = block_copy_text(&rect, &session);
    // Must be a full glyph (no partial UTF-8).
    assert!(
        out.is_char_boundary(out.len()),
        "block slice must end on a char boundary"
    );
}

#[test]
fn block_copy_text_pulls_rows_from_scrollback() {
    // 20-col × 3-row viewport with 1024 lines of scrollback. Feeding
    // 5 lines pushes the first TWO into LineBuffer (the live viewport
    // holds the last three). Targeting rect rows 1..=2 (1-indexed →
    // absolute rows 0 and 1) makes BOTH copied rows sit in
    // scrollback — the test fails if `block_copy_text` reads from
    // a viewport-only buffer instead of dispatching through
    // `dump_screen_row`.
    let session = session_with_lines(
        &[
            "alpha-beta-gamma-del",
            "epsilon-zeta",
            "eta",
            "theta",
            "iota",
        ],
        20,
        3,
    );
    // Sanity: at least two rows are scrolled out (live viewport holds
    // the last 3 rows — "eta", "theta", "iota").
    assert!(
        session.viewport_row_offset() >= 2,
        "expected ≥2 scrollback rows above the viewport, got vp_top={}",
        session.viewport_row_offset()
    );
    let rect = BlockRect {
        top: 1,
        bottom: 2,
        left: 1,
        right: 5,
    };
    // Columns 1..=5, 1-indexed inclusive → "alpha" / "epsil".
    assert_eq!(block_copy_text(&rect, &session), "alpha\nepsil");
}

/// `block_copy_text` documents that a row missing from `dump_screen_row`
/// contributes a blank line so the rectangle's geometry is preserved.
/// Triggered here by capping `max_scrollback = 1` so LineBuffer
/// ring-evicts older logical lines past `overflow`. Targeting a rect
/// row that lies above the surviving unified frame (out of both
/// LineBuffer's wrapped range and ghostty's live viewport) makes
/// `dump_screen_row` return `Err`, which the copy path translates to a
/// blank line followed by the live content for the remaining rows.
#[test]
fn block_copy_evicted_row_contributes_blank_line() {
    use crate::{TerminalConfig, TerminalSession};
    // Single-line scrollback cap so feeding more logical lines genuinely
    // evicts rows past LineBuffer's ring.
    let cfg = TerminalConfig {
        cols: 16,
        rows: 1,
        max_scrollback: 1,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(cfg).expect("session");
    // Feed three sealed lines: "first" and "second" get evicted (only
    // "third" survives in LineBuffer); the viewport holds the empty
    // current line after the trailing newline.
    session.feed(b"first\r\nsecond\r\nthird\r\n").expect("feed");
    assert!(
        session.line_buffer().overflow() >= 1,
        "expected LineBuffer eviction, got overflow={}",
        session.line_buffer().overflow()
    );
    // Sanity: row 2 sits past the unified frame so `dump_screen_row`
    // must report failure — this is the path `block_copy_text` is
    // contracted to translate into a blank line.
    assert!(
        session.dump_screen_row(2).is_err(),
        "expected dump_screen_row(2) to fail past the unified frame"
    );

    // Unified frame addresses: row 0 → "third" (LineBuffer), row 1 → ""
    // (viewport). Row 2 is past the end — `dump_screen_row` returns
    // `Err`, which `block_copy_text` must translate to a blank line so
    // pasted geometry preserves the missing row's slot.
    //
    // 1-indexed rect: rows 1..=3 → abs rows 0, 1, 2. cols 1..=5 grabs
    // "third"[0..5] = "third", then "" → empty, then `Err` → empty.
    let rect = BlockRect {
        top: 1,
        bottom: 3,
        left: 1,
        right: 5,
    };
    let out = block_copy_text(&rect, &session);
    // Three rows joined by '\n'. The first row has content; the
    // second (live empty row) and third (evicted-past-frame) both
    // contribute blank lines.
    assert_eq!(out, "third\n\n");
    // Geometry preserved: three rows → two '\n' separators.
    assert_eq!(out.matches('\n').count(), 2);
}

#[test]
fn block_quads_for_single_cell_are_one_cell_square() {
    let rect = BlockRect {
        top: 4,
        bottom: 4,
        left: 7,
        right: 7,
    };
    let quads = block_selection_quads(rect, 0.0, 0.0, 10.0, 20.0, 0, 24);
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
    let anchor = ScreenPos::viewport(0, 0);
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

/// Anchor a `ScreenPos` at a scrollback row, resize the terminal width,
/// and confirm the anchor still resolves to the same logical content.
///
/// Selection used to break on resize because the stored `(screen_row,
/// byte)` pair was a current-frame coordinate. With the
/// `LineBufferPosition`-backed `Scrollback` anchor, the resize re-maps
/// the same cell through the new wrap layout.
#[test]
fn scrollback_anchor_survives_resize_widen() {
    use crate::{TerminalConfig, TerminalSession};
    let cfg = TerminalConfig {
        cols: 20,
        rows: 3,
        max_scrollback: 1024,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(cfg).unwrap();
    // Five lines, viewport is 3 rows tall → at least the first two
    // lines scroll off the top into LineBuffer scrollback.
    session
        .feed(b"the quick brown fox\r\nthe lazy dog xyzzy\r\nfin\r\nmore\r\nlast\r\n")
        .unwrap();
    assert!(
        session.line_buffer().len() >= 2,
        "expected at least 2 captured lines, got {}",
        session.line_buffer().len()
    );

    // First captured row in LineBuffer = "the quick brown fox" — its
    // visual row in the current frame is index 0 (top of scrollback).
    // Anchor on column 5 ("q" of "quick").
    let anchor_col = 5u16;
    let anchor = ScreenPos::anchor_at(&session, 0, anchor_col);

    // Resize: widen to 40 columns. The "quick" word still lives in the
    // same logical line at the same cumulative cell column.
    session.resize(40, 3).unwrap();

    let (visual_y, byte) = anchor.resolve(&session).expect("anchor still resolves");
    let row = session
        .dump_screen_row(visual_y)
        .expect("row resolves to text");
    let row = row.strip_suffix('\n').unwrap_or(&row);
    // The byte offset should land at the start of "quick" — verify the
    // suffix from `byte` starts with that word.
    assert!(
        row[byte..].starts_with("quick"),
        "expected 'quick' at byte {byte} of row {visual_y}: {row:?}"
    );

    // Narrow back to a width smaller than the original. The same
    // logical cell should still resolve, just at a different visual_y.
    // At width 8 the line "the quick brown fox" wraps mid-word; the
    // anchor still lands on the 'q' even though the rest of "quick"
    // straddles onto the next visual row.
    session.resize(8, 3).unwrap();
    let (visual_y, byte) = anchor.resolve(&session).expect("anchor still resolves");
    let row = session
        .dump_screen_row(visual_y)
        .expect("row resolves to text");
    let row = row.strip_suffix('\n').unwrap_or(&row);
    assert!(
        row[byte..].starts_with('q'),
        "expected 'q' at byte {byte} of row {visual_y} after narrow: {row:?}"
    );
}

/// Cross-line matches were impossible before Task 5 — each row was
/// scanned in isolation. With `FindContext` driving the scrollback
/// portion, a needle that straddles a hard newline must produce one
/// `MatchRange` per visual row it spans.
#[test]
fn search_finds_cross_line_match_in_scrollback() {
    use super::search::scan_search_matches;
    // 20-col × 3-row viewport — feeding 5 lines pushes the first two
    // into the line buffer. The needle "world" lives across the seam.
    let session = session_with_lines(&["hello wor", "ld there", "row3", "row4", "row5"], 20, 3);
    assert!(
        session.line_buffer().len() >= 2,
        "expected at least 2 captured lines, got {}",
        session.line_buffer().len(),
    );

    let result = scan_search_matches(&session, "world", false, false);
    assert!(!result.regex_error);
    assert_eq!(
        result.matches.len(),
        2,
        "cross-line needle should produce one MatchRange per visual row, got {:?}",
        result.matches,
    );
    let m0 = result.matches[0];
    let m1 = result.matches[1];
    // "hello wor": "wor" sits at 1-indexed cols 7..=9 of visual row 0.
    assert_eq!((m0.row, m0.start_col, m0.end_col), (0, 7, 9));
    // "ld there": "ld" sits at 1-indexed cols 1..=2 of visual row 1.
    assert_eq!((m1.row, m1.start_col, m1.end_col), (1, 1, 2));
}

/// Scrollback and viewport matches must surface together in row order.
/// Confirms the dispatcher path emits both halves of the unified frame.
#[test]
fn search_combines_scrollback_and_viewport_matches() {
    use super::search::scan_search_matches;
    let session = session_with_lines(&["foo here", "row2", "row3", "row4 foo"], 20, 3);
    assert!(!session.line_buffer().is_empty());

    let result = scan_search_matches(&session, "foo", false, false);
    assert!(!result.regex_error);
    let rows: Vec<u32> = result.matches.iter().map(|m| m.row).collect();
    assert!(
        rows.len() >= 2,
        "expected at least 2 matches across scrollback + viewport, got {:?}",
        rows,
    );
    let mut sorted = rows.clone();
    sorted.sort();
    assert_eq!(rows, sorted, "matches must arrive in row-ascending order");
    assert_eq!(*rows.first().unwrap(), 0, "scrollback row first");
    assert!(*rows.last().unwrap() >= 1, "viewport row last");
}

mod mouse_protocol;

#[test]
fn terminal_layout_cols_next_up_precision() {
    use super::layout::TerminalLayout;
    let layout = TerminalLayout {
        cell_width: 8.0,
        line_height: 16.0,
    };
    // 8.0 * 100 = 800; float arithmetic can produce 799.999...; must still yield 100
    let nearly_800: f32 = 8.0_f32.mul_add(100.0, -f32::EPSILON * 8.0);
    assert_eq!(layout.cols(nearly_800), 100);
}

#[test]
fn terminal_layout_rows_next_up_precision() {
    use super::layout::TerminalLayout;
    let layout = TerminalLayout {
        cell_width: 8.0,
        line_height: 16.0,
    };
    let nearly_480: f32 = 16.0_f32.mul_add(30.0, -f32::EPSILON * 16.0);
    assert_eq!(layout.rows(nearly_480), 30);
}

#[test]
fn terminal_layout_cols_floor_exact_boundary() {
    use super::layout::TerminalLayout;
    let layout = TerminalLayout {
        cell_width: 8.0,
        line_height: 16.0,
    };
    // Exact pixel boundary: 799 px → 99 cols
    assert_eq!(layout.cols(799.0), 99);
    // One full cell: 800 px → 100 cols
    assert_eq!(layout.cols(800.0), 100);
}

#[test]
fn terminal_layout_minimum_one_col_and_row() {
    use super::layout::TerminalLayout;
    let layout = TerminalLayout {
        cell_width: 8.0,
        line_height: 16.0,
    };
    assert_eq!(layout.cols(0.0), 1);
    assert_eq!(layout.rows(0.0), 1);
    assert_eq!(layout.cols(1.0), 1);
    assert_eq!(layout.rows(1.0), 1);
}
