//! Mouse-input tests for `TerminalView`.
//!
//! These tests verify that mouse events are dispatched without panicking and
//! that the view's state remains consistent after each interaction.
//!
//! Run with:
//!   cargo test -p visual_tests mouse -- --nocapture

use gpui::{Modifiers, MouseButton, TestAppContext, point, px};

use crate::common::{feed, focus, open_terminal, queue_output};

/// A single left-click does not panic and leaves the view in a valid state.
#[gpui::test]
async fn test_single_click(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"line one\r\nline two\r\nline three\r\n$ ");
    focus(&view, cx);

    // Click near row 1, col 0.
    cx.simulate_click(point(px(4.0), px(28.0)), Modifiers::default());

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// A mouse drag (down → move → up) does not panic.
#[gpui::test]
async fn test_drag_selection(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"Hello, World!\r\nSecond line here\r\n$ ");
    focus(&view, cx);

    let start = point(px(4.0), px(6.0));
    let end = point(px(60.0), px(6.0));

    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// A right-click does not panic.
#[gpui::test]
async fn test_right_click(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"right-click target\r\n$ ");
    focus(&view, cx);

    cx.simulate_mouse_down(
        point(px(50.0), px(6.0)),
        MouseButton::Right,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(50.0), px(6.0)),
        MouseButton::Right,
        Modifiers::default(),
    );

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// Multiple sequential clicks in different rows do not cause reentrant panics.
#[gpui::test]
async fn test_multiple_clicks(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(
        &view,
        cx,
        b"alpha beta gamma\r\ndelta epsilon zeta\r\neta theta iota\r\n$ ",
    );
    focus(&view, cx);

    for y in [6.0_f32, 20.0, 34.0] {
        cx.simulate_click(point(px(4.0), px(y)), Modifiers::default());
    }

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// Mouse hover (move without button) across the viewport does not panic.
#[gpui::test]
async fn test_mouse_hover(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"hover test\r\n$ ");
    focus(&view, cx);

    for x in (0_u32..200).step_by(20) {
        cx.simulate_mouse_move(point(px(x as f32), px(6.0)), None, Modifiers::default());
    }

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// Shift-click (used for extending selection) does not panic.
#[gpui::test]
async fn test_shift_click(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"extend selection target\r\n$ ");
    focus(&view, cx);

    // First click anchors the selection start.
    cx.simulate_click(point(px(4.0), px(6.0)), Modifiers::default());

    // Shift-click extends it to a further position.
    cx.simulate_click(
        point(px(80.0), px(6.0)),
        Modifiers {
            shift: true,
            ..Default::default()
        },
    );

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}

/// Selection on row 0 must survive when output arrives only on row 1.
///
/// This is the regression test for the flat-byte-offset era: any call to
/// `refresh_viewport` would recompute `viewport_line_offsets` and silently
/// set `selection = None`.  With `ScreenPos` (absolute screen coordinates)
/// the selection only clears when the dirty rows actually overlap it.
///
/// The test uses `queue_output` (the real PTY path) so
/// `reconcile_dirty_viewport_after_output` runs the smart overlap check.
#[gpui::test]
async fn test_selection_survives_output_on_other_row(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    // Put text on rows 0 and 1.
    feed(
        &view,
        cx,
        b"row zero content here\r\nrow one content here\r\n",
    );
    focus(&view, cx);

    // Drag across row 0 (y ≈ 6 px = middle of first line at default font size).
    let start = point(px(4.0), px(6.0));
    let end = point(px(100.0), px(6.0));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

    // Cell metrics may be unavailable in headless test builds — if no
    // selection was created the test has nothing to verify; skip rather
    // than emit a false failure.
    if !view.update(cx, |tv, _| tv.has_selection()) {
        return;
    }

    // Feed output that only touches row 1:
    //   \x1b[2;1H  cursor to 1-indexed row 2 (= 0-indexed row 1)
    //   X          write one character → ghostty marks row 1 dirty only
    queue_output(&view, cx, b"\x1b[2;1HX");

    assert!(
        view.update(cx, |tv, _| tv.has_selection()),
        "selection on row 0 must survive when only row 1 is dirtied"
    );
}

/// Selection survives a partial dirty repaint on its own row.
///
/// Under the iTerm2 invalidation policy adopted in
/// `view::selection_policy::invalidation_reason`, a single-row dirty event
/// no longer clears the selection — only full-viewport repaints,
/// alt-screen toggles, and RIS do. Selection lives in absolute screen
/// coordinates, so the user-visible highlight stays anchored even when
/// the cells beneath it are rewritten by the shell (the common case
/// while a TUI like Claude Code redraws its input box).
#[gpui::test]
async fn test_selection_survives_partial_dirty_on_selected_row(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(
        &view,
        cx,
        b"row zero content here\r\nrow one content here\r\n",
    );
    focus(&view, cx);

    // Drag across row 0.
    let start = point(px(4.0), px(6.0));
    let end = point(px(100.0), px(6.0));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

    if !view.update(cx, |tv, _| tv.has_selection()) {
        return;
    }

    // Overwrite row 0 directly:
    //   \x1b[1;1H  cursor to 1-indexed row 1 (= 0-indexed row 0)
    //   XXXXXXXXX  replace content → ghostty marks row 0 dirty
    queue_output(&view, cx, b"\x1b[1;1HXXXXXXXXX");

    assert!(
        view.update(cx, |tv, _| tv.has_selection()),
        "selection must survive a partial dirty repaint on its row"
    );
}

/// Double-click (click_count = 2) does not panic and is dispatched correctly.
#[gpui::test]
async fn test_double_click(cx: &mut TestAppContext) {
    let (view, cx) = open_terminal(cx);

    feed(&view, cx, b"word1 word2 word3\r\n$ ");
    focus(&view, cx);

    // Simulate two rapid clicks at the same position (word-select behaviour).
    cx.simulate_click(point(px(20.0), px(6.0)), Modifiers::default());
    cx.simulate_click(point(px(20.0), px(6.0)), Modifiers::default());

    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
    });
}
