use std::ops::Range;

use gpui::{Context, Window};
use ropey::Rope;
use sum_tree::Bias;

use crate::{RopeExt as _, input::InputState};
use daruda_core::text;

impl InputState {
    /// Select the word at the given offset on double-click.
    ///
    /// The offset is the UTF-8 offset.
    pub(super) fn select_word(&mut self, offset: usize, _: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = TextSelector::word_range(&self.text, offset) else {
            return;
        };

        self.selected_range = (range.start..range.end).into();
        self.selected_word_range = Some(self.selected_range);
        cx.notify()
    }
}

struct TextSelector;
impl TextSelector {
    /// Select a word in the given text at the specified offset.
    ///
    /// The offset is the UTF-8 byte offset.
    ///
    /// Returns the start and end byte offsets of the selected word.
    pub fn word_range(text: &Rope, offset: usize) -> Option<Range<usize>> {
        let offset = text.clip_offset(offset, Bias::Left);
        text::word_range(text.len(), |i| text.char_at(i), offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_core::text::CharType;
    use ropey::Rope;

    #[test]
    fn test_char_type_from_char() {
        assert_eq!(CharType::from('a'), CharType::Word);
        assert_eq!(CharType::from('Z'), CharType::Word);
        assert_eq!(CharType::from('0'), CharType::Word);
        assert_eq!(CharType::from('_'), CharType::Word);
        assert_eq!(CharType::from('.'), CharType::Other);
        assert_eq!(CharType::from(','), CharType::Other);
        assert_eq!(CharType::from(';'), CharType::Other);
        assert_eq!(CharType::from('!'), CharType::Other);
        assert_eq!(CharType::from('?'), CharType::Other);
        assert_eq!(CharType::from('['), CharType::Other);
        assert_eq!(CharType::from('{'), CharType::Other);
        assert_eq!(CharType::from(' '), CharType::Whitespace);
        assert_eq!(CharType::from('\t'), CharType::Whitespace);
        assert_eq!(CharType::from('\u{00A0}'), CharType::Whitespace);
        assert_eq!(CharType::from('\n'), CharType::Newline);
        assert_eq!(CharType::from('\r'), CharType::Newline);
        assert_eq!(CharType::from('汉'), CharType::Word);
        assert_eq!(CharType::from('é'), CharType::Word);
    }

    #[test]
    fn test_word_range() {
        use indoc::indoc;

        let rope = Rope::from(indoc! {
            r#"
            test text:
            abcde 中文🎉 test
            hello[()]
            test_connector ____
            Rope
            "#
        });

        let tests = vec![
            (0, 0, Some("test")),
            (0, 4, Some(" ")),
            (1, 0, Some("abcde")),
            (1, 4, Some("abcde")),
            (1, 5, Some(" ")),
            (1, 6, Some("中文")),
            (1, 9, Some("中文")),
            (1, 13, Some("🎉")),
            (1, 20, Some("test")),
            (2, 5, Some("[")),
            (2, 6, Some("(")),
            (2, 7, Some(")")),
            (2, 8, Some("]")),
            (3, 5, Some("test_connector")),
            (3, 14, Some(" ")),
            (3, 16, Some("____")),
            (4, 0, Some("Rope")),
        ];

        for (line, column, expected) in tests {
            let line_start_offset = rope.line_start_offset(line);
            let offset = line_start_offset + column;
            let range = TextSelector::word_range(&rope, offset);

            let actual = range.map(|r| rope.slice(r).to_string());
            let expect = expected.map(|s| s.to_string());
            assert_eq!(actual, expect, "line {}, column {}", line, column);
        }
    }
}
