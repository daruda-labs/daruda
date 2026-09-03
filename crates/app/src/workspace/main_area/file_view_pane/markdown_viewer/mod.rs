//! Markdown parser and IR types for the file viewer preview mode.
//!
//! `parse_markdown` converts a raw Markdown string into a flat list of
//! `MdBlock`s using `pulldown-cmark`. Code blocks are syntax-highlighted
//! in-place using the existing `highlighter` infrastructure.
//!
//! This module is the source → IR half. [`plain_text`] flattens the IR back to
//! text for copy, [`resolve`] loads the images and diagrams a block only names
//! and stamps each one's table slot, and the rendering lives in
//! `file_view_pane/render/markdown/`.

mod plain_text;
mod resolve;

#[cfg(test)]
mod tests;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::highlighter::{LanguageHint, highlight_raw_rows};
use super::{VisualRow, VisualRowKind};

pub(in crate::workspace) use self::plain_text::md_block_plain_text;
pub(in crate::workspace) use self::resolve::{lone_image, resolve_all};

// ----------------------------------------------------------------
// Inline IR
// ----------------------------------------------------------------

#[derive(Clone)]
pub(in crate::workspace) enum MdSpan {
    Text(String),
    Bold(Vec<MdSpan>),
    Italic(Vec<MdSpan>),
    Code(String),
    Link {
        children: Vec<MdSpan>,
        url: String,
    },
    Strikethrough(Vec<MdSpan>),
    SoftBreak,
    HardBreak,
    /// The boundary between two paragraphs inside one inline run — a
    /// blockquote or a loose list item. `pulldown-cmark` reports these as
    /// `Start`/`End(Paragraph)` events nested in the block, and without an arm
    /// for them both fell to `parse_inline`'s catch-all and became empty text:
    /// the paragraphs ran together on a single line.
    ParagraphBreak,
    /// Inline footnote reference `[^label]`.
    Footnote(String),
    /// Inline HTML shown verbatim in dim monospace.
    Html(String),
    /// Image reference. `slot` indexes the pane's GPU image table and is
    /// stamped by [`resolve_images`]; a slot with no image behind it falls
    /// back to the alt text. `0` until that pass runs.
    Image {
        url: String,
        alt: String,
        slot: u32,
    },
}

/// A single item in a bullet or ordered list.
#[derive(Clone)]
pub(in crate::workspace) struct ListItem {
    /// `Some(true)` = [x] checked, `Some(false)` = [ ] unchecked, `None` = plain bullet.
    pub checked: Option<bool>,
    /// The item's content in document order — its prose is a `Paragraph` like
    /// any other block, so a fence between two paragraphs stays between them.
    pub blocks: Vec<MdBlock>,
}

#[derive(Clone)]
pub(in crate::workspace) enum MdBlock {
    Heading {
        level: u8,
        spans: Vec<MdSpan>,
    },
    Paragraph(Vec<MdSpan>),
    /// Pre-highlighted code block rows (one `VisualRow` per source line).
    CodeBlock {
        #[allow(dead_code)]
        lang: Option<String>,
        rows: Vec<VisualRow>,
    },
    BulletList {
        items: Vec<ListItem>,
        /// A loose list — blank lines separate its items, so they are spaced
        /// like paragraphs instead of stacked flush.
        loose: bool,
    },
    OrderedList {
        start: u64,
        items: Vec<ListItem>,
        /// See [`MdBlock::BulletList::loose`].
        loose: bool,
    },
    Blockquote(Vec<MdSpan>),
    Rule,
    /// GFM table. `header` is the first row; `rows` are the body rows.
    /// Each cell is a list of inline spans.
    Table {
        header: Vec<Vec<MdSpan>>,
        rows: Vec<Vec<Vec<MdSpan>>>,
    },
    /// Footnote definition `[^label]: ...` collected at end of document.
    FootnoteDefinition {
        label: String,
        spans: Vec<MdSpan>,
    },
    /// Raw HTML block (dim monospace passthrough).
    HtmlBlock(String),
    /// Mermaid diagram (```mermaid fence). `slot` indexes the pane's GPU
    /// image table and is stamped by [`resolve_mermaid`]; a slot with no
    /// diagram behind it falls back to the raw source. `0` until that pass
    /// runs.
    Mermaid {
        source: String,
        slot: u32,
    },
}

