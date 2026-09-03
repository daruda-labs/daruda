//! The IR flattened back to plain text — what a copy of a preview selection
//! yields. Structural markers a reader would expect (list bullets, a table's
//! cell separators) are re-emitted; inline styling is dropped.

use super::{MdBlock, MdSpan};

/// What joins two items of a list in its plain-text form. A loose list is
/// blank-line separated in the source, so a copy that flattened it to single
/// newlines would paste back as a tight one.
fn item_separator(loose: bool) -> &'static str {
    if loose { "\n\n" } else { "\n" }
}

/// Plain-text representation of a block for clipboard copy.
pub(in crate::workspace) fn md_block_plain_text(block: &MdBlock) -> String {
    match block {
        MdBlock::Heading { level, spans } => {
            let prefix = "#".repeat(*level as usize);
            format!("{} {}", prefix, flatten_spans_to_text(spans))
        }
        MdBlock::Paragraph(spans) => flatten_spans_to_text(spans),
        MdBlock::CodeBlock { lang, rows } => {
            let fence = lang.as_deref().unwrap_or("");
            let body = rows
                .iter()
                .map(|r| r.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            format!("```{fence}\n{body}\n```")
        }
        MdBlock::Mermaid { source, .. } => format!("```mermaid\n{source}\n```"),
        MdBlock::BulletList { items, loose } => items
            .iter()
            .map(|item| {
                let prefix = match item.checked {
                    Some(true) => "- [x] ",
                    Some(false) => "- [ ] ",
                    None => "- ",
                };
                let mut text = format!("{}{}", prefix, flatten_spans_to_text(&item.spans));
                for child in &item.children {
                    for line in md_block_plain_text(child).lines() {
                        text.push('\n');
                        text.push_str("  ");
                        text.push_str(line);
                    }
                }
                text
            })
            .collect::<Vec<_>>()
            .join(item_separator(*loose)),
        MdBlock::OrderedList {
            start,
            items,
            loose,
        } => items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let mut text = format!(
                    "{}. {}",
                    start + i as u64,
                    flatten_spans_to_text(&item.spans)
                );
                for child in &item.children {
                    for line in md_block_plain_text(child).lines() {
                        text.push('\n');
                        text.push_str("  ");
                        text.push_str(line);
                    }
                }
                text
            })
            .collect::<Vec<_>>()
            .join(item_separator(*loose)),
        MdBlock::Blockquote(spans) => format!("> {}", flatten_spans_to_text(spans)),
        MdBlock::Rule => "---".to_owned(),
        MdBlock::FootnoteDefinition { label, spans } => {
            format!("[^{}]: {}", label, flatten_spans_to_text(spans))
        }
        MdBlock::HtmlBlock(html) => html.clone(),
        MdBlock::Table { header, rows } => {
            let header_line = header
                .iter()
                .map(|c| flatten_spans_to_text(c))
                .collect::<Vec<_>>()
                .join(" | ");
            let sep_line = header.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");
            let body_lines: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|c| flatten_spans_to_text(c))
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .collect();
            let mut out = format!("{header_line}\n{sep_line}");
            for line in body_lines {
                out.push('\n');
                out.push_str(&line);
            }
            out
        }
    }
}

fn flatten_spans_to_text(spans: &[MdSpan]) -> String {
    let mut out = String::new();
    for s in spans {
        match s {
            MdSpan::Text(t) | MdSpan::Code(t) => out.push_str(t),
            MdSpan::Bold(inner)
            | MdSpan::Italic(inner)
            | MdSpan::Link {
                children: inner, ..
            }
            | MdSpan::Strikethrough(inner) => {
                out.push_str(&flatten_spans_to_text(inner));
            }
            MdSpan::SoftBreak | MdSpan::HardBreak | MdSpan::ParagraphBreak => out.push(' '),
            MdSpan::Footnote(label) => out.push_str(&format!("[^{label}]")),
            MdSpan::Html(s) => out.push_str(s),
            MdSpan::Image { url, alt, .. } => {
                let label = if alt.is_empty() { url } else { alt };
                out.push_str(&format!("[{label}]"));
            }
        }
    }
    out
}
