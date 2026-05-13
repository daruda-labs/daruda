//! Transparent background coverage tests for `TerminalView`.
//!
//! In transparent mode every cell must have exactly one background quad.
//! Ghostty_vt only emits style runs for cells that have been explicitly
//! written; trailing empty columns (e.g. "ls" → cols 1-2, cols 3-80 empty)
//! have no run and would show as fully transparent if not filled.
//!
//! Run with:
//!   cargo test -p visual_tests background -- --nocapture

use gpui::TestAppContext;

use crate::common::{feed, open_terminal};

/// After feeding short text, the unfilled trailing columns on that row
/// must still have background coverage in transparent mode.
#[gpui::test]
async fn transparent_trailing_cols_have_background(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);
    view.update(cx, |tv, _| tv.set_background_alpha(0.5));
    // "ls" occupies only cols 1-2; cols 3..=cols are trailing empty space
    // that ghostty_vt emits no style runs for.
    feed(&view, cx, b"ls");
    let covered = view.update(cx, |tv, _| tv.transparent_bg_covers_row(0));
    assert!(
        covered,
        "row with short text must have full-width bg coverage in transparent mode"
    );
}

/// A completely empty row (below any shell output) must be covered too.
#[gpui::test]
async fn transparent_empty_row_has_background(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);
    view.update(cx, |tv, _| tv.set_background_alpha(0.5));
    // Feed nothing — row 0 has no style runs at all.
    let covered = view.update(cx, |tv, _| tv.transparent_bg_covers_row(0));
    assert!(
        covered,
        "empty row must have full-width bg coverage in transparent mode"
    );
}

/// Multiple rows each covering different amounts of text must all be
/// fully covered.
#[gpui::test]
async fn transparent_mixed_rows_all_covered(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);
    view.update(cx, |tv, _| tv.set_background_alpha(0.7));
    // Row 0: short command; row 1: empty; row 2: longer output.
    feed(&view, cx, b"ls\r\n\r\nhello world\r\n");
    for row in 0..3 {
        let covered = view.update(cx, |tv, _| tv.transparent_bg_covers_row(row));
        assert!(
            covered,
            "row {row} must have full-width bg coverage in transparent mode"
        );
    }
}

/// Opaque mode is unaffected — the check only applies to the transparent
/// path, but `transparent_bg_covers_row` must still return true for opaque
/// sessions (coverage is provided by the viewport's own fill).
#[gpui::test]
async fn opaque_mode_coverage_check_passes(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);
    // Default alpha = 1.0 (opaque).
    feed(&view, cx, b"ls");
    let covered = view.update(cx, |tv, _| tv.transparent_bg_covers_row(0));
    assert!(covered);
}
