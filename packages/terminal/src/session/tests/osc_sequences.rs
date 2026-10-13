use super::*;

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

#[test]
fn command_activity_begins_at_execution_and_ends_at_finish_or_new_prompt() {
    let mut session =
        TerminalSession::new(TerminalDims::default(), TerminalConfig::default()).unwrap();
    for (mark, running) in [
        (b'B', false),
        (b'C', true),
        (b'D', false),
        (b'C', true),
        (b'A', false),
    ] {
        session
            .feed(&[0x1b, b']', b'1', b'3', b'3', b';', mark, 7])
            .unwrap();
        assert_eq!(session.command_is_running(), running);
    }
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
