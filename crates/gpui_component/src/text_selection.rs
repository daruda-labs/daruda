/// Shared text-selection primitives used by both the input widget
/// (ropey::Rope, byte offsets) and future plain-text consumers (&str).
///
/// **Scope**: word-boundary logic only.  SelectMode, mouse branching, line
/// range, drag extension, and selection state remain per-widget.
use std::ops::Range;

/// Category of a character for word-boundary detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharType {
    /// a–z, A–Z, 0–9, _
    Word,
    /// '\t', ' ', '\u{00A0}', and other Unicode whitespace (excluding newlines)
    Whitespace,
    /// '\n', '\r'
    Newline,
    /// Punctuation, CJK characters, emoji, etc.
    Other,
}

impl CharType {
    /// Classify a single Unicode character.
    pub fn from_char(c: char) -> Self {
        match c {
            '_' => CharType::Word,
            c if c.is_ascii_alphanumeric() => CharType::Word,
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
/// - `len`     – total byte length of the text.
/// - `char_at` – closure that maps a byte offset to the `char` that *starts*
///               at that offset, or `None` when the offset is past the end or
///               is not a UTF-8 character boundary.
/// - `offset`  – byte offset of the character to expand from.
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
    fn test_word_boundary_ascii() {
        let s = "hello world";
        assert_eq!(word(s, 0), Some("hello".into()));
        assert_eq!(word(s, 4), Some("hello".into()));
        assert_eq!(word(s, 5), Some(" ".into()));
        assert_eq!(word(s, 6), Some("world".into()));
        assert_eq!(word(s, 10), Some("world".into()));
    }

    #[test]
    fn test_word_boundary_underscore() {
        let s = "foo_bar baz";
        assert_eq!(word(s, 0), Some("foo_bar".into()));
        assert_eq!(word(s, 6), Some("foo_bar".into()));
        assert_eq!(word(s, 7), Some(" ".into()));
        assert_eq!(word(s, 8), Some("baz".into()));
    }

    #[test]
    fn test_word_boundary_punctuation() {
        let s = "a.b[c]";
        assert_eq!(word(s, 0), Some("a".into()));
        assert_eq!(word(s, 1), Some(".".into()));
        assert_eq!(word(s, 2), Some("b".into()));
        assert_eq!(word(s, 3), Some("[".into()));
        assert_eq!(word(s, 4), Some("c".into()));
        assert_eq!(word(s, 5), Some("]".into()));
    }

    #[test]
    fn test_word_boundary_newline() {
        let s = "foo\nbar";
        assert_eq!(word(s, 0), Some("foo".into()));
        assert_eq!(word(s, 3), Some("\n".into()));
        assert_eq!(word(s, 4), Some("bar".into()));
    }

    #[test]
    fn test_word_boundary_out_of_bounds() {
        let s = "hi";
        assert_eq!(word(s, 2), None); // offset == len
        assert_eq!(word(s, 99), None);
    }

    #[test]
    fn test_word_boundary_multibyte() {
        // "中文" — each char is 3 bytes
        let s = "中文";
        assert_eq!(word(s, 0), Some("中".into()));
        assert_eq!(word(s, 3), Some("文".into()));
    }

    #[test]
    fn test_char_type_from_char() {
        assert_eq!(CharType::from_char('a'), CharType::Word);
        assert_eq!(CharType::from_char('Z'), CharType::Word);
        assert_eq!(CharType::from_char('0'), CharType::Word);
        assert_eq!(CharType::from_char('_'), CharType::Word);
        assert_eq!(CharType::from_char('.'), CharType::Other);
        assert_eq!(CharType::from_char(' '), CharType::Whitespace);
        assert_eq!(CharType::from_char('\t'), CharType::Whitespace);
        assert_eq!(CharType::from_char('\n'), CharType::Newline);
        assert_eq!(CharType::from_char('\r'), CharType::Newline);
        assert_eq!(CharType::from_char('汉'), CharType::Other);
    }
}
