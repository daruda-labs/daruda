//! ANSI / xterm escape sequences and byte-level helpers.
//!
//! Ready-to-send byte slices, format templates for synthesized replies,
//! and constructor helpers for the escape families we emit the most
//! (CSI mode set/reset, OSC).
//!
//! Keep this file focused on **bytes we write to the PTY** (or format
//! to send back in response to a query). Numeric parameter codes live
//! in [`crate::vt_codes`]; buffer sizes in [`crate::vt_limits`].

// ============================================================================
// Raw control bytes
// ============================================================================

/// `ESC` — introducer for every CSI / OSC sequence.
pub const ESC: u8 = 0x1b;
/// `BEL` — terminates an OSC string (7-bit form).
pub const BEL: u8 = 0x07;
/// Final byte of the ST 2-byte terminator (`ESC \`). See [`ST`] for the
/// full 2-byte slice when emitting.
pub const ST_FINAL: u8 = b'\\';
/// Full String Terminator (`ESC \`) — preferred to BEL by modern xterm.
pub const ST: &[u8] = b"\x1b\\";

// ============================================================================
// Bracketed paste — `CsiMode::BracketedPaste`
// ============================================================================

/// Prefix we wrap pasted text with when the shell has enabled bracketed
/// paste. Matches xterm / `CSI ? 2004 h` semantics.
pub const PTY_BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub const PTY_BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

// ============================================================================
// Focus event reporting — `CsiMode::FocusEvent`
// ============================================================================

/// Sent to PTY when the terminal window gains focus (DECSET 1004).
pub const FOCUS_IN: &[u8] = b"\x1b[I";
/// Sent to PTY when the terminal window loses focus (DECSET 1004).
pub const FOCUS_OUT: &[u8] = b"\x1b[O";

// ============================================================================
// Device Status Report / Cursor Position Report replies
// ============================================================================

/// DSR "OK" reply — response to `CSI 5 n`.
pub const DSR_OK: &[u8] = b"\x1b[0n";

/// Build a Cursor Position Report reply (`CSI row ; col R`) — response
/// to `CSI 6 n`. `row` / `col` are 1-indexed per the spec.
pub fn cpr_reply(row: u16, col: u16) -> String {
    format!("\x1b[{};{}R", row, col)
}

// ============================================================================
// OSC color query replies (Ps = 10 / 11)
// ============================================================================

/// Build the reply to an OSC color query (`OSC 10 ?` / `OSC 11 ?`).
/// xterm replies with 16-bit channels — each 8-bit component is
/// duplicated (`0xAB` → `0xABAB`) so it round-trips losslessly.
pub fn osc_color_reply(ps: u32, rgb: (u8, u8, u8)) -> String {
    let (r, g, b) = rgb;
    let r16 = u16::from(r) * 0x0101;
    let g16 = u16::from(g) * 0x0101;
    let b16 = u16::from(b) * 0x0101;
    format!("\x1b]{};rgb:{:04x}/{:04x}/{:04x}\x1b\\", ps, r16, g16, b16)
}

// ============================================================================
// OSC 4 — ANSI palette color set
// ============================================================================

/// Build an OSC 4 sequence to set a single palette entry.
/// `index` is 0–255; the format matches xterm/ghostty expectations.
pub fn osc4_set_color(index: u8, r: u8, g: u8, b: u8) -> String {
    format!("\x1b]4;{};rgb:{:02x}/{:02x}/{:02x}\x1b\\", index, r, g, b)
}

// ============================================================================
// Generic CSI / OSC constructors
// ============================================================================

/// `CSI ? n h` — set a DEC private mode (`h` for set, per DEC vt).
pub fn csi_mode_set(n: u32) -> String {
    format!("\x1b[?{}h", n)
}

/// `CSI ? n l` — reset a DEC private mode.
pub fn csi_mode_reset(n: u32) -> String {
    format!("\x1b[?{}l", n)
}

/// `OSC Ps ; body ST` — body is UTF-8; terminator is ST (`ESC \`).
pub fn osc(ps: u32, body: &str) -> String {
    format!("\x1b]{};{}\x1b\\", ps, body)
}

// ============================================================================
// Cursor movement sequences (used for alternate-scroll mode)
// ============================================================================

/// `CSI A` — cursor up / arrow up.
/// Sent in place of scroll-up events when alternate-scroll mode is active.
pub const CURSOR_UP: &[u8] = b"\x1b[A";
/// `CSI B` — cursor down / arrow down.
/// Sent in place of scroll-down events when alternate-scroll mode is active.
pub const CURSOR_DOWN: &[u8] = b"\x1b[B";

/// `CSI Z` — Cursor Backward Tab (CBT) — emitted on Shift+Tab.
pub const CBT: &[u8] = b"\x1b[Z";

// ============================================================================
// SGR (1006) mouse reporting
// ============================================================================

