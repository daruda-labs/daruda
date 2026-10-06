//! Keyboard-input tests for `TerminalView`.
//!
//! These tests verify that keystrokes produce the correct byte sequences
//! by intercepting the `TerminalInput` sink instead of using a real PTY.
//!
//! Run with:
//!   cargo test -p visual_tests keyboard -- --nocapture

use std::sync::{Arc, Mutex};

use gpui::TestAppContext;

use crate::common::{drain, feed, focus, open_terminal_with_sink};

/// Enter → CR, Escape → ESC, and Ctrl-C → ETX (0x03).
#[gpui::test]
async fn test_special_keys(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(&view, cx, b"\x1b[32m$\x1b[0m ");
    focus(&view, cx);

    cx.simulate_keystrokes("enter");
    assert_eq!(drain(&sink), b"\r", "Enter must send CR");

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(drain(&sink), b"\x03", "Ctrl-C must send ETX");

    cx.simulate_keystrokes("escape");
    assert_eq!(drain(&sink), b"\x1b", "Escape must send ESC");
}

/// Arrow keys → ANSI cursor sequences (no application-cursor mode).
#[gpui::test]
async fn test_arrow_keys(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(&view, cx, b"$ ");
    focus(&view, cx);

    let cases: &[(&str, &[u8], &str)] = &[
        ("up", b"\x1b[A", "Up"),
        ("down", b"\x1b[B", "Down"),
        ("right", b"\x1b[C", "Right"),
        ("left", b"\x1b[D", "Left"),
    ];

    for (key, expected, label) in cases {
        cx.simulate_keystrokes(key);
        let got = drain(&sink);
        assert_eq!(
            &got, *expected,
            "{label} arrow should send {expected:?}, got {got:?}"
        );
    }
}

/// Tab → HT (0x09), Backspace → DEL (0x7f) or BS (0x08).
#[gpui::test]
async fn test_tab_and_backspace(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(&view, cx, b"$ ");
    focus(&view, cx);

    cx.simulate_keystrokes("tab");
    let got = drain(&sink);
    assert_eq!(got, b"\t", "Tab must send HT (0x09), got {got:?}");

    cx.simulate_keystrokes("backspace");
    let got = drain(&sink);
    assert!(
        got == b"\x7f" || got == b"\x08",
        "Backspace must send DEL or BS, got {got:?}"
    );
}

/// Ctrl+letter combos → corresponding control codes (SOH … SUB).
#[gpui::test]
async fn test_ctrl_letter_keys(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(&view, cx, b"$ ");
    focus(&view, cx);

    let cases: &[(&str, u8, &str)] = &[
        ("ctrl-a", 0x01, "Ctrl-A → SOH"),
        ("ctrl-d", 0x04, "Ctrl-D → EOT"),
        ("ctrl-l", 0x0c, "Ctrl-L → FF"),
        ("ctrl-u", 0x15, "Ctrl-U → NAK"),
        ("ctrl-z", 0x1a, "Ctrl-Z → SUB"),
    ];

    for (key, byte, label) in cases {
        cx.simulate_keystrokes(key);
        let got = drain(&sink);
        assert_eq!(got, vec![*byte], "{label}: got {got:?}");
    }
}

/// `simulate_input` types printable text; each character is dispatched as a
/// keystroke with `key_char` set, which routes through the IME / key-down path
/// in `TerminalView`.
///
/// In the GPUI test platform the IME subsystem is not present, so printable
/// input typically does NOT reach the PTY sink. This test verifies that
/// dispatching such events does not panic and the view remains consistent.
#[gpui::test]
async fn test_printable_input_no_panic(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(
        &view,
        cx,
        b"Welcome to daruda\r\nline two\r\nline three\r\n$ ",
    );
    focus(&view, cx);

    // Must not panic. Byte output is empty in the mock platform (no IME).
    cx.simulate_input("hello world");

    // View is still in a consistent state: title and cwd accessors do not panic.
    view.update(cx, |tv, _| {
        let _ = tv.terminal_title();
        let _ = tv.terminal_cwd();
    });
}

/// Multiple rapid keystrokes do not cause reentrant panics.
#[gpui::test]
async fn test_rapid_keystrokes(cx: &mut TestAppContext) {
    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (view, cx) = open_terminal_with_sink(cx, sink.clone());

    feed(&view, cx, b"$ ");
    focus(&view, cx);

    for _ in 0..20 {
        cx.simulate_keystrokes("up");
    }

    let sent = sink.lock().unwrap();
    // Every Up arrow must have produced the ANSI escape.
    assert_eq!(sent.len(), 20 * b"\x1b[A".len());
    for chunk in sent.chunks(3) {
        assert_eq!(chunk, b"\x1b[A");
    }
}
