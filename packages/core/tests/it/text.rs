//! Public API tests for text.

use daruda_core::text::*;

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
