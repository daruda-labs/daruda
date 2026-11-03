use std::ops::Range;

#[derive(Clone, Copy)]
enum CharacterKind {
    Word,
    Whitespace,
    Newline,
    Other,
}

impl From<char> for CharacterKind {
    fn from(character: char) -> Self {
        // daruda patch: any-script alphanumerics are word characters, so a
        // double click selects a Hangul or CJK word rather than one syllable.
        if character == '_' || character.is_alphanumeric() {
            Self::Word
        } else if matches!(character, '\n' | '\r') {
            Self::Newline
        } else if character.is_whitespace() {
            Self::Whitespace
        } else {
            Self::Other
        }
    }
}

pub(crate) fn word_range_from_chars(
    offset: usize,
    character: char,
    previous: impl Iterator<Item = char>,
    following: impl Iterator<Item = char>,
) -> Range<usize> {
    let kind = CharacterKind::from(character);
    let connects = |next| {
        matches!(
            (kind, CharacterKind::from(next)),
            (CharacterKind::Word, CharacterKind::Word)
                | (CharacterKind::Whitespace, CharacterKind::Whitespace)
        )
    };
    let start = previous
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset, |offset, character| offset - character.len_utf8());
    let end = following
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset + character.len_utf8(), |offset, character| {
            offset + character.len_utf8()
        });
    start..end
}

pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = clip_offset_left(text, offset);
    let character = text[offset..].chars().next()?;
    let end = offset + character.len_utf8();
    Some(word_range_from_chars(
        offset,
        character,
        text[..offset].chars().rev(),
        text[end..].chars(),
    ))
}

pub(crate) fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clip_offset_left(text, offset);
    let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |newline| offset + newline);
    start..end
}

fn clip_offset_left(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
