use super::*;

// OSC 133 (FinalTerm / shell integration) ------------------------

#[test]
fn parses_osc133_prompt_start() {
    assert_eq!(
        parse_osc133_payload(b"A"),
        Some((PromptMarkKind::PromptStart, None))
    );
}

#[test]
fn parses_osc133_semantic_text_start_end() {
    assert_eq!(
        parse_osc133_payload(b"E"),
        Some((PromptMarkKind::SemanticTextStart, None))
    );
    assert_eq!(
        parse_osc133_payload(b"F"),
        Some((PromptMarkKind::SemanticTextEnd, None))
    );
}

#[test]
fn last_command_output_prefers_ef_pair() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // C..E..F..D cycle (iTerm2 output-only annotation).
    session.feed(b"\x1b]133;C\x07").unwrap();
    session.feed(b"\x1b]133;E\x07").unwrap();
    for _ in 0..3 {
        session.feed(b"output\r\n").unwrap();
    }
    session.feed(b"\x1b]133;F\x07").unwrap();
    session.feed(b"\x1b]133;D;0\x07").unwrap();
    let rng = session.last_command_output_rows().unwrap();
    assert!(rng.start < rng.end, "E must precede F; got {rng:?}");
}

#[test]
fn last_command_output_falls_back_to_cd_pair() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;C\x07").unwrap();
    for _ in 0..3 {
        session.feed(b"output\r\n").unwrap();
    }
    session.feed(b"\x1b]133;D;0\x07").unwrap();
    let rng = session.last_command_output_rows().unwrap();
    assert!(rng.start < rng.end);
}

#[test]
fn last_command_output_none_until_pair_closes() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;C\x07").unwrap(); // only start
    assert!(session.last_command_output_rows().is_none());
}

#[test]
fn parses_osc133_command_phases() {
    assert_eq!(
        parse_osc133_payload(b"B"),
        Some((PromptMarkKind::CommandStart, None))
    );
    assert_eq!(
        parse_osc133_payload(b"C"),
        Some((PromptMarkKind::CommandExecuted, None))
    );
    assert_eq!(
        parse_osc133_payload(b"D"),
        Some((PromptMarkKind::CommandFinished, None))
    );
}

#[test]
fn parses_osc133_command_finished_with_exit_code() {
    assert_eq!(
        parse_osc133_payload(b"D;0"),
        Some((PromptMarkKind::CommandFinished, Some(0)))
    );
    assert_eq!(
        parse_osc133_payload(b"D;127"),
        Some((PromptMarkKind::CommandFinished, Some(127)))
    );
}

#[test]
fn parses_osc133_accepts_payload_with_prefix() {
    // Some callers may not strip the `133;` prefix.
    assert_eq!(
        parse_osc133_payload(b"133;A"),
        Some((PromptMarkKind::PromptStart, None))
    );
}

#[test]
fn parses_osc133_rejects_unknown_subcommands() {
    assert!(parse_osc133_payload(b"Z").is_none());
    assert!(parse_osc133_payload(b"").is_none());
    assert!(parse_osc133_payload(b";C").is_none());
}

#[test]
fn feed_records_prompt_marks_via_bel_terminator() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;A\x07").unwrap();
    let marks = session.prompt_marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].kind, PromptMarkKind::PromptStart);
}

#[test]
fn feed_records_prompt_marks_via_st_terminator() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;D;1\x1b\\").unwrap();
    let marks = session.prompt_marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].kind, PromptMarkKind::CommandFinished);
    assert_eq!(marks[0].exit_code, Some(1));
}

#[test]
fn command_history_reports_duration_between_c_and_d_marks() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Need B (CommandStart) before C so command_history() records an entry.
    // Trailing space gives extract_command_text a non-empty slice between
    // the B and C cursor columns so the entry is not dropped as empty.
    session.feed(b"\x1b]133;B\x07x\x1b]133;C\x07").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    session.feed(b"\x1b]133;D;0\x07").unwrap();
    let history = session.command_history();
    let last = history.last().expect("entry recorded");
    assert!(
        last.duration
            .is_some_and(|d| d >= std::time::Duration::from_millis(5)),
        "expected duration ≥5ms, got {:?}",
        last.duration
    );
}

#[test]
fn feed_records_prompt_marks_across_chunk_boundaries() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Chunk 1: start of OSC, chunk 2: rest. The session buffers the
    // tail between calls so the mark must still land.
    session.feed(b"\x1b]133;A").unwrap();
    session.feed(b"\x07prompt$ ").unwrap();
    assert_eq!(session.prompt_marks().len(), 1);
}

#[test]
fn feed_does_not_duplicate_osc133_when_csi_and_osc_share_a_chunk() {
    // If a complete CSI mode sequence arrives in the same feed as
    // a complete OSC 133, the drain must not re-scan the OSC on
    // the next feed and push a duplicate mark.
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b[?2004h\x1b]133;A\x07more-output")
        .unwrap();
    assert_eq!(session.prompt_marks().len(), 1);
    // A second feed that merely appends unrelated text must not
    // re-emit the mark.
    session.feed(b"still more output\r\n").unwrap();
    assert_eq!(session.prompt_marks().len(), 1);
}

