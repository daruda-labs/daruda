//! UTF-8 text navigation shared across the GPUI boundary.
//!
//! Byte-offset word and line expansion, plus the selection hit-test that
//! turns a pixel span into "is this cell selected". Navigation takes a
//! `char_at` closure so rope-backed editors and plain strings can share it.
//! Plain string prefixes use [`char_prefix`].
//!
//! Deliberately *not* here: what counts as a word. The editor treats
//! `foo/bar.txt` as three tokens while the terminal treats it as one path,
//! and both are right for their surface. [`CharType`] is the editor policy;
//! a caller wanting different rules walks with its own predicate.
//!
//! For plain char-boundary rounding reach for std's
//! `str::{floor_char_boundary, ceil_char_boundary}` (stable since 1.91) —
//! this module used to carry stand-ins for them, which are no longer needed.
//! Every crate here declares the workspace floor of 1.95, so they are always
//! callable; clippy's `incompatible_msrv` is what would say otherwise.

use std::ops::Range;

/// Return at most `max_chars` Unicode scalar values and whether text was cut.
/// The prefix borrows from `text`; no ellipsis is added. A combining mark
/// counts separately from its base character, as with [`str::chars`].
pub fn char_prefix(text: &str, max_chars: usize) -> (&str, bool) {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => (&text[..end], true),
        None => (text, false),
    }
}

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