/// Parse `text` into a `Vec<MdBlock>`. Code fences are syntax-highlighted
/// using `syntax_theme` (falls back to the bundled default on unknown names).
pub(in crate::workspace) fn parse_markdown(
    text: &str,
    syntax_theme: &str,
    is_light: bool,
) -> Vec<MdBlock> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_GFM);
    opts.insert(Options::ENABLE_TASKLISTS);

    let events: Vec<Event<'_>> = Parser::new_ext(text, opts).collect();
    let mut pos = 0;
    let mut blocks = Vec::new();

    while pos < events.len() {
        if let Some((block, consumed)) = parse_block(&events, pos, syntax_theme, is_light) {
            blocks.push(block);
            pos += consumed;
        } else {
            pos += 1;
        }
    }
    blocks
}

fn parse_block(
    events: &[Event<'_>],
    pos: usize,
    syntax_theme: &str,
    is_light: bool,
) -> Option<(MdBlock, usize)> {
    match &events[pos] {
        Event::Start(Tag::Heading { level, .. }) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Heading(_)))
            });
            Some((
                MdBlock::Heading {
                    level: heading_level_to_u8(*level),
                    spans,
                },
                consumed + 2, // +2: Start + End
            ))
        }

        Event::Start(Tag::Paragraph) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Paragraph))
            });
            Some((MdBlock::Paragraph(spans), consumed + 2))
        }

        Event::Start(Tag::CodeBlock(kind)) => {
            let lang = match kind {
                CodeBlockKind::Fenced(s) if !s.is_empty() => {
                    let lang = s.split_whitespace().next().unwrap_or("").to_owned();
                    if lang.is_empty() { None } else { Some(lang) }
                }
                _ => None,
            };
            let (text, consumed) = collect_text_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::CodeBlock))
            });
            if lang.as_deref() == Some("mermaid") {
                return Some((
                    MdBlock::Mermaid {
                        source: text,
                        slot: 0,
                    },
                    consumed + 2,
                ));
            }
            let mut rows = build_code_rows(&text);
            if let Some(ref l) = lang {
                highlight_raw_rows(
                    &mut rows,
                    LanguageHint::FenceToken(l),
                    syntax_theme,
                    is_light,
                );
            }
            Some((MdBlock::CodeBlock { lang, rows }, consumed + 2))
        }

        Event::Start(Tag::BlockQuote(_)) => {
            let (spans, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::BlockQuote(_)))
            });
            Some((MdBlock::Blockquote(spans), consumed + 2))
        }

        Event::Start(Tag::List(start_num)) => {
            let ordered = start_num.is_some();
            let start = start_num.unwrap_or(1);
            let mut items: Vec<ListItem> = Vec::new();
            let mut loose = false;
            let mut i = pos + 1;
            while i < events.len() {
                match &events[i] {
                    Event::Start(Tag::Item) => {
                        loose |= item_is_paragraph_wrapped(events, i + 1);
                        let (item, consumed) = parse_item(events, i + 1, syntax_theme, is_light);
                        items.push(item);
                        i += consumed + 2;
                    }
                    Event::End(TagEnd::List(_)) => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            let consumed = i - pos;
            let block = if ordered {
                MdBlock::OrderedList {
                    start,
                    items,
                    loose,
                }
            } else {
                MdBlock::BulletList { items, loose }
            };
            Some((block, consumed))
        }

        Event::Rule => Some((MdBlock::Rule, 1)),

        Event::Start(Tag::FootnoteDefinition(label)) => {
            let label = label.to_string();
            let mut spans: Vec<MdSpan> = Vec::new();
            let mut i = pos + 1;
            while i < events.len() {
                match &events[i] {
                    Event::End(TagEnd::FootnoteDefinition) => {
                        i += 1;
                        break;
                    }
                    // pulldown-cmark wraps footnote body in paragraphs; strip the wrapper.
                    Event::Start(Tag::Paragraph) => {
                        if !spans.is_empty() {
                            spans.push(MdSpan::SoftBreak);
                        }
                        let (ps, consumed) = collect_inline_until(events, i + 1, |e| {
                            matches!(e, Event::End(TagEnd::Paragraph))
                        });
                        spans.extend(ps);
                        i += consumed + 2;
                    }
                    _ => {
                        let (span, consumed) = parse_inline(events, i);
                        spans.push(span);
                        i += consumed;
                    }
                }
            }
            Some((MdBlock::FootnoteDefinition { label, spans }, i - pos))
        }

        Event::Html(s) => Some((MdBlock::HtmlBlock(s.to_string()), 1)),

        Event::Start(Tag::Table(_alignments)) => {
            let mut header: Vec<Vec<MdSpan>> = Vec::new();
            let mut rows: Vec<Vec<Vec<MdSpan>>> = Vec::new();
            let mut i = pos + 1;

            while i < events.len() {
                match &events[i] {
                    Event::Start(Tag::TableHead) => {
                        i += 1;
                        let mut head_cells: Vec<Vec<MdSpan>> = Vec::new();
                        while i < events.len() {
                            match &events[i] {
                                Event::Start(Tag::TableCell) => {
                                    let (spans, consumed) =
                                        collect_inline_until(events, i + 1, |e| {
                                            matches!(e, Event::End(TagEnd::TableCell))
                                        });
                                    head_cells.push(spans);
                                    i += consumed + 2;
                                }
                                Event::End(TagEnd::TableHead) => {
                                    i += 1;
                                    break;
                                }
                                _ => i += 1,
                            }
                        }
                        header = head_cells;
                    }
                    Event::Start(Tag::TableRow) => {
                        i += 1;
                        let mut row_cells: Vec<Vec<MdSpan>> = Vec::new();
                        while i < events.len() {
                            match &events[i] {
                                Event::Start(Tag::TableCell) => {
                                    let (spans, consumed) =
                                        collect_inline_until(events, i + 1, |e| {
                                            matches!(e, Event::End(TagEnd::TableCell))
                                        });
                                    row_cells.push(spans);
                                    i += consumed + 2;
                                }
                                Event::End(TagEnd::TableRow) => {
                                    i += 1;
                                    break;
                                }
                                _ => i += 1,
                            }
                        }
                        rows.push(row_cells);
                    }
                    Event::End(TagEnd::Table) => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            Some((MdBlock::Table { header, rows }, i - pos))
        }

        _ => None,
    }
}

/// Whether an item's content is paragraph-wrapped — how pulldown-cmark reports
/// a loose list, which it wraps and a tight one it does not. `pos` is the
/// item's first event, the index [`parse_item`] is given.
///
/// Blind spot: an item whose blocks are all non-paragraphs (a fence, a quote)
/// is unwrapped either way, so a list of only those reads tight — 0.13's
/// `Tag::List` carries no looseness flag to fall back on.
fn item_is_paragraph_wrapped(events: &[Event<'_>], pos: usize) -> bool {
    matches!(events.get(pos), Some(Event::Start(Tag::Paragraph)))
}

/// The checkbox state of a task item, or `None` for a plain one. `pos` is the
/// item's first event.
///
/// pulldown-cmark puts the marker *inside* the leading paragraph when the list
/// is loose, so looking only at the item's own first event drops the checkbox
/// from every blank-line-separated task list.
fn task_marker(events: &[Event<'_>], pos: usize) -> Option<bool> {
    let at = if item_is_paragraph_wrapped(events, pos) {
        pos + 1
    } else {
        pos
    };
    match events.get(at) {
        Some(Event::TaskListMarker(checked)) => Some(*checked),
        _ => None,
    }
}

/// Parse one list item starting at `pos` (just after `Start(Item)`).
/// Returns the item and the number of events consumed (NOT including `End(Item)`).
fn parse_item(
    events: &[Event<'_>],
    pos: usize,
    syntax_theme: &str,
    is_light: bool,
) -> (ListItem, usize) {
    let mut i = pos;
    let checked = task_marker(events, pos);

    let mut blocks: Vec<MdBlock> = Vec::new();
    // A tight item's prose arrives as bare inline events with no paragraph
    // wrapper (a loose item's comes wrapped, and `parse_block` takes it whole).
    // Bare spans gather here and close into a paragraph when a block follows.
    let mut bare: Vec<MdSpan> = Vec::new();
    fn close_bare(bare: &mut Vec<MdSpan>, blocks: &mut Vec<MdBlock>) {
        if !bare.is_empty() {
            blocks.push(MdBlock::Paragraph(std::mem::take(bare)));
        }
    }

    while i < events.len() {
        match &events[i] {
            Event::End(TagEnd::Item) => break,

            // Already read into `checked`; it is state, not prose.
            Event::TaskListMarker(_) => i += 1,

            _ => {
                if let Some((block, consumed)) = parse_block(events, i, syntax_theme, is_light) {
                    close_bare(&mut bare, &mut blocks);
                    blocks.push(block);
                    i += consumed;
                } else {
                    let (span, consumed) = parse_inline(events, i);
                    bare.push(span);
                    i += consumed;
                }
            }
        }
    }
    close_bare(&mut bare, &mut blocks);

    (ListItem { checked, blocks }, i - pos)
}

fn collect_inline_until<F>(events: &[Event<'_>], start: usize, stop: F) -> (Vec<MdSpan>, usize)
where
    F: Fn(&Event<'_>) -> bool,
{
    let mut spans = Vec::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        // A paragraph nested in this run (a blockquote, a loose list item) is a
        // boundary, not content: its `Start`/`End` carry nothing to render, and
        // the second and later ones start a new line.
        if matches!(events[i], Event::Start(Tag::Paragraph)) {
            if !spans.is_empty() {
                spans.push(MdSpan::ParagraphBreak);
            }
            i += 1;
            continue;
        }
        if matches!(events[i], Event::End(TagEnd::Paragraph)) {
            i += 1;
            continue;
        }
        // A loose task item's checkbox lands here. `parse_item` has already
        // read it into the item's `checked`.
        if matches!(events[i], Event::TaskListMarker(_)) {
            i += 1;
            continue;
        }
        let (span, consumed) = parse_inline(events, i);
        spans.push(span);
        i += consumed;
    }
    (spans, i - start)
}

fn parse_inline(events: &[Event<'_>], pos: usize) -> (MdSpan, usize) {
    match &events[pos] {
        Event::Text(s) => (MdSpan::Text(s.to_string()), 1),
        Event::Code(s) => (MdSpan::Code(s.to_string()), 1),
        Event::SoftBreak => (MdSpan::SoftBreak, 1),
        Event::HardBreak => (MdSpan::HardBreak, 1),

        Event::Start(Tag::Strong) => {
            let (inner, consumed) =
                collect_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Strong)));
            (MdSpan::Bold(inner), consumed + 2)
        }

        Event::Start(Tag::Emphasis) => {
            let (inner, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Emphasis))
            });
            (MdSpan::Italic(inner), consumed + 2)
        }

        Event::Start(Tag::Strikethrough) => {
            let (inner, consumed) = collect_inline_until(events, pos + 1, |e| {
                matches!(e, Event::End(TagEnd::Strikethrough))
            });
            (MdSpan::Strikethrough(inner), consumed + 2)
        }

        Event::Start(Tag::Link {
            dest_url, title, ..
        }) => {
            let url = if dest_url.is_empty() {
                title.to_string()
            } else {
                dest_url.to_string()
            };
            let (inner, consumed) =
                collect_inline_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Link)));
            (
                MdSpan::Link {
                    children: inner,
                    url,
                },
                consumed + 2,
            )
        }

        Event::Start(Tag::Image { dest_url, .. }) => {
            let (alt_text, consumed) =
                collect_text_until(events, pos + 1, |e| matches!(e, Event::End(TagEnd::Image)));
            (
                MdSpan::Image {
                    url: dest_url.to_string(),
                    alt: alt_text,
                    slot: 0,
                },
                consumed + 2,
            )
        }

        Event::InlineHtml(s) => (MdSpan::Html(s.to_string()), 1),
        Event::FootnoteReference(label) => (MdSpan::Footnote(label.to_string()), 1),
        Event::Html(_) => (MdSpan::Text(String::new()), 1),

        _ => (MdSpan::Text(String::new()), 1),
    }
}

fn collect_text_until<F>(events: &[Event<'_>], start: usize, stop: F) -> (String, usize)
where
    F: Fn(&Event<'_>) -> bool,
{
    let mut text = String::new();
    let mut i = start;
    while i < events.len() {
        if stop(&events[i]) {
            break;
        }
        if let Event::Text(s) = &events[i] {
            text.push_str(s);
        }
        i += 1;
    }
    (text, i - start)
}

fn heading_level_to_u8(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn build_code_rows(text: &str) -> Vec<VisualRow> {
    text.lines()
        .enumerate()
        .map(|(i, line)| VisualRow {
            content: line.to_owned(),
            kind: VisualRowKind::Plain,
            line_no_left: (i + 1).to_string(),
            line_no_right: String::new(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        })
        .collect()
}
