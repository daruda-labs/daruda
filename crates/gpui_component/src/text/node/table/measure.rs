//! Shape rendered runs and find the line breaks paint can actually use.

use super::super::{Inline, NodeRenderOptions, Paragraph, Table};
use gpui::{
    App, AvailableSpace, FontWeight, HighlightStyle, IntoElement as _, Pixels, Styled as _,
    TextStyle, Window, img, prelude::FluentBuilder as _, px, size,
};
use std::ops::Range;

#[derive(Clone, Copy, Default, Debug)]
pub(super) struct IntrinsicWidths {
    pub min: Pixels,
    pub max: Pixels,
}

impl IntrinsicWidths {
    fn include(&mut self, other: Self) {
        self.min = self.min.max(other.min);
        self.max = self.max.max(other.max);
    }
}

pub(super) fn columns(
    table: &Table,
    options: NodeRenderOptions,
    window: &mut Window,
    cx: &mut App,
) -> Vec<IntrinsicWidths> {
    let mut columns = vec![IntrinsicWidths::default(); table.column_count()];
    let body_style = window.text_style();
    for (row_ix, row) in table.children.iter().enumerate() {
        let mut text_style = body_style.clone();
        if row_ix == 0 {
            text_style.font_weight = FontWeight::BOLD;
        }
        for (column, cell) in columns.iter_mut().zip(&row.children) {
            column.include(paragraph(&cell.children, options, &text_style, window, cx));
        }
    }
    for (ix, column) in columns.iter_mut().enumerate() {
        // px_2 on both sides is one rem; only non-leading cells own a border.
        let chrome = window.rem_size() + if ix > 0 { px(1.) } else { px(0.) };
        column.min = column.min.ceil() + chrome;
        column.max = column.max.ceil().max(column.min - chrome) + chrome;
    }
    columns
}

fn paragraph(
    paragraph: &Paragraph,
    options: NodeRenderOptions,
    text_style: &TextStyle,
    window: &mut Window,
    cx: &mut App,
) -> IntrinsicWidths {
    let mut widths = IntrinsicWidths::default();
    let mut text = String::new();
    let mut highlights = Vec::new();
    // Walks the children in the order `Paragraph::render` does, splitting a run
    // at each image exactly where it starts a new `Inline`. Widths combine by
    // maximum rather than sum because that render stacks its children as blocks.
    for node in &paragraph.children {
        let offset = text.len();
        text.push_str(&node.text);
        if let Some(image) = &node.image {
            widths.include(text_widths(&text, &highlights, text_style, window));
            text.clear();
            highlights.clear();
            let image_size = img(image.url.clone())
                .when_some(image.width, |this, width| this.w(width))
                .into_any_element()
                .layout_as_root(
                    size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                    window,
                    cx,
                );
            widths.include(IntrinsicWidths {
                min: image_size.width,
                max: image_size.width,
            });
            continue;
        }
        highlights = node.fold_highlights(offset, highlights, options, cx);
    }
    widths.include(text_widths(&text, &highlights, text_style, window));
    widths
}

fn text_widths(
    text: &str,
    highlights: &[(Range<usize>, HighlightStyle)],
    style: &TextStyle,
    window: &Window,
) -> IntrinsicWidths {
    let mut widths = IntrinsicWidths::default();
    let mut offset = 0;
    for line in text.split('\n') {
        let end = offset + line.len();
        let line_highlights = highlights
            .iter()
            .filter_map(|(range, highlight)| {
                let start = range.start.max(offset);
                let end = range.end.min(end);
                (start < end).then(|| (start - offset..end - offset, *highlight))
            })
            .collect::<Vec<_>>();
        let runs = Inline::text_runs(line, &line_highlights, style);
        let layout = window.text_system().layout_line(
            line,
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        // Index once so CJK's many break opportunities do not each scan the
        // shaped line from the start. These are glyph positions, not estimates.
        // The binary search below assumes byte indices ascend with x. That
        // holds for LTR runs only, so a bidi cell's minimum is approximate.
        let positions = layout
            .runs
            .iter()
            .flat_map(|run| {
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.index, glyph.position.x))
            })
            .collect::<Vec<_>>();
        let x = |index| {
            let ix = positions.partition_point(|(byte, _)| *byte < index);
            positions.get(ix).map_or(layout.width, |(_, x)| *x)
        };
        let mut start = 0;
        let mut minimum = px(0.);
        for end in supported_linebreaks(line) {
            // A space discarded by wrapping must not inflate min-content.
            let trimmed_end = start + line[start..end].trim_end_matches([' ', '\t']).len();
            minimum = minimum.max(x(trimmed_end) - x(start));
            start = end;
        }
        widths.include(IntrinsicWidths {
            min: minimum,
            max: layout.width,
        });
        offset = end + 1;
    }
    widths
}

fn supported_linebreaks(line: &str) -> Vec<usize> {
    let paint_boundaries = gpui_natural_wrap_boundaries(line);
    let mut boundaries = unicode_linebreak::linebreaks(line)
        .filter_map(|(end, _)| {
            (end == line.len() || paint_boundaries.binary_search(&end).is_ok()).then_some(end)
        })
        .collect::<Vec<_>>();
    if !line.is_empty() && boundaries.last().copied() != Some(line.len()) {
        boundaries.push(line.len());
    }
    boundaries
}

fn gpui_natural_wrap_boundaries(line: &str) -> Vec<usize> {
    let mut boundaries = Vec::new();
    let mut seen_non_whitespace = false;
    let mut previous = '\0';
    for (index, character) in line.char_indices() {
        let is_boundary = if gpui_word_char(character) {
            previous == ' ' && character != ' ' && seen_non_whitespace
        } else {
            character != ' ' && seen_non_whitespace
        };
        if is_boundary {
            boundaries.push(index);
        }
        if character != ' ' {
            seen_non_whitespace = true;
        }
        previous = character;
    }
    boundaries
}

// Mirrors the pinned GPUI LineWrapper predicate. Min-content must not promise
// a natural break that the painter will replace with an arbitrary hard wrap.
fn gpui_word_char(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(character, '\u{00C0}'..='\u{00FF}')
        || matches!(character, '\u{0100}'..='\u{017F}')
        || matches!(character, '\u{0180}'..='\u{024F}')
        || matches!(character, '\u{0400}'..='\u{04FF}')
        || matches!(character, '\u{1E00}'..='\u{1EFF}')
        || matches!(character, '\u{0300}'..='\u{036F}')
        || matches!(character, '\u{0980}'..='\u{09FF}')
        || matches!(
            character,
            '-' | '_'
                | '.'
                | '\''
                | '’'
                | '‘'
                | '$'
                | '%'
                | '@'
                | '#'
                | '^'
                | '~'
                | ','
                | '='
                | ':'
                | ';'
                | '⋯'
        )
}

#[cfg(test)]
mod tests;
