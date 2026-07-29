//! UTF-8 text navigation shared across the GPUI boundary.
//!
//! Byte-offset word and line expansion, plus the selection hit-test that
//! turns a pixel span into "is this cell selected". Everything here takes
//! `&str`-shaped input through a `char_at` closure rather than a concrete
//! string type, so a `ropey::Rope`-backed editor and a plain `&str` reader
//! share one implementation.
//!
//! Deliberately *not* here: what counts as a word. The editor treats
//! `foo/bar.txt` as three tokens while the terminal treats it as one path,
//! and both are right for their surface. [`CharType`] is the editor policy;
//! a caller wanting different rules walks with its own predicate.
//!
//! For plain char-boundary rounding reach for std's
//! `str::{floor_char_boundary, ceil_char_boundary}` (stable since 1.91) —
//! this module used to carry stand-ins for them, which are no longer needed.
//! A crate whose `rust-version` predates 1.91 cannot call them; `daruda_terminal`
//! still declares 1.86 and keeps its own `text_edit` helpers for that reason.

use std::ops::Range;

/// Category of a character for word-boundary detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharType {
    /// Unicode letters/digits of any script (a–z, 0–9, Hangul, CJK, accented
    /// Latin, …) plus `_`. A run of these is one word token.
    Word,
    /// '\t', ' ', '\u{00A0}', and other Unicode whitespace (excluding newlines)
    Whitespace,
    /// '\n', '\r'
    Newline,
    /// Punctuation, symbols, emoji, etc.
    Other,
}

impl CharType {
    /// Classify a single Unicode character.
    pub fn from_char(c: char) -> Self {
        match c {
            '_' => CharType::Word,
            // Any-script alphanumeric, not just ASCII — so double-click word
            // selection works for Hangul / CJK / accented text, not only a–z.
            c if c.is_alphanumeric() => CharType::Word,
            c if c == '\n' || c == '\r' => CharType::Newline,
            c if c.is_whitespace() => CharType::Whitespace,
            _ => CharType::Other,
        }
    }

    /// Two characters are "connectable" (part of the same word token) when
    /// they share the same connectable type. Only `Word` and `Whitespace` runs
    /// extend; `Newline` and `Other` never connect.
    pub fn is_connectable(self, c: char) -> bool {
        let other = CharType::from_char(c);
        matches!(
            (self, other),
            (CharType::Word, CharType::Word) | (CharType::Whitespace, CharType::Whitespace)
        )
    }
}

impl From<char> for CharType {
    fn from(c: char) -> Self {
        CharType::from_char(c)
    }
}

/// Return the byte range of the word/run that contains `offset`.
///
/// # Parameters
/// - `len` – total byte length of the text.
/// - `char_at` – closure that maps a byte offset to the `char` that *starts* at
///   that offset, or `None` when the offset is past the end or is not a UTF-8
///   character boundary.
/// - `offset` – byte offset of the character to expand from.
///
/// Returns `None` when `offset >= len` or `char_at(offset)` is `None`.
///
/// The walk is capped at 128 characters in each direction so it stays O(1)
/// for practical inputs (matches the upstream gpui-component behaviour).
pub fn word_range(
    len: usize,
    char_at: impl Fn(usize) -> Option<char>,
    offset: usize,
) -> Option<Range<usize>> {
    if offset >= len {
        return None;
    }

    let anchor_char = char_at(offset)?;
    let char_type = CharType::from_char(anchor_char);

    let mut start = offset;
    let mut end = offset + anchor_char.len_utf8();

    // Walk backward (up to 128 chars)
    let mut pos = offset;
    let mut count = 0usize;
    loop {
        if pos == 0 || count >= 128 {
            break;
        }
        // Step back one byte at a time to find the previous char boundary.
        let mut prev = pos - 1;
        while prev > 0 && char_at(prev).is_none() {
            prev -= 1;
        }
        let Some(ch) = char_at(prev) else { break };
        if char_type.is_connectable(ch) {
            start = prev;
            pos = prev;
            count += 1;
        } else {
            break;
        }
    }

    // Walk forward (up to 128 chars)
    let mut pos = end;
    let mut count = 0usize;
    loop {
        if pos >= len || count >= 128 {
            break;
        }
        let Some(ch) = char_at(pos) else {
            pos += 1;
            continue;
        };
        if char_type.is_connectable(ch) {
            end = pos + ch.len_utf8();
            pos = end;
            count += 1;
        } else {
            break;
        }
    }

    Some(start..end)
}

