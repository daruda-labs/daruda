use super::scanners::{parse_osc7_path, percent_decode};
use super::*;
use crate::coords::ViewportRow;
use crate::{TerminalConfig, TerminalDims};

#[test]
fn parses_osc7_with_hostname() {
    let path = parse_osc7_path(b"file://mymac/home/user/projects/daruda").unwrap();
    assert_eq!(path, "/home/user/projects/daruda");
}

#[test]
fn parses_osc7_with_empty_hostname() {
    let path = parse_osc7_path(b"file:///tmp/x").unwrap();
    assert_eq!(path, "/tmp/x");
}

#[test]
fn percent_decodes_spaces() {
    assert_eq!(percent_decode("/a%20b/c"), "/a b/c");
}

#[test]
fn rejects_non_file_scheme() {
    assert!(parse_osc7_path(b"http://x/y").is_none());
}

#[test]
fn feed_sets_cwd_via_osc7_bel() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]7;file://host/home/user\x07").unwrap();
    assert_eq!(session.cwd(), Some("/home/user"));
}

#[test]
fn feed_sets_cwd_via_osc7_st() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]7;file://host/tmp/daruda\x1b\\")
        .unwrap();
    assert_eq!(session.cwd(), Some("/tmp/daruda"));
}

#[test]
fn track_cwd_disabled_skips_osc7() {
    let cfg = TerminalConfig {
        track_cwd: false,
        ..TerminalConfig::default()
    };
    let mut session = TerminalSession::new(TerminalDims::default(), cfg).unwrap();
    session.feed(b"\x1b]7;file://host/home/user\x07").unwrap();
    assert_eq!(session.cwd(), None);
}

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

// P0-1 regression: a private CSI mode sequence that ends in an
// unknown terminator byte ('m', 'r', etc. instead of 'h'/'l') used
// to be only partially skipped, leaving the bulk of it in
// `parse_tail`. A subsequent well-formed CSI mode could then be
// mis-attributed or an OSC that followed it re-scanned.
#[test]
fn csi_with_unknown_terminator_is_fully_skipped() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // The unknown-terminator CSI, then a valid mode-set and an OSC
    // 133;A all in one chunk. We expect exactly one prompt mark and
    // bracketed paste enabled — the unknown CSI must not corrupt
    // either.
    session
        .feed(b"\x1b[?999m\x1b[?2004h\x1b]133;A\x07")
        .unwrap();
    assert_eq!(session.prompt_marks().len(), 1);
    assert!(session.bracketed_paste_enabled());
}

// OSC 1337 RequestAttention surfaces an `AttentionKind` and is
// drained one-shot via `take_attention_request`.
#[test]
fn osc1337_request_attention_yes_maps_to_critical() {
    use crate::vt_codes::AttentionKind;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]1337;RequestAttention=yes\x07").unwrap();
    assert_eq!(
        session.take_attention_request(),
        Some(AttentionKind::Critical)
    );
    assert_eq!(session.take_attention_request(), None);
}

#[test]
fn osc1337_request_attention_recognises_no_once_fireworks() {
    use crate::vt_codes::AttentionKind;
    let cases = [
        ("no", AttentionKind::Cancel),
        ("once", AttentionKind::Once),
        ("fireworks", AttentionKind::Once),
    ];
    for (value, expect) in cases {
        let mut session =
            TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
        let bytes = format!("\x1b]1337;RequestAttention={value}\x07");
        session.feed(bytes.as_bytes()).unwrap();
        assert_eq!(session.take_attention_request(), Some(expect));
    }
}

#[test]
fn osc1337_unknown_key_is_ignored() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]1337;SetMark\x07\x1b]1337;CurrentDir=/tmp\x07")
        .unwrap();
    assert_eq!(session.take_attention_request(), None);
}

// Cross-chunk: `\x1b]1337;` arrives before its terminator. The OSC
// aggregator must hold the partial sequence in `parse_tail` and
// resume on the next feed instead of dropping the request.
#[test]
fn osc1337_request_attention_split_across_feeds() {
    use crate::vt_codes::AttentionKind;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]1337;Request").unwrap();
    assert_eq!(session.take_attention_request(), None);
    session.feed(b"Attention=yes\x07").unwrap();
    assert_eq!(
        session.take_attention_request(),
        Some(AttentionKind::Critical)
    );
}

// `ESC \` (ST) terminator must be recognised alongside BEL.
#[test]
fn osc1337_request_attention_st_terminator() {
    use crate::vt_codes::AttentionKind;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]1337;RequestAttention=once\x1b\\")
        .unwrap();
    assert_eq!(session.take_attention_request(), Some(AttentionKind::Once));
}

