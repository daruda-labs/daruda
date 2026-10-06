//! Terminal keymap logic split out of `view::input`.
//!
//! GPUI-free translation of keystrokes into the bytes sent to the PTY.
//! Currently hosts the macOS-native "Natural Text Editing" remap: a
//! `Cmd`/`Opt` + arrow/delete keystroke becomes the equivalent readline
//! byte sequence the shell already understands (mirroring iTerm2's preset).
//! The caller (`view::input`) gates this behind the `natural_text_editing`
//! config toggle and forwards the returned bytes straight to the PTY.

use crate::ansi;

/// Map a keystroke to its Natural Text Editing byte sequence, or `None`
/// when the keystroke is not part of the preset.
///
/// The modifier set must match a table entry **exactly** — any extra
/// modifier yields `None`. This keeps `Cmd+Alt+Arrow` (pane focus) and
/// `Cmd+Shift+Arrow` (prompt jump) clear of the remap.
pub fn natural_text_editing_bytes(
    key: &str,
    shift: bool,
    ctrl: bool,
    alt: bool,
    platform: bool,
) -> Option<&'static [u8]> {
    // No table entry uses Ctrl or Shift; bail before matching so those
    // combinations fall through to their existing handlers.
    if ctrl || shift {
        return None;
    }

    match (platform, alt, key) {
        // Cmd + key — line-scoped edits.
        (true, false, "left") => Some(ansi::READLINE_LINE_START),
        (true, false, "right") => Some(ansi::READLINE_LINE_END),
        (true, false, "backspace") => Some(ansi::READLINE_KILL_TO_LINE_START),
        // Opt + key — word-scoped edits.
        (false, true, "left") => Some(ansi::READLINE_WORD_BACK),
        (false, true, "right") => Some(ansi::READLINE_WORD_FORWARD),
        (false, true, "backspace") => Some(ansi::READLINE_DELETE_WORD_BACK),
        (false, true, "delete") => Some(ansi::READLINE_DELETE_WORD_FORWARD),
        // Forward-delete key with no modifier.
        (false, false, "delete") => Some(ansi::READLINE_DELETE_CHAR_FORWARD),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (shift, ctrl, alt, platform)
    const CMD: (bool, bool, bool, bool) = (false, false, false, true);
    const OPT: (bool, bool, bool, bool) = (false, false, true, false);
    const NONE: (bool, bool, bool, bool) = (false, false, false, false);

    fn map(key: &str, m: (bool, bool, bool, bool)) -> Option<&'static [u8]> {
        natural_text_editing_bytes(key, m.0, m.1, m.2, m.3)
    }

    #[test]
    fn cmd_left_moves_to_line_start() {
        assert_eq!(map("left", CMD), Some(&b"\x01"[..]));
    }

    #[test]
    fn cmd_right_moves_to_line_end() {
        assert_eq!(map("right", CMD), Some(&b"\x05"[..]));
    }

    #[test]
    fn cmd_backspace_kills_to_line_start() {
        assert_eq!(map("backspace", CMD), Some(&b"\x15"[..]));
    }

    #[test]
    fn opt_left_moves_word_back() {
        assert_eq!(map("left", OPT), Some(&b"\x1bb"[..]));
    }

    #[test]
    fn opt_right_moves_word_forward() {
        assert_eq!(map("right", OPT), Some(&b"\x1bf"[..]));
    }

    #[test]
    fn opt_backspace_deletes_word_back() {
        assert_eq!(map("backspace", OPT), Some(&b"\x1b\x7f"[..]));
    }

    #[test]
    fn opt_delete_deletes_word_forward() {
        assert_eq!(map("delete", OPT), Some(&b"\x1bd"[..]));
    }

    #[test]
    fn plain_delete_deletes_char_forward() {
        assert_eq!(map("delete", NONE), Some(&b"\x04"[..]));
    }

    #[test]
    fn unmodified_arrow_is_unmapped() {
        assert_eq!(map("left", NONE), None);
        assert_eq!(map("right", NONE), None);
    }

    #[test]
    fn cmd_alt_left_is_unmapped() {
        // Cmd+Alt+Arrow is pane focus — must not be remapped.
        assert_eq!(map("left", (false, false, true, true)), None);
    }

    #[test]
    fn cmd_shift_left_is_unmapped() {
        // Cmd+Shift+Arrow is prompt jump — must not be remapped.
        assert_eq!(map("left", (true, false, false, true)), None);
    }

    #[test]
    fn ctrl_combo_is_unmapped() {
        assert_eq!(map("left", (false, true, false, true)), None);
    }

    #[test]
    fn plain_backspace_is_unmapped() {
        // Plain backspace keeps its existing DEL handling.
        assert_eq!(map("backspace", NONE), None);
    }
}
