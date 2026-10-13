use super::*;

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