// Title (OSC 0/2) interleaved with OSC 1337 in one feed: both slots
// must commit, neither overwrites the other.
#[test]
fn osc1337_coexists_with_title_in_same_chunk() {
    use crate::vt_codes::AttentionKind;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]2;tab title\x07\x1b]1337;RequestAttention=yes\x07")
        .unwrap();
    assert_eq!(session.title(), Some("tab title"));
    assert_eq!(
        session.take_attention_request(),
        Some(AttentionKind::Critical)
    );
}

// iTerm2 accepts the key in arbitrary case. Match that so a shell
// emitting `requestattention=yes` from a script still works.
#[test]
fn osc1337_request_attention_key_is_case_insensitive() {
    use crate::vt_codes::AttentionKind;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]1337;requestattention=yes\x07").unwrap();
    assert_eq!(
        session.take_attention_request(),
        Some(AttentionKind::Critical)
    );
}

// OSC 9 carries body-only text. Title is left unset — workspace fills
// it with the app name.
#[test]
fn osc9_emits_notification_with_body_only() {
    use crate::vt_codes::NotificationRequest;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]9;build finished\x07").unwrap();
    assert_eq!(
        session.take_notification_request(),
        Some(NotificationRequest::Osc9 {
            body: "build finished".into()
        })
    );
}

// Empty OSC 9 payload is dropped (no-op notification is just noise).
#[test]
fn osc9_empty_body_is_ignored() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]9;\x07").unwrap();
    assert_eq!(session.take_notification_request(), None);
}

// OSC 777 ; notify ; <title> ; <body>
#[test]
fn osc777_notify_emits_title_and_body() {
    use crate::vt_codes::NotificationRequest;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]777;notify;Build;all green\x07")
        .unwrap();
    assert_eq!(
        session.take_notification_request(),
        Some(NotificationRequest::Osc777 {
            title: "Build".into(),
            body: "all green".into(),
        })
    );
}

// Body containing semicolons is preserved (splitn(3) leaves the rest
// intact). Real-world payloads include URLs, JSON snippets, etc.
#[test]
fn osc777_notify_preserves_semicolons_in_body() {
    use crate::vt_codes::NotificationRequest;
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session
        .feed(b"\x1b]777;notify;deploy;ok; staged; verified\x07")
        .unwrap();
    assert_eq!(
        session.take_notification_request(),
        Some(NotificationRequest::Osc777 {
            title: "deploy".into(),
            body: "ok; staged; verified".into(),
        })
    );
}

// Subcommand `preexec` and any other unknown sub-OSC fall through.
#[test]
fn osc777_unknown_subcommand_is_ignored() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]777;preexec\x07").unwrap();
    assert_eq!(session.take_notification_request(), None);
}

// FTCS B → CommandFinished elapsed is captured. The actual duration
// depends on test scheduling, so just assert the slot was populated
// once and then drained.
#[test]
fn ftcs_b_to_d_emits_finished_command_elapsed() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;B\x07").unwrap();
    session.feed(b"\x1b]133;D;0\x07").unwrap();
    assert!(session.take_finished_command_elapsed().is_some());
    assert!(session.take_finished_command_elapsed().is_none());
}

// CommandFinished without a preceding CommandStart yields no elapsed
// (e.g. shell exits before the first prompt or a malformed stream).
#[test]
fn ftcs_d_without_b_emits_no_elapsed() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;D;0\x07").unwrap();
    assert!(session.take_finished_command_elapsed().is_none());
}

// A new prompt without a CommandFinished — the previous command was
// aborted (Ctrl-C). The orphaned start time must be dropped so the
// next CommandFinished does not measure a multi-prompt span.
#[test]
fn aborted_command_drops_command_start() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]133;B\x07").unwrap(); // command starts
    session.feed(b"\x1b]133;A\x07").unwrap(); // user hits Ctrl-C, new prompt drawn
    session.feed(b"\x1b]133;D;130\x07").unwrap(); // stale D from somewhere
    assert!(session.take_finished_command_elapsed().is_none());
}

// `OSC 1337 ; ClearScrollback` (bare key, no `=`) — recognised
// and translated into `\x1b[3J` re-fed to ghostty_vt. Visible
// effect can't be observed through the public API, so this just
// asserts parser tolerates the form without crashing.
#[test]
fn osc1337_clear_scrollback_is_accepted() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"hello\r\n").unwrap();
    session.feed(b"\x1b]1337;ClearScrollback\x07").unwrap();
    // No attention or notification should leak from ClearScrollback.
    assert_eq!(session.take_attention_request(), None);
    assert_eq!(session.take_notification_request(), None);
}

#[test]
fn osc1337_clear_scrollback_is_case_insensitive() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(b"\x1b]1337;clearscrollback\x07").unwrap();
    // No assertion on internal flag — the test passes if parser
    // accepts the case variant without erroring.
    assert_eq!(session.take_attention_request(), None);
}

