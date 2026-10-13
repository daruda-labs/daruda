use super::*;

// Alt-screen detection -------------------------------------------------

#[test]
fn alt_screen_entry_sets_screen_changed() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(!session.is_alt_screen());
    assert_eq!(session.take_screen_changed(), None);

    session.feed(b"\x1b[?1049h").unwrap();
    assert!(session.is_alt_screen());
    assert_eq!(session.take_screen_changed(), Some(true));
    assert_eq!(session.take_screen_changed(), None); // consumed
}

#[test]
fn alt_screen_exit_sets_screen_changed() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b[?1049h").unwrap();
    let _ = session.take_screen_changed();

    session.feed(b"\x1b[?1049l").unwrap();
    assert!(!session.is_alt_screen());
    assert_eq!(session.take_screen_changed(), Some(false));
}

#[test]
fn alt_screen_legacy_modes_detected() {
    let mut s = TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    s.feed(b"\x1b[?47h").unwrap();
    assert!(s.is_alt_screen());
    assert_eq!(s.take_screen_changed(), Some(true));

    s.feed(b"\x1b[?47l").unwrap();
    assert!(!s.is_alt_screen());
    assert_eq!(s.take_screen_changed(), Some(false));
}

// Scrollback buffer clearing on alt-screen transitions ----------------

#[test]
fn line_buffer_scrollback_survives_alt_screen_cycle() {
    // Under the new LineBuffer dispatcher, daruda's persistent
    // scrollback (`line_buffer`) is decoupled from ghostty's transient
    // ring. An alt-screen cycle wipes ghostty's primary buffer (we
    // still inject `\x1b[3J` for parity with the ring), but
    // `line_buffer` keeps the captured logical lines so the user can
    // scroll back to them after the TUI exits — matches iTerm2 /
    // Alacritty behaviour.
    let mut session = TerminalSession::new(
        TerminalDims { cols: 80, rows: 24 },
        TerminalConfig::default(),
    )
    .unwrap();
    for i in 0..50u32 {
        session.feed(format!("line-{i:02}\r\n").as_bytes()).unwrap();
    }
    let total_before = session.total_rows();
    assert!(
        total_before > 24,
        "scrollback should exist before alt-screen entry: total_rows={total_before}"
    );

    session.feed(b"\x1b[?1049h").unwrap();
    session.feed(b"\x1b[?1049l").unwrap();

    let total_after = session.total_rows();
    assert_eq!(
        total_after, total_before,
        "line_buffer scrollback must survive alt-screen cycle: \
         before={total_before} after={total_after}"
    );
}

#[test]
fn ghostty_vt_ed3_clears_scrollback() {
    // Direct verification that \x1b[3J works in ghostty_vt at all.
    let mut session = TerminalSession::new(
        TerminalDims { cols: 80, rows: 24 },
        TerminalConfig::default(),
    )
    .unwrap();
    for i in 0..30u32 {
        session.feed(format!("line-{i:02}\r\n").as_bytes()).unwrap();
    }
    assert!(session.total_rows() > 24, "scrollback should exist");

    session.feed(b"\x1b[3J").unwrap();
    assert_eq!(
        session.total_rows(),
        24,
        "\\x1b[3J should erase scrollback: total_rows={}",
        session.total_rows()
    );
}

#[test]
fn dump_viewport_is_blank_after_alt_screen_entry() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Write content on the primary screen
    session.feed(b"hello world").unwrap();
    let primary = session.dump_viewport().unwrap();
    assert!(
        primary.contains("hello"),
        "primary should have content: {primary:?}"
    );

    // Enter alt-screen (mode 1049 clears the alt-screen on entry)
    session.feed(b"\x1b[?1049h").unwrap();
    let alt = session.dump_viewport().unwrap();
    assert!(
        !alt.contains("hello"),
        "alt-screen should not show primary content: {alt:?}"
    );
}

#[test]
fn dump_viewport_returns_to_primary_after_alt_screen_exit() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"hello world").unwrap();
    session.feed(b"\x1b[?1049h").unwrap();
    session.feed(b"tui content").unwrap();

    // Exit alt-screen (mode 1049 restores cursor on exit)
    session.feed(b"\x1b[?1049l").unwrap();
    let restored = session.dump_viewport().unwrap();
    // Primary screen content should be back
    assert!(
        restored.contains("hello"),
        "primary content should be restored: {restored:?}"
    );
    assert!(
        !restored.contains("tui content"),
        "alt-screen content should not appear: {restored:?}"
    );
}

// DECCKM / DECNKM / SynchronizedOutput mode tracking ------------------

#[test]
fn decckm_tracks_set_and_reset() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(!session.decckm_enabled(), "DECCKM off by default");

    session.feed(b"\x1b[?1h").unwrap();
    assert!(session.decckm_enabled(), "DECCKM should be set");

    session.feed(b"\x1b[?1l").unwrap();
    assert!(!session.decckm_enabled(), "DECCKM should be reset");
}

#[test]
fn decckm_decrqm_reflects_current_state() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Default: reset (2)
    let resp = collect_responses(&mut session, b"\x1b[?1$p");
    assert_eq!(resp, b"\x1b[?1;2$y", "DECCKM reset by default");

    collect_responses(&mut session, b"\x1b[?1h");
    let resp = collect_responses(&mut session, b"\x1b[?1$p");
    assert_eq!(resp, b"\x1b[?1;1$y", "DECCKM set");
}

#[test]
fn decnkm_tracks_set_and_reset() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(!session.decnkm_enabled(), "DECNKM off by default");

    session.feed(b"\x1b[?66h").unwrap();
    assert!(session.decnkm_enabled(), "DECNKM should be set");

    session.feed(b"\x1b[?66l").unwrap();
    assert!(!session.decnkm_enabled(), "DECNKM should be reset");
}

#[test]
fn synchronized_output_tracks_set_and_reset() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(
        !session.synchronized_output_enabled(),
        "sync output off by default"
    );

    session.feed(b"\x1b[?2026h").unwrap();
    assert!(
        session.synchronized_output_enabled(),
        "sync output should be set"
    );

    let resp = collect_responses(&mut session, b"\x1b[?2026$p");
    assert_eq!(resp, b"\x1b[?2026;1$y", "DECRQM should reflect set state");

    session.feed(b"\x1b[?2026l").unwrap();
    assert!(
        !session.synchronized_output_enabled(),
        "sync output should be reset"
    );

    let resp = collect_responses(&mut session, b"\x1b[?2026$p");
    assert_eq!(resp, b"\x1b[?2026;2$y", "DECRQM should reflect reset state");
}

// X10 press-only mode ---------------------------------------------------

#[test]
fn mouse_x10_only_true_when_only_x10_active() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    assert!(!session.mouse_x10_only());

    session.feed(b"\x1b[?1000h").unwrap();
    assert!(session.mouse_x10_only(), "X10 alone → press-only");

    // Adding button-event mode (1002) must clear the press-only flag.
    session.feed(b"\x1b[?1002h").unwrap();
    assert!(
        !session.mouse_x10_only(),
        "X10 + ButtonEvent → not press-only"
    );

    // Removing button-event mode restores press-only.
    session.feed(b"\x1b[?1002l").unwrap();
    assert!(session.mouse_x10_only(), "back to X10 alone → press-only");
}

#[test]
fn mouse_x10_only_false_when_any_event_active() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b[?1000h").unwrap();
    session.feed(b"\x1b[?1003h").unwrap();
    assert!(!session.mouse_x10_only(), "X10 + AnyEvent → not press-only");
}