/// Return the byte range of the logical (newline-delimited) line that
/// contains `offset`.
///
/// # Parameters
/// - `len` – total byte length of the text.
/// - `char_at` – closure that maps a byte offset to the `char` starting there,
///   or `None` when the offset is out of range or not a UTF-8 character
///   boundary.
/// - `offset` – byte offset inside the line to locate.
///
/// Always returns a valid `Range<usize>` within `0..len`.  When the text
/// has no newlines the whole range `0..len` is returned.
pub fn logical_line_range(
    len: usize,
    char_at: impl Fn(usize) -> Option<char>,
    offset: usize,
) -> Range<usize> {
    let offset = offset.min(len);
    // Walk backward to find the start (byte after the preceding '\n').
    let start = if offset == 0 {
        0
    } else {
        let mut pos = offset;
        loop {
            if pos == 0 {
                break 0;
            }
            pos -= 1;
            if char_at(pos) == Some('\n') {
                break pos + 1;
            }
        }
    };
    // Walk forward to find the end (the '\n' itself, exclusive; or `len`).
    let end = (offset..len)
        .find(|&i| char_at(i) == Some('\n'))
        .unwrap_or(len);
    start..end
}

/// Whether a character occupying the horizontal cell `[cell_left, cell_left +
/// cell_width)` is covered by a horizontal selection span `[sel_left,
/// sel_right]` (all values in the same 1-D pixel space, e.g. window x).
///
/// Two regimes, keyed on whether the span is degenerate:
/// - **Drag** (`sel_left != sel_right`): a character is selected when its
///   *center* falls within the span. This is the half-character threshold
///   convention used while dragging a selection.
/// - **Click, no drag** (`sel_left == sel_right`): the span collapses to a
///   single point (the click). The character whose cell *contains* that point
///   is hit. Without this case a word/line click — which produces a zero-width
///   selection box and re-expands from the raw pixel scan — would match no
///   character (center == point is effectively never true), so nothing gets
///   selected.
pub fn char_cell_hit_x(cell_left: f32, cell_width: f32, sel_left: f32, sel_right: f32) -> bool {
    if sel_left == sel_right {
        cell_left <= sel_left && sel_left < cell_left + cell_width
    } else {
        let center = cell_left + cell_width / 2.0;
        center >= sel_left && center <= sel_right
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: char_at for a plain &str, byte-offset based.
    fn str_char_at(s: &str, byte_offset: usize) -> Option<char> {
        if byte_offset >= s.len() {
            return None;
        }
        if !s.is_char_boundary(byte_offset) {
            return None;
        }
        s[byte_offset..].chars().next()
    }

    fn word(s: &str, offset: usize) -> Option<String> {
        let range = word_range(s.len(), |i| str_char_at(s, i), offset)?;
        Some(s[range].to_string())
    }

    #[test]
    fn word_boundary_ascii() {
        let s = "hello world";
        assert_eq!(word(s, 0), Some("hello".into()));
        assert_eq!(word(s, 4), Some("hello".into()));
        assert_eq!(word(s, 5), Some(" ".into()));
        assert_eq!(word(s, 6), Some("world".into()));
        assert_eq!(word(s, 10), Some("world".into()));
    }

    #[test]
    fn word_boundary_underscore() {
        let s = "foo_bar baz";
        assert_eq!(word(s, 0), Some("foo_bar".into()));
        assert_eq!(word(s, 6), Some("foo_bar".into()));
        assert_eq!(word(s, 7), Some(" ".into()));
        assert_eq!(word(s, 8), Some("baz".into()));
    }

    #[test]
    fn word_boundary_punctuation() {
        let s = "a.b[c]";
        assert_eq!(word(s, 0), Some("a".into()));
        assert_eq!(word(s, 1), Some(".".into()));
        assert_eq!(word(s, 2), Some("b".into()));
        assert_eq!(word(s, 3), Some("[".into()));
        assert_eq!(word(s, 4), Some("c".into()));
        assert_eq!(word(s, 5), Some("]".into()));
    }

    #[test]
    fn word_boundary_newline() {
        let s = "foo\nbar";
        assert_eq!(word(s, 0), Some("foo".into()));
        assert_eq!(word(s, 3), Some("\n".into()));
        assert_eq!(word(s, 4), Some("bar".into()));
    }

    #[test]
    fn word_boundary_out_of_bounds() {
        let s = "hi";
        assert_eq!(word(s, 2), None); // offset == len
        assert_eq!(word(s, 99), None);
    }

    #[test]
    fn word_boundary_multibyte() {
        // "中文" — each char is 3 bytes. Both are alphanumeric letters, so they
        // form one connectable word (the point of treating CJK/Hangul as Word).
        let s = "中文";
        assert_eq!(word(s, 0), Some("中文".into()));
        assert_eq!(word(s, 3), Some("中文".into()));
    }

    #[test]
    fn word_boundary_hangul() {
        // Hangul is 3 bytes/char; a run is one word, stopping at the space.
        let s = "한글 테스트";
        assert_eq!(word(s, 0), Some("한글".into()));
        assert_eq!(word(s, 3), Some("한글".into())); // second syllable, same word
        assert_eq!(word(s, 6), Some(" ".into()));
        assert_eq!(word(s, 7), Some("테스트".into()));
    }

    #[test]
    fn logical_line_range_splits_on_newlines() {
        let s = "one\ntwo\nthree";
        let range = |o| logical_line_range(s.len(), |i| str_char_at(s, i), o);
        assert_eq!(&s[range(0)], "one");
        assert_eq!(&s[range(3)], "one"); // the '\n' itself belongs to the line before it
        assert_eq!(&s[range(4)], "two");
        assert_eq!(&s[range(9)], "three");
        assert_eq!(&s[range(99)], "three"); // clamped past the end
    }

    #[test]
    fn logical_line_range_without_newlines_is_whole_text() {
        let s = "single line";
        assert_eq!(
            logical_line_range(s.len(), |i| str_char_at(s, i), 4),
            0..s.len()
        );
    }

    #[test]
    fn char_type_classifies_by_script_not_ascii() {
        assert_eq!(CharType::from_char('a'), CharType::Word);
        assert_eq!(CharType::from_char('Z'), CharType::Word);
        assert_eq!(CharType::from_char('0'), CharType::Word);
        assert_eq!(CharType::from_char('_'), CharType::Word);
        assert_eq!(CharType::from_char('.'), CharType::Other);
        assert_eq!(CharType::from_char(' '), CharType::Whitespace);
        assert_eq!(CharType::from_char('\t'), CharType::Whitespace);
        assert_eq!(CharType::from_char('\n'), CharType::Newline);
        assert_eq!(CharType::from_char('\r'), CharType::Newline);
        // CJK / Hangul are letters → Word (so double-click selects them).
        assert_eq!(CharType::from_char('汉'), CharType::Word);
        assert_eq!(CharType::from_char('글'), CharType::Word);
        // Symbols / emoji stay Other.
        assert_eq!(CharType::from_char('😀'), CharType::Other);
    }

    #[test]
    fn char_cell_hit_x_uses_center_for_drag_and_containment_for_click() {
        // Drag span: the cell's center must fall inside it.
        assert!(char_cell_hit_x(10.0, 8.0, 0.0, 20.0)); // center 14 in 0..20
        assert!(!char_cell_hit_x(10.0, 8.0, 0.0, 12.0)); // center 14 outside
        // Degenerate span (a click): the cell must contain the point.
        assert!(char_cell_hit_x(10.0, 8.0, 10.0, 10.0));
        assert!(char_cell_hit_x(10.0, 8.0, 17.9, 17.9));
        assert!(!char_cell_hit_x(10.0, 8.0, 18.0, 18.0)); // exclusive right edge
    }
}