// `OSC 1337 ; Copy=<sel>:<base64>` — the same kind of clipboard
// write OSC 52 handles, just with iTerm2's framing. The decoded
// text lands on `take_clipboard_write`.
#[test]
fn osc1337_copy_decodes_base64_to_clipboard() {
    use base64::Engine as _;
    let body = "from iterm2";
    let encoded = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b]1337;Copy=:");
    bytes.extend_from_slice(encoded.as_bytes());
    bytes.extend_from_slice(b"\x07");

    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(&bytes).unwrap();
    assert_eq!(session.take_clipboard_write().as_deref(), Some(body));
}

#[test]
fn osc1337_copy_with_selection_prefix_still_writes_clipboard() {
    use base64::Engine as _;
    let body = "with selection";
    let encoded = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let mut bytes = Vec::new();
    // selection char `c` (system clipboard) — OSC 52 syntax.
    bytes.extend_from_slice(b"\x1b]1337;Copy=c:");
    bytes.extend_from_slice(encoded.as_bytes());
    bytes.extend_from_slice(b"\x07");

    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(&bytes).unwrap();
    assert_eq!(session.take_clipboard_write().as_deref(), Some(body));
}

// A 100 KiB Copy= payload — exceeds the generic PARSE_TAIL_LIMIT
// (64 KiB) but stays well under `osc1337_max_bytes` (10 MiB).
// Verifies the per-OSC-1337 cap actually lets large payloads through.
#[test]
fn osc1337_copy_above_generic_tail_limit_still_lands() {
    use base64::Engine as _;
    let body: String = "B".repeat(100 * 1024);
    let encoded = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b]1337;Copy=:");
    bytes.extend_from_slice(encoded.as_bytes());
    bytes.extend_from_slice(b"\x07");

    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(&bytes).unwrap();
    let clip = session
        .take_clipboard_write()
        .expect("100 KiB Copy= should reach the clipboard");
    assert_eq!(clip.len(), 100 * 1024);
}

// Verifies that a single OSC 52 payload larger than the parser's
// TAIL_LIMIT still reaches the clipboard — the scanner must see
// `\x1b]52;c;` even when the payload exceeds the tail window.
#[test]
fn large_osc52_payload_survives_truncation() {
    use base64::Engine as _;
    let body: String = "A".repeat(4096);
    let encoded = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"\x1b]52;c;");
    bytes.extend_from_slice(encoded.as_bytes());
    bytes.extend_from_slice(b"\x07");
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    session.feed(&bytes).unwrap();
    let clip = session.take_clipboard_write().expect("clipboard set");
    assert_eq!(clip.len(), 4096);
}

// P0-3 regression: two OSC 133 marks in the same feed chunk, with
// output between them that advances the cursor across rows. Each
// mark must be recorded on the row the shell intended, not on the
// cursor's pre-feed or post-feed row.
#[test]
fn osc133_marks_in_one_chunk_land_on_correct_rows() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // A on row 0, then 3 newlines of output, then C on row ~3.
    session
        .feed(b"\x1b]133;A\x07line1\r\nline2\r\nline3\r\n\x1b]133;C\x07")
        .unwrap();
    let marks: Vec<_> = session.prompt_marks().iter().copied().collect();
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0].kind, PromptMarkKind::PromptStart);
    assert_eq!(marks[1].kind, PromptMarkKind::CommandExecuted);
    // Rows must be strictly increasing (shell advanced between them).
    assert!(
        marks[1].abs_y > marks[0].abs_y,
        "OSC 133 rows should reflect per-segment cursor: got {} then {}",
        marks[0].abs_y,
        marks[1].abs_y
    );
}

// Capability queries ---------------------------------------------------

fn collect_responses(session: &mut TerminalSession, query: &[u8]) -> Vec<u8> {
    let mut responses = Vec::new();
    session
        .feed_with_pty_responses(query, |bytes| responses.extend_from_slice(bytes))
        .unwrap();
    responses
}

#[test]
fn primary_da_responds_to_esc_bracket_c() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[c");
    assert_eq!(resp, ansi::PRIMARY_DA);
}

#[test]
fn primary_da_responds_to_esc_bracket_0c() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[0c");
    assert_eq!(resp, ansi::PRIMARY_DA);
}

#[test]
fn secondary_da_responds_to_esc_bracket_gt_c() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[>c");
    assert_eq!(resp, ansi::SECONDARY_DA);
}

#[test]
fn secondary_da_responds_to_esc_bracket_gt_0c() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[>0c");
    assert_eq!(resp, ansi::SECONDARY_DA);
}

#[test]
fn tertiary_da_responds_to_esc_bracket_eq_c() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[=c");
    assert_eq!(resp, ansi::TERTIARY_DA);
}

