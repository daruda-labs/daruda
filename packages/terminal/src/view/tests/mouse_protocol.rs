// ============================================================================
// Normal mouse protocol + UTF-8 encoding
// ============================================================================

#[test]
fn normal_mouse_sequence_press_single_byte_coords() {
    use super::super::mouse::normal_mouse_sequence;
    // Left press (button=0), col=1, row=1, no utf8
    // Expected: ESC [ M (32+0) (32+1) (32+1) = ESC [ M ' ' '!' '!'
    let seq = normal_mouse_sequence(0, 1, 1, false);
    assert_eq!(seq, b"\x1b[M !!");
}

#[test]
fn normal_mouse_sequence_release_uses_button3() {
    use super::super::mouse::normal_mouse_sequence;
    // Release in Normal mode should be sent as button 3 by the caller.
    // button=3, col=1, row=1 → 32+3=35='#', coords 33='!'
    let seq = normal_mouse_sequence(3, 1, 1, false);
    assert_eq!(seq, b"\x1b[M#!!");
}

#[test]
fn normal_mouse_sequence_out_of_range_returns_empty() {
    use super::super::mouse::normal_mouse_sequence;
    // Without UTF-8, col > 222 (0-indexed) is out of range.
    let seq = normal_mouse_sequence(0, 224, 1, false);
    assert!(seq.is_empty(), "should be empty for out-of-range col");
}

#[test]
fn normal_mouse_sequence_utf8_extends_range() {
    use super::super::mouse::normal_mouse_sequence;
    // With UTF-8, col=224 (0-indexed=223, within 2014 limit) is valid.
    let seq = normal_mouse_sequence(0, 224, 1, true);
    assert!(!seq.is_empty(), "UTF-8 mode should encode col=224");
    assert_eq!(&seq[..3], b"\x1b[M");
    assert_eq!(seq[3], 32); // button 0 + 32
    // col0=223, v=32+1+223=256: first=0xC0+256/64=0xC4, second=0x80+(256&63)=0x80
    assert_eq!(seq[4], 0xC4);
    assert_eq!(seq[5], 0x80);
}

#[test]
fn normal_mouse_sequence_boundary_col95_uses_utf8() {
    use super::super::mouse::normal_mouse_sequence;
    // col=96 is 0-indexed 95 — exactly the UTF-8 threshold.
    let without = normal_mouse_sequence(0, 96, 1, false);
    let with_utf8 = normal_mouse_sequence(0, 96, 1, true);
    // Without UTF-8: single byte 32+1+95=128 (wraps/truncates to u8)
    assert_eq!(without.len(), 6); // 3 header + button + col(1 byte) + row(1 byte)
    // With UTF-8: col is encoded as 2 bytes
    assert_eq!(with_utf8.len(), 7); // 3 header + button + col(2 bytes) + row(1 byte)
}

// ============================================================================
// Alternate-scroll mode: session parses DECSET 1007
// ============================================================================

#[test]
fn alternate_scroll_defaults_to_enabled() {
    use crate::session::TerminalSession;
    use crate::{TerminalConfig, TerminalDims};
    let dims = TerminalDims { cols: 80, rows: 24 };
    let cfg = TerminalConfig::default();
    let session = TerminalSession::new(dims, cfg).unwrap();
    assert!(session.alternate_scroll_enabled());
}

#[test]
fn alternate_scroll_disabled_by_decset_reset() {
    use crate::session::TerminalSession;
    use crate::{TerminalConfig, TerminalDims};
    let dims = TerminalDims { cols: 80, rows: 24 };
    let cfg = TerminalConfig::default();
    let mut session = TerminalSession::new(dims, cfg).unwrap();
    assert!(session.alternate_scroll_enabled());
    let _ = session.feed(b"\x1b[?1007l"); // disable
    assert!(!session.alternate_scroll_enabled());
    let _ = session.feed(b"\x1b[?1007h"); // re-enable
    assert!(session.alternate_scroll_enabled());
}

// ============================================================================
// UTF-8 mouse mode: session parses DECSET 1005
// ============================================================================

#[test]
fn mouse_utf8_mode_parsed_correctly() {
    use crate::session::TerminalSession;
    use crate::{TerminalConfig, TerminalDims};
    let dims = TerminalDims { cols: 80, rows: 24 };
    let cfg = TerminalConfig::default();
    let mut session = TerminalSession::new(dims, cfg).unwrap();
    assert!(!session.mouse_utf8_enabled());
    let _ = session.feed(b"\x1b[?1005h");
    assert!(session.mouse_utf8_enabled());
    let _ = session.feed(b"\x1b[?1005l");
    assert!(!session.mouse_utf8_enabled());
}

// ============================================================================
// vt_codes: CsiMode round-trip for new codes
// ============================================================================

#[test]
fn csi_mode_new_codes_round_trip() {
    use crate::vt_codes::CsiMode;
    assert_eq!(CsiMode::from_raw(1005), Some(CsiMode::MouseUtf8));
    assert_eq!(CsiMode::from_raw(1007), Some(CsiMode::AlternateScroll));
}