#[test]
fn feed_captures_sequence_of_prompt_events() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;0\x07")
        .unwrap();
    let kinds: Vec<_> = session.prompt_marks().iter().map(|m| m.kind).collect();
    assert_eq!(
        kinds,
        vec![
            PromptMarkKind::PromptStart,
            PromptMarkKind::CommandStart,
            PromptMarkKind::CommandExecuted,
            PromptMarkKind::CommandFinished,
        ]
    );
    assert_eq!(session.prompt_marks().back().unwrap().exit_code, Some(0));
}

#[test]
fn prompt_marks_are_bounded() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Emit PROMPT_MARKS_CAP + 5 events via many small feeds so the
    // OSC parse tail (2 KB cap) does not truncate the stream.
    for _ in 0..(PROMPT_MARKS_CAP + 5) {
        session.feed(b"\x1b]133;A\x07").unwrap();
    }
    assert_eq!(session.prompt_marks().len(), PROMPT_MARKS_CAP);
}

#[test]
fn prompt_mark_seq_is_strictly_monotonic() {
    // Every pushed mark gets a unique, strictly-increasing `seq` so it can
    // serve as a position-independent identity for jump-focus tracking.
    // Unlike a mark's screen row (re-flow / scroll move it) or the mark's
    // list position (`clear_line_buffer_and_drop_history_marks` drops wiped
    // entries), `seq` is never reused and never resets.
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    for _ in 0..5 {
        session.feed(b"\x1b]133;A\x07").unwrap();
    }
    let seqs: Vec<u64> = session.prompt_marks().iter().map(|m| m.seq).collect();
    assert_eq!(seqs.len(), 5);
    assert!(
        seqs.windows(2).all(|w| w[0] < w[1]),
        "expected strictly monotonic seqs, got {seqs:?}"
    );
}

#[test]
fn clear_scrollback_preserves_surviving_mark_seq() {
    // The `\x1b[3J` mirror in `clear_line_buffer_and_drop_history_marks`
    // leaves the viewport-resident mark's `abs_y` untouched — the
    // wipe is absorbed into `LineBuffer::overflow` (line-symmetric
    // with the logical-line `abs_y`). What it must never touch is
    // `seq`: that is the load-bearing identity invariant the focus
    // model relies on (see `view/state.rs::focused_prompt`). Without
    // it, `focused_prompt`'s stored seq would no longer match any
    // surviving mark and the jump highlight would silently drop
    // after a scrollback wipe.
    let mut s = session_with(80, 3, 1024);
    // History-resident mark — dropped by the wipe (`m.abs_y < history_top`).
    s.feed(b"x\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-1\r\n").unwrap();
    // Viewport-resident mark — must survive with `seq` unchanged. We
    // capture it here as the "focus candidate".
    s.feed(b"a\r\nb\r\nc\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-2").unwrap();
    let mark_before = *s.prompt_marks().back().unwrap();
    let history = s.line_buffer().wrapped_row_count(80) as u64;
    assert!(history > 0, "test setup must populate LineBuffer first");

    s.feed(b"\x1b[3J").unwrap();

    let marks = s.prompt_marks();
    assert_eq!(marks.len(), 1, "history mark dropped, viewport mark kept");
    let mark_after = marks[0];
    assert_eq!(
        mark_after.seq, mark_before.seq,
        "seq must survive the wipe — without this, identity-based focus breaks across \\x1b[3J",
    );
    // The post-wipe overflow has absorbed every line that the LineBuffer
    // held pre-wipe, so the surviving mark's `abs_y` (which was issued
    // for a viewport row above the wiped history) still sits above the
    // new overflow and remains reachable through `abs_to_screen_row`.
    assert!(
        mark_after.abs_y.as_u64() >= s.line_buffer().overflow(),
        "surviving viewport mark must remain past the new overflow",
    );
}

#[test]
fn prompt_mark_screen_row_follows_scrollback() {
    // PROMPT_START emitted after 50 lines of output must land on the
    // correct absolute screen row (viewport_row_offset + cursor_y)
    // once translated back from the mark's stored `abs_y`.
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    for i in 0..50 {
        session
            .feed(format!("line-{:02}\r\n", i).as_bytes())
            .unwrap();
    }
    let offset_before = session.viewport_row_offset();
    session.feed(b"\x1b]133;A\x07").unwrap();
    let mark = session.prompt_marks().back().copied().unwrap();
    assert_eq!(mark.kind, PromptMarkKind::PromptStart);
    let row = session
        .abs_to_screen_row(mark.abs_y)
        .expect("mark must still translate to a current row");
    assert!(
        row >= offset_before,
        "expected screen_row >= viewport offset ({offset_before}); got {row}"
    );
}

#[test]
fn feed_ignores_malformed_osc133() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;X\x07").unwrap();
    assert!(session.prompt_marks().is_empty());
}