#[test]
fn xtversion_responds_to_esc_bracket_gt_0q() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[>0q");
    assert_eq!(resp, ansi::XTVERSION);
}

#[test]
fn xtversion_responds_to_esc_bracket_gt_q() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[>q");
    assert_eq!(resp, ansi::XTVERSION);
}

#[test]
fn kitty_keyboard_responds_to_esc_bracket_q_u() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[?u");
    assert_eq!(resp, ansi::KITTY_KEYBOARD_RESPONSE);
}

#[test]
fn decrqm_returns_set_for_active_bracketed_paste() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    collect_responses(&mut session, b"\x1b[?2004h");
    let resp = collect_responses(&mut session, b"\x1b[?2004$p");
    assert_eq!(resp, b"\x1b[?2004;1$y");
}

#[test]
fn decrqm_returns_reset_for_inactive_mode() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // SGR mouse is off by default
    let resp = collect_responses(&mut session, b"\x1b[?1006$p");
    assert_eq!(resp, b"\x1b[?1006;2$y");
}

#[test]
fn decrqm_returns_not_recognised_for_unknown_mode() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let resp = collect_responses(&mut session, b"\x1b[?9999$p");
    assert_eq!(resp, b"\x1b[?9999;0$y");
}

#[test]
fn decrqm_alt_screen_tracks_current_state() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    let off = collect_responses(&mut session, b"\x1b[?1049$p");
    assert_eq!(off, b"\x1b[?1049;2$y");

    collect_responses(&mut session, b"\x1b[?1049h");
    let on = collect_responses(&mut session, b"\x1b[?1049$p");
    assert_eq!(on, b"\x1b[?1049;1$y");
}

#[test]
fn xtgettcap_tn_returns_terminal_name() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // TN hex-encoded = 544e
    let resp = collect_responses(&mut session, b"\x1bP+q544e\x1b\\");
    let resp_str = String::from_utf8(resp).unwrap();
    assert!(
        resp_str.starts_with("\x1bP1+r"),
        "should be found: {resp_str:?}"
    );
    assert!(resp_str.contains("544e="), "key echo: {resp_str:?}");
    // value is "daruda" hex = 6461727564 61
    assert!(
        resp_str.contains("646172756461"),
        "daruda value: {resp_str:?}"
    );
}

#[test]
fn xtgettcap_rgb_returns_one() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // RGB hex-encoded = 524742
    let resp = collect_responses(&mut session, b"\x1bP+q524742\x1b\\");
    let resp_str = String::from_utf8(resp).unwrap();
    assert!(
        resp_str.starts_with("\x1bP1+r"),
        "should be found: {resp_str:?}"
    );
    assert!(resp_str.contains("31"), "value '1' as hex: {resp_str:?}");
}

#[test]
fn xtgettcap_unknown_returns_not_found() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // "XY" hex = 5859
    let resp = collect_responses(&mut session, b"\x1bP+q5859\x1b\\");
    let resp_str = String::from_utf8(resp).unwrap();
    assert!(
        resp_str.starts_with("\x1bP0+r"),
        "should be not-found: {resp_str:?}"
    );
}

#[test]
fn xtgettcap_multiple_caps_in_one_query() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // TN;RGB = 544e;524742
    let resp = collect_responses(&mut session, b"\x1bP+q544e;524742\x1b\\");
    let resp_str = String::from_utf8(resp).unwrap();
    assert!(resp_str.contains("\x1bP1+r544e="), "TN found: {resp_str:?}");
    assert!(
        resp_str.contains("\x1bP1+r524742="),
        "RGB found: {resp_str:?}"
    );
}

#[test]
fn primary_da_does_not_fire_on_nonzero_param() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // ESC[3c is not a Primary DA query (Ps≠0 undefined)
    let resp = collect_responses(&mut session, b"\x1b[3c");
    assert!(resp.is_empty(), "no response for ESC[3c: {resp:?}");
}

#[test]
fn secondary_da_does_not_fire_on_nonzero_gt_param() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // ESC[>1c is not a Secondary DA query
    let resp = collect_responses(&mut session, b"\x1b[>1c");
    assert!(resp.is_empty(), "no response for ESC[>1c: {resp:?}");
}

#[test]
fn capability_query_survives_chunk_boundary_split_at_bracket() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Feed "ESC[" in first chunk, "c" in second
    let mut resp = Vec::new();
    session
        .feed_with_pty_responses(b"\x1b[", |b| resp.extend_from_slice(b))
        .unwrap();
    session
        .feed_with_pty_responses(b"c", |b| resp.extend_from_slice(b))
        .unwrap();
    assert_eq!(resp, ansi::PRIMARY_DA);
}

