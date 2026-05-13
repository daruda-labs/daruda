//! UTF-8 char-boundary safe string navigation.
//!
//! Shared by the search-input cursor (Left / Right / Backspace /
//! Delete / click-to-position) and by `text_metrics` to clamp byte
//! indices into a valid char boundary before handing them to GPUI's
//! `ShapedLine::x_for_index` (which panics on a non-boundary).

/// Move `byte` left by one character in `text`, clamped to 0.
/// Returns the byte at the start of the *previous* character.
pub(crate) fn step_char_left(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == 0 {
        return 0;
    }
    let mut prev = byte - 1;
    while prev > 0 && !text.is_char_boundary(prev) {
        prev -= 1;
    }
    prev
}

/// Move `byte` right by one character in `text`, clamped to `text.len()`.
/// Returns the byte at the start of the *next* character (or
/// `text.len()` when already at the end).
pub(crate) fn step_char_right(text: &str, byte: usize) -> usize {
    let len = text.len();
    let byte = byte.min(len);
    if byte == len {
        return len;
    }
    let mut next = byte + 1;
    while next < len && !text.is_char_boundary(next) {
        next += 1;
    }
    next
}

/// Round `byte` *down* to the nearest UTF-8 char boundary in `text`.
///
/// `line_text` (from `viewport_lines`, i.e. `dump_viewport`) and
/// `shaped.text` (re-shaped through GPUI) can disagree on trailing
/// whitespace handling, which means a byte offset valid in one
/// string may land mid-codepoint in the other. `ShapedLine::
/// x_for_index` panics on a non-boundary, so this helper rounds down
/// to the nearest valid boundary.
pub(crate) fn clamp_to_char_boundary(text: &str, mut byte: usize) -> usize {
    if byte > text.len() {
        return text.len();
    }
    while byte > 0 && !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_char_left_walks_multibyte() {
        let text = "한a한"; // 3 + 1 + 3 bytes
        assert_eq!(step_char_left(text, 0), 0);
        assert_eq!(step_char_left(text, 3), 0);
        assert_eq!(step_char_left(text, 4), 3);
        assert_eq!(step_char_left(text, 7), 4);
        // Mid-codepoint clamps down before stepping.
        assert_eq!(step_char_left(text, 99), 4);
    }

    #[test]
    fn step_char_right_walks_multibyte() {
        let text = "한a한";
        assert_eq!(step_char_right(text, 0), 3);
        assert_eq!(step_char_right(text, 3), 4);
        assert_eq!(step_char_right(text, 4), 7);
        assert_eq!(step_char_right(text, 7), 7);
    }

    #[test]
    fn clamp_to_char_boundary_rounds_into_multibyte_glyph() {
        let text = "한a";
        assert_eq!(clamp_to_char_boundary(text, 0), 0);
        assert_eq!(clamp_to_char_boundary(text, 1), 0);
        assert_eq!(clamp_to_char_boundary(text, 2), 0);
        assert_eq!(clamp_to_char_boundary(text, 3), 3);
        assert_eq!(clamp_to_char_boundary(text, 4), 4);
        assert_eq!(clamp_to_char_boundary(text, 99), text.len());
    }

    #[test]
    fn clamp_to_char_boundary_no_op_on_ascii() {
        let text = "hello";
        for i in 0..=text.len() {
            assert_eq!(clamp_to_char_boundary(text, i), i);
        }
    }
}
