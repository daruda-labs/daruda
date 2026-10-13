use super::*;

// ============================================================================
// Command history (B → C → D walking)
// ============================================================================

// Single-row command typed at the prompt.
//   `$ git status<Enter>` →
//      A B git status C D
// The B mark fires after `$ `, the user types, then C fires after
// the command is sent to the shell. command_history() should slice
// "git status" out of the row by the captured columns.
#[test]
fn command_history_extracts_single_row_command() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Prompt + B mark, user types, C mark, D mark with exit 0.
    session.feed(b"$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
    session.feed(b"git status").unwrap();
    session.feed(b"\x1b]133;C\x07\x1b]133;D;0\x07").unwrap();

    let history = session.command_history();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].command_text, "git status");
    assert_eq!(history[0].exit_code, Some(0));
}

// Non-zero exit propagates from D.
#[test]
fn command_history_propagates_nonzero_exit_code() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
    session.feed(b"false").unwrap();
    session.feed(b"\x1b]133;C\x07\x1b]133;D;1\x07").unwrap();
    let history = session.command_history();
    assert_eq!(history[0].exit_code, Some(1));
}

// CommandStart without a CommandExecuted is dropped — the user
// closed the prompt with ^C before pressing Enter.
#[test]
fn command_history_drops_unfinished_command_start() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
    session.feed(b"git st").unwrap();
    // No C — user aborted with ^C, next prompt drawn instead.
    session.feed(b"\r\n$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
    let history = session.command_history();
    assert!(
        history.is_empty(),
        "abandoned command must not surface in history: {history:?}"
    );
}

// Multiple commands, oldest-first ordering.
#[test]
fn command_history_returns_entries_in_chronological_order() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    for cmd in ["echo 1", "echo 2", "echo 3"] {
        session.feed(b"$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
        session.feed(cmd.as_bytes()).unwrap();
        session.feed(b"\x1b]133;C\x07\x1b]133;D;0\x07").unwrap();
        // Move cursor to next line so the next prompt lands on a
        // fresh row and the screen_row column matches reality.
        session.feed(b"\r\n").unwrap();
    }
    let history = session.command_history();
    assert_eq!(
        history
            .iter()
            .map(|e| e.command_text.as_str())
            .collect::<Vec<_>>(),
        vec!["echo 1", "echo 2", "echo 3"]
    );
}

// Whitespace-only command is dropped (B and C captured the same
// column → empty slice). Mirrors iTerm2 history which never
// records "the user pressed Enter on an empty line".
#[test]
fn command_history_drops_empty_command() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"$ \x1b]133;A\x07\x1b]133;B\x07").unwrap();
    // No bytes between B and C.
    session.feed(b"\x1b]133;C\x07\x1b]133;D;0\x07").unwrap();
    assert!(session.command_history().is_empty());
}

// Regression: a long prompt with `B` and `C` at the same column on
// the same row must drop the entry. Without the `end <= start` guard
// inside `slice_chars`, the extractor hallucinated a single-char
// command (the byte just past `B.col`) because `nth(0)` reached the
// next char rather than emitting an empty slice.
#[test]
fn command_history_empty_command_with_long_prompt_is_dropped() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // ~30-char prompt — long enough that nth(B.col) is `Some`
    // (which is the precondition that surfaced the bug).
    session
        .feed(b"[user@host /Users/x/dev]$ \x1b]133;A\x07\x1b]133;B\x07")
        .unwrap();
    session.feed(b"\x1b]133;C\x07\x1b]133;D;0\x07").unwrap();
    let history = session.command_history();
    assert!(
        history.is_empty(),
        "empty command on a long-prompt row must drop, not hallucinate: {history:?}"
    );
}

// `slice_chars` is private; tested via the `super::*` re-export.
// These cases pin the boundary semantics — `end == start` returns
// empty (the fix), while `end > start` slices the requested chars.
#[test]
fn slice_chars_empty_range_returns_empty() {
    use super::slice_chars;
    // Row content longer than `start` — the previous bug would
    // return one stray character here.
    assert_eq!(slice_chars("hello world", 5, 5), "");
    assert_eq!(slice_chars("hello world", 0, 0), "");
    // `end < start` should also return empty (defensive — no caller
    // sends this today but the math is the same).
    assert_eq!(slice_chars("hello world", 7, 3), "");
}

#[test]
fn slice_chars_normal_ranges() {
    use super::slice_chars;
    assert_eq!(slice_chars("hello world", 0, 5), "hello");
    assert_eq!(slice_chars("hello world", 6, 11), "world");
    assert_eq!(slice_chars("hello world", 6, usize::MAX), "world");
}

#[test]
fn slice_chars_past_end_returns_empty() {
    use super::slice_chars;
    assert_eq!(slice_chars("hi", 5, 7), "");
    assert_eq!(slice_chars("", 0, 0), "");
}

#[test]
fn slice_chars_handles_multibyte() {
    use super::slice_chars;
    // Hangul syllables — 3 bytes each, one char per `nth` step.
    assert_eq!(slice_chars("가나다라", 1, 3), "나다");
    assert_eq!(slice_chars("가나다라", 0, 1), "가");
}