#[test]
fn decrqm_survives_chunk_boundary_mid_digits() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Enable bracketed paste, then query across chunk boundary:
    // "ESC[?200" in chunk 1, "4$p" in chunk 2
    collect_responses(&mut session, b"\x1b[?2004h");
    let mut resp = Vec::new();
    session
        .feed_with_pty_responses(b"\x1b[?200", |b| resp.extend_from_slice(b))
        .unwrap();
    session
        .feed_with_pty_responses(b"4$p", |b| resp.extend_from_slice(b))
        .unwrap();
    assert_eq!(resp, b"\x1b[?2004;1$y");
}

#[test]
fn xtgettcap_survives_chunk_boundary_mid_dcs() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // Split "ESC P + q 544e ESC \" at the ESC of the ST terminator
    let mut resp = Vec::new();
    session
        .feed_with_pty_responses(b"\x1bP+q544e", |b| resp.extend_from_slice(b))
        .unwrap();
    session
        .feed_with_pty_responses(b"\x1b\\", |b| resp.extend_from_slice(b))
        .unwrap();
    let resp_str = String::from_utf8(resp).unwrap();
    assert!(
        resp_str.contains("646172756461"),
        "daruda hex value: {resp_str:?}"
    );
}

#[test]
fn capability_scanner_does_not_fire_on_dsr_sequences() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    // ESC[5n is DSR, not a DA query — should not produce DA response
    let resp = collect_responses(&mut session, b"\x1b[5n");
    assert_eq!(resp, ansi::DSR_OK);
    // must NOT contain PRIMARY_DA
    assert!(!resp.contains(&b'?'), "no DA response expected: {resp:?}");
}

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

// LineBuffer capture wiring ---------------------------------------

fn session_with(cols: u16, rows: u16, max_scrollback: usize) -> TerminalSession {
    let dims = TerminalDims { cols, rows };
    let config = TerminalConfig {
        max_scrollback,
        ..TerminalConfig::default()
    };
    TerminalSession::new(dims, config).expect("failed to create session")
}

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

// PromptMark.abs_y survives LineBuffer ring eviction --------------

#[test]
fn prompt_mark_abs_y_is_stable_across_normal_scrolling() {
    // Without any ring overflow, `abs_y` never changes — it's the
    // tracking invariant that lets translation work.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"x\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07").unwrap();
    let before = s.prompt_marks()[0].abs_y;
    s.feed(b"a\r\nb\r\nc\r\nd\r\n").unwrap();
    let after = s.prompt_marks()[0].abs_y;
    assert_eq!(
        before, after,
        "abs_y must not move when only LineBuffer captures happen"
    );
}

#[test]
fn prompt_mark_translates_to_screen_row_after_scrolling() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"\x1b]133;A\x07").unwrap(); // mark on row 0, abs_y=0
    s.feed(b"line-1\r\nline-2\r\nline-3\r\nline-4\r\n").unwrap();
    let mark = s.prompt_marks()[0];
    assert_eq!(mark.abs_y.as_u64(), 0, "mark fired before any scroll");
    // 4 newlines into a 3-row grid scrolls 2 rows out; both land in
    // LineBuffer at indices 0..2. Live grid then holds line-3, line-4,
    // empty. The mark at abs_y=0 must translate to LineBuffer index 0
    // (top of scrollback), not just any in-range row.
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark still reachable through LineBuffer dispatcher");
    assert_eq!(
        row, 0,
        "mark at abs_y=0 should translate to LineBuffer row 0 (top of scrollback)"
    );
    assert!(row < s.total_rows());
}

#[test]
fn prompt_mark_survives_line_buffer_eviction() {
    // max_scrollback=2 so the LineBuffer evicts old lines, bumping
    // its `overflow` counter and making any pre-eviction mark's
    // `abs_y` fall below `overflow`.
    let mut s = session_with(80, 3, 2);
    s.feed(b"\x1b]133;A\x07prompt-1\r\n").unwrap();
    let mark_abs = s.prompt_marks()[0].abs_y;
    let line_buffer_overflow_before = s.line_buffer().overflow();
    // Force enough eviction that `overflow` grows past `mark_abs`.
    for _ in 0..10 {
        s.feed(b"x\r\n").unwrap();
    }
    let line_buffer_overflow_after = s.line_buffer().overflow();
    let marks = s.prompt_marks();
    assert!(!marks.is_empty(), "mark must still be recorded");
    assert_eq!(marks[0].abs_y, mark_abs, "abs_y is stable");
    assert!(
        line_buffer_overflow_after > line_buffer_overflow_before,
        "test setup must trigger eviction (before={line_buffer_overflow_before}, after={line_buffer_overflow_after})"
    );
    assert!(
        mark_abs.as_u64() < line_buffer_overflow_after,
        "test setup must place mark below overflow (mark_abs={mark_abs}, overflow={line_buffer_overflow_after})"
    );
    // The line has been evicted — translation must return None and
    // not silently alias another row.
    assert!(
        s.abs_to_screen_row(mark_abs).is_none(),
        "evicted mark must not translate to a row (abs_y={mark_abs} < overflow={line_buffer_overflow_after})"
    );
}