/// Build an SGR-extended mouse report (`CSI < button ; col ; row M|m`).
/// `pressed` selects the final byte (`M` = press / move, `m` = release).
/// `button` is the encoded SGR button value (low 7 bits + modifier and
/// motion bits) computed by the caller.
pub fn sgr_mouse_report(button: u8, col: u16, row: u16, pressed: bool) -> String {
    let suffix = if pressed { 'M' } else { 'm' };
    format!("\x1b[<{};{};{}{}", button, col, row, suffix)
}

// ============================================================================
// Alt-screen / clear-screen helpers
// ============================================================================

/// Erase the scrollback buffer (`ED 3`).
/// Injected into ghostty on alt-screen entry to discard accumulated
/// primary-screen history so it cannot be scrolled back to while the
/// TUI is running.
pub const ERASE_SCROLLBACK: &[u8] = b"\x1b[3J";

/// Erase entire display and move cursor to top-left.
/// Injected into ghostty after an alt-screen exit to clear the restored
/// primary screen so old terminal output doesn't show through.
pub const ERASE_DISPLAY_AND_HOME: &[u8] = b"\x1b[2J\x1b[H";

// ============================================================================
// URI schemes that appear inside OSC payloads
// ============================================================================

/// OSC 7 payload format: `file://<hostname>/<path>`. The shell reports
/// its cwd with this scheme; we strip it and keep the local path.
pub const OSC7_FILE_SCHEME: &str = "file://";

// ============================================================================
// Terminal capability query responses
// ============================================================================

/// Primary DA response (`CSI ? 62 ; 22 c`) — VT220 (62), ANSI colour (22).
pub const PRIMARY_DA: &[u8] = b"\x1b[?62;22c";

/// Secondary DA response (`CSI > 0 ; 100 ; 0 c`) — xterm-compat type,
/// version 100, no ROM cartridge.
pub const SECONDARY_DA: &[u8] = b"\x1b[>0;100;0c";

/// Tertiary DA response — DCS device-identifier sequence with all-zero ID.
pub const TERTIARY_DA: &[u8] = b"\x1bP!|00000000\x1b\\";

/// XTVERSION response (`DCS > | NAME VERSION ST`).
pub const XTVERSION: &[u8] = b"\x1bP>|daruda 0.1.0\x1b\\";

/// Kitty keyboard query response — zero flags (protocol not supported).
pub const KITTY_KEYBOARD_RESPONSE: &[u8] = b"\x1b[?0u";

/// Build a DECRQM reply (`CSI ? Ps ; Pm $ y`).
///
/// `pm`: `0` = not recognised, `1` = set, `2` = reset,
/// `3` = permanently set, `4` = permanently reset.
pub fn decrqm_reply(ps: u32, pm: u8) -> String {
    format!("\x1b[?{};{}$y", ps, pm)
}

/// Build a successful XTGETTCAP response for one capability.
///
/// `key_hex` is the hex-encoded capability name as received; `value` is
/// the plain-text answer, which will be hex-encoded in the reply.
pub fn xtgettcap_found(key_hex: &str, value: &str) -> String {
    let value_hex: String = value.bytes().map(|b| format!("{:02x}", b)).collect();
    format!("\x1bP1+r{}={}\x1b\\", key_hex, value_hex)
}

/// Build a not-found XTGETTCAP response for one capability.
pub fn xtgettcap_not_found(key_hex: &str) -> String {
    format!("\x1bP0+r{}\x1b\\", key_hex)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpr_is_1_indexed() {
        assert_eq!(cpr_reply(1, 1), "\x1b[1;1R");
        assert_eq!(cpr_reply(24, 80), "\x1b[24;80R");
    }

    #[test]
    fn osc_color_reply_doubles_each_channel() {
        assert_eq!(
            osc_color_reply(10, (0xAB, 0xCD, 0xEF)),
            "\x1b]10;rgb:abab/cdcd/efef\x1b\\"
        );
    }

    #[test]
    fn csi_mode_helpers_match_xterm() {
        assert_eq!(csi_mode_set(2004), "\x1b[?2004h");
        assert_eq!(csi_mode_reset(1006), "\x1b[?1006l");
    }

    #[test]
    fn osc_builder_uses_st_terminator() {
        assert_eq!(osc(0, "title"), "\x1b]0;title\x1b\\");
    }

    #[test]
    fn osc4_set_color_formats_correctly() {
        assert_eq!(
            osc4_set_color(0, 0x00, 0x00, 0x00),
            "\x1b]4;0;rgb:00/00/00\x1b\\"
        );
        assert_eq!(
            osc4_set_color(15, 0xFF, 0xEE, 0xDD),
            "\x1b]4;15;rgb:ff/ee/dd\x1b\\"
        );
    }

    #[test]
    fn osc4_covers_full_ansi_range() {
        // Indices 0–15 should all produce valid sequences.
        for i in 0..16u8 {
            let seq = osc4_set_color(i, 0xAA, 0xBB, 0xCC);
            assert!(seq.starts_with("\x1b]4;"));
            assert!(seq.ends_with("\x1b\\"));
            assert!(seq.contains(&format!("{};", i)));
        }
    }
}