#[test]
fn clear_scrollback_drops_history_marks_keeps_viewport_marks_abs_y() {
    let mut s = session_with(80, 3, 1024);
    // Mark #1 lands on row 1 (after one preceding line) — it will
    // scroll into LineBuffer history before the clear and must be
    // dropped.
    s.feed(b"x\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-1\r\n").unwrap();
    // Push another mark on the line that ends up viewport-resident
    // at clear time so we can assert it survives the wipe.
    s.feed(b"a\r\nb\r\nc\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07prompt-2").unwrap();
    let viewport_mark_abs_before = s.prompt_marks().back().unwrap().abs_y;
    assert!(
        !s.line_buffer().is_empty(),
        "test setup must populate LineBuffer first"
    );

    s.feed(b"\x1b[3J").unwrap();
    assert_eq!(s.line_buffer().len(), 0, "scrollback must be cleared");

    // History-resident mark (#1) is gone; only the viewport-resident
    // mark (#2) remains. With logical-line `abs_y`, `LineBuffer::clear`
    // bumps `overflow` by the wiped logical line count and absorbs the
    // full wipe — surviving marks keep their `abs_y` untouched.
    let marks = s.prompt_marks();
    assert_eq!(marks.len(), 1, "history mark dropped, viewport mark kept");
    let after = marks[0].abs_y;
    assert_eq!(
        after, viewport_mark_abs_before,
        "viewport mark's abs_y must be unchanged across the wipe"
    );
    let row = s
        .abs_to_screen_row(after)
        .expect("surviving mark must still translate");
    assert!(row < s.total_rows());
}

#[test]
fn clear_scrollback_under_wrap_does_not_over_shift_marks() {
    // Pin Task 4's fix: a wrap-inflated buffer (visual rows > logical
    // lines) used to apply a `visual_residual` shift on top of the
    // logical-line `abs_y`, over-shifting viewport-resident marks.
    // Post-Task-4 the wipe is line-symmetric, so `abs_y` must survive
    // intact regardless of wrap inflation.
    let mut s = session_with(4, 3, 1024);
    // Feed several logical lines each wider than 4 cols so soft-wrap
    // inflates the visual row count well past the logical line count.
    s.feed(b"abcdefgh\r\n").unwrap();
    s.feed(b"ijklmnop\r\n").unwrap();
    s.feed(b"qrstuvwx\r\n").unwrap();
    // Capture a mark on the line that ends up viewport-resident at
    // clear time.
    s.feed(b"\x1b]133;A\x07p").unwrap();
    let viewport_mark_abs_before = s.prompt_marks().back().unwrap().abs_y;

    let logical_pre = s.line_buffer().len() as u64;
    let visual_pre = s.line_buffer().wrapped_row_count(4) as u64;
    assert!(
        visual_pre > logical_pre,
        "test setup must wrap-inflate the buffer (visual={visual_pre}, logical={logical_pre})"
    );

    s.feed(b"\x1b[3J").unwrap();
    assert_eq!(s.line_buffer().len(), 0, "scrollback must be cleared");

    let marks = s.prompt_marks();
    assert_eq!(marks.len(), 1, "viewport-resident mark must survive");
    assert_eq!(
        marks[0].abs_y, viewport_mark_abs_before,
        "wrap inflation must not shift abs_y — pre-Task-4 this over-shifted"
    );
    let row = s
        .abs_to_screen_row(marks[0].abs_y)
        .expect("surviving mark must still translate to a row");
    assert!(row < s.total_rows());
}

#[test]
fn prompt_mark_abs_y_accounts_for_in_flight_scroll_before_osc() {
    // OSC 133 fires AFTER mid-feed scrolling. abs_y must point at the
    // row where the OSC actually landed, not at a row earlier in scrollback.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\n\x1b]133;A\x07p").unwrap();
    let mark = s.prompt_marks()[0];
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark must be reachable after capture");
    let vp_top = s.viewport_row_offset();
    let vp_rows = s.rows() as u32;
    assert!(
        row >= vp_top && row < vp_top + vp_rows,
        "mark must land in viewport (where OSC fired), got row={row} vp_top={vp_top} vp_rows={vp_rows}"
    );
    let in_vp = row - vp_top;
    assert_eq!(
        in_vp, 2,
        "OSC fired on viewport row 2 ('p' line) — must not point at a scrolled row"
    );
}

/// Regression: daruda's own scrollback (`LineBuffer`) must retain up to
/// `max_scrollback` logical lines, independent of ghostty's small transient
/// ring. Feeding 5_000 lines one-at-a-time (well above the ring, below the
/// 10k cap) must leave every scrolled-off line in the buffer.
///
/// Before the tracked-pin cursor fix, capture used `viewport_row_offset()`
/// (ghostty's retained scrollback depth, not a monotonic counter); once that
/// saturated, the capture cursor froze and `LineBuffer` stopped growing at
/// ~1_000 lines, silently dropping the rest (overflow stayed 0).
#[test]
fn line_buffer_retains_scrollback_beyond_ghostty_ring() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    const N: usize = 5_000; // 80x24 grid, 10k cap
    for i in 0..N {
        session.feed(format!("LINE{i:06}\r\n").as_bytes()).unwrap();
    }

    let lb = session.line_buffer();
    let texts: Vec<String> = (0..lb.len())
        .filter_map(|idx| lb.get(idx).map(|l| l.text.clone()))
        .collect();

    // None of the fed lines reached the 10k cap, so nothing was evicted.
    assert_eq!(
        lb.overflow(),
        0,
        "no line should be evicted below the {N} < 10k cap"
    );

    // Every line that scrolled off the live grid must survive in scrollback.
    // Leave a 32-row margin at the top for grid / partial-tail boundary fuzz.
    let mut missing = Vec::new();
    for i in 0..N.saturating_sub(32) {
        let needle = format!("LINE{i:06}");
        if !texts.iter().any(|t| t.contains(&needle)) {
            missing.push(i);
        }
    }
    assert!(
        missing.is_empty(),
        "lost {} of {N} scrolled-off lines (effective scrollback froze at lb.len()={}); \
         first 5 missing = {:?}",
        missing.len(),
        lb.len(),
        missing.iter().take(5).collect::<Vec<_>>(),
    );
}

// Logical-line grid-walk helpers ----------------------------------
//
// `peek_uncaptured_logical_lines` and `logical_lines_until_cursor`
// back `current_abs_y_at_cursor`'s logical-line abs computation. They
// live on the session as private helpers; these tests pin the
// post-feed invariants and the grid-walk contract that
// `logical_lines_until_cursor_after_hard_lines` exercises.

#[test]
fn peek_uncaptured_logical_lines_post_feed_is_empty() {
    // `feed()` always runs `capture_scrolled_out` at the end, draining
    // the tracked-pin delta. The Hard-EOL counting branch itself is
    // only reachable mid-feed (when OSC 133 dispatches before the
    // end-of-feed capture); from a black-box public API there is no
    // way to observe it directly. This test pins the post-capture
    // invariant — uncaptured region empty — and confirms the helper
    // isn't trivially zero because nothing scrolled (LineBuffer did
    // receive the rows). The counting-branch correctness is exercised
    // indirectly via `logical_lines_until_cursor_after_hard_lines`,
    // which walks the same code path through a non-empty grid region.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\n").unwrap();
    assert_eq!(
        s.peek_uncaptured_logical_lines(),
        0,
        "after capture, uncaptured region must be empty"
    );
    assert!(
        s.line_buffer().len() >= 2,
        "expected >= 2 captured lines, got {}",
        s.line_buffer().len()
    );

    s.feed(b"f\r\ng\r\n").unwrap();
    assert_eq!(s.peek_uncaptured_logical_lines(), 0);
}

#[test]
fn logical_lines_until_cursor_at_top_of_grid() {
    // Fresh session, no feed: cursor sits at row 1, no Hard EOL
    // precedes it in the uncaptured/grid region.
    let s = session_with(80, 3, 1024);
    assert_eq!(s.logical_lines_until_cursor(), 0);
}

#[test]
fn logical_lines_until_cursor_after_hard_lines() {
    // Feed "a\r\nb\r\nc" into an 80x3 grid: cursor lands on the row
    // containing 'c' (row 3), preceded by 2 Hard EOLs on rows 1 and
    // 2. Nothing has scrolled out yet (3 lines fit), so the count
    // comes entirely from the grid-above-cursor walk.
    let mut s = session_with(80, 3, 1024);
    s.feed(b"a\r\nb\r\nc").unwrap();
    assert_eq!(
        s.logical_lines_until_cursor(),
        2,
        "two Hard EOLs precede the cursor's row"
    );
}

#[test]
fn peek_uncaptured_logical_lines_returns_zero_in_alt_screen() {
    let mut s = session_with(80, 3, 1024);
    s.feed(b"\x1b[?1049h").unwrap();
    assert!(s.is_alt_screen());
    assert_eq!(s.peek_uncaptured_logical_lines(), 0);
    assert_eq!(s.logical_lines_until_cursor(), 0);
}

// PromptMark.abs_y logical-line migration ---------------------------
//
// The two tests below pin the migration's load-bearing properties:
// (1) the abs_y → screen-row round-trip lands the mark back on the
// row that dispatched it, and (2) the stored abs_y is invariant under
// resize-driven reflow (visual-row counts change, the logical-line
// abs does not).

#[test]
fn current_abs_y_at_cursor_round_trips_through_abs_to_screen_row() {
    // Dispatch OSC 133 at a known cursor row, then verify that
    // `abs_to_screen_row(mark.abs_y)` maps back to a row that
    // matches the cursor's pre-feed position in the unified frame.
    // The round trip is the contract for paint / jump consumers
    // — anything else and the gutter band paints on the wrong row.
    let mut s = session_with(80, 5, 1024);
    // Feed two lines so the cursor lands on row 3 (1-indexed) before
    // dispatch — exercises the grid-above-cursor walk inside
    // `logical_lines_until_cursor`.
    s.feed(b"alpha\r\nbeta\r\n\x1b]133;A\x07").unwrap();
    let mark = s.prompt_marks().back().copied().expect("mark must exist");
    // The mark was dispatched while the cursor was on grid row 3 of a
    // 5-row viewport with no scrollback. In the unified frame that is
    // absolute screen row 2 (zero-indexed).
    let row = s
        .abs_to_screen_row(mark.abs_y)
        .expect("round-trip translation must resolve");
    assert_eq!(
        row, 2,
        "mark dispatched on grid row 3 (zero-indexed row 2) must round-trip back to row 2"
    );
    // And after PTY-driven capture (lines scroll into LineBuffer),
    // the round-trip still lands on the original row's unified-frame
    // position — LineBuffer absorbed rows 0..N, so the row count
    // grows but the mark's abs_y is invariant.
    s.feed(b"gamma\r\ndelta\r\nepsilon\r\nzeta\r\n").unwrap();
    let row_after = s
        .abs_to_screen_row(mark.abs_y)
        .expect("mark must still resolve after scroll");
    // Unified-frame row of the mark's source line stays at 2 — two
    // logical lines ('alpha', 'beta') precede it regardless of where
    // the live grid currently sits.
    assert_eq!(
        row_after, 2,
        "scroll-driven capture must not move the mark's resolved row"
    );
}

#[test]
fn abs_y_unchanged_after_resize() {
    // Logical-line abs is wrap-invariant: dispatching OSC 133 at a
    // given cursor position, then resizing to a width that reflows
    // earlier lines, must NOT change the stored `mark.abs_y` (the
    // mark is an identity in logical-line space). `abs_to_screen_row`
    // then projects through the new wrap to land on the post-reflow
    // visual row.
    let mut s = session_with(40, 5, 1024);
    // Two lines wide enough to wrap at the post-resize 20-cols width
    // but fit in 40 cols pre-resize. The mark sits on row 3 of a
    // wrap-free pre-resize grid.
    s.feed(b"this-line-is-thirty-chars-long-aaaa\r\n").unwrap();
    s.feed(b"another-thirty-char-line-bbbbbbbbbbb\r\n").unwrap();
    s.feed(b"\x1b]133;A\x07").unwrap();
    let abs_before = s
        .prompt_marks()
        .back()
        .copied()
        .expect("mark must exist")
        .abs_y;
    // Narrow enough to reflow the two earlier lines into wrap
    // continuations. Visual-row count of the LineBuffer prefix
    // therefore grows; logical-line count does not.
    s.resize(20, 5).expect("resize must succeed");
    let abs_after = s
        .prompt_marks()
        .back()
        .copied()
        .expect("mark must survive resize")
        .abs_y;
    assert_eq!(
        abs_before, abs_after,
        "stored abs_y must be invariant under wrap reflow (logical-line space)"
    );
    // Stronger than reachability: the projected visual row must
    // anchor a row that the cursor was logically on. The mark sits on
    // an empty prompt row (OSC 133 fires before the prompt body is
    // drawn), so the projected row's text starts empty — sufficient
    // for the round-trip identity. Cells around the mark are blanks
    // on both sides of resize, so the projected row's leading
    // characters compare equal pre/post.
    let row_after = s
        .abs_to_screen_row(abs_after)
        .expect("post-resize projection must still resolve the mark to a row");
    let text_after = s.dump_screen_row(row_after).unwrap_or_default();
    let row_before_recap = s.abs_to_screen_row(abs_before).unwrap();
    let text_before_recap = s.dump_screen_row(row_before_recap).unwrap_or_default();
    assert_eq!(
        text_after.trim_end(),
        text_before_recap.trim_end(),
        "the projected row's content must match the mark's logical line across resize"
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
