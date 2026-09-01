use gpui::SharedString;
use markdown::{
    ParseOptions,
    mdast::{self, Node},
};

use crate::{
    highlighter::HighlightTheme,
    text::{
        TextViewStyle,
        node::{
            self, CodeBlock, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph, Span, Table,
            TableRow, TextMark,
        },
    },
};

/// Parse Markdown into a tree of nodes.
pub(crate) fn parse(
    raw: &str,
    style: &TextViewStyle,
    cx: &mut NodeContext,
    highlight_theme: &HighlightTheme,
) -> Result<node::Node, SharedString> {
    markdown::to_mdast(&raw, &ParseOptions::gfm())
        .map(|n| ast_to_node(n, style, cx, highlight_theme))
        .map_err(|e| e.to_string().into())
}

fn parse_table_row(table: &mut Table, node: &mdast::TableRow, cx: &mut NodeContext) {
    let mut row = TableRow::default();
    node.children.iter().for_each(|c| {
        match c {
            Node::TableCell(cell) => {
                parse_table_cell(&mut row, cell, cx);
            }
            _ => {}
        };
    });
    table.children.push(row);
}

fn parse_table_cell(row: &mut node::TableRow, node: &mdast::TableCell, cx: &mut NodeContext) {
    let mut paragraph = Paragraph::default();
    node.children.iter().for_each(|c| {
        parse_paragraph(&mut paragraph, c, cx);
    });
    let table_cell = node::TableCell {
        children: paragraph,
        ..Default::default()
    };
    row.children.push(table_cell);
}

/// Apply `mark` over every run of an already-parsed paragraph, keeping the
/// marks those runs carry. Emphasis (`**`/`*`/`~~`) nests around other inline
/// nodes, so flattening its children into one run would drop their inline
/// code / link / nested-emphasis formatting.
fn mark_children(paragraph: &mut Paragraph, mark: TextMark) {
    for child in paragraph.children.iter_mut() {
        child.marks.push((0..child.text.len(), mark.clone()));
    }
}

fn parse_paragraph(paragraph: &mut Paragraph, node: &mdast::Node, cx: &mut NodeContext) -> String {
    let span = node.position().map(|pos| Span {
        start: pos.start.offset,
        end: pos.end.offset,
    });
    if let Some(span) = span {
        paragraph.set_span(span);
    }

    let mut text = String::new();

    match node {
        Node::Paragraph(val) => {
            val.children.iter().for_each(|c| {
                text.push_str(&parse_paragraph(paragraph, c, cx));
            });
        }
        Node::Text(val) => {
            text = val.value.clone();
            paragraph.push_str(&val.value)
        }
        Node::Emphasis(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in val.children.iter() {
                text.push_str(&parse_paragraph(&mut child_paragraph, &child, cx));
            }
            mark_children(&mut child_paragraph, TextMark::default().italic());
            paragraph.merge(child_paragraph);
        }
        Node::Strong(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in val.children.iter() {
                text.push_str(&parse_paragraph(&mut child_paragraph, &child, cx));
            }
            mark_children(&mut child_paragraph, TextMark::default().bold());
            paragraph.merge(child_paragraph);
        }
        Node::Delete(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in val.children.iter() {
                text.push_str(&parse_paragraph(&mut child_paragraph, &child, cx));
            }
            mark_children(&mut child_paragraph, TextMark::default().strikethrough());
            paragraph.merge(child_paragraph);
        }
        Node::InlineCode(val) => {
            text = val.value.clone();
            paragraph.push(
                InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        // A hard line break — two trailing spaces or a trailing backslash.
        // `mdast` reports it as an inline child of the paragraph, so without
        // this arm it fell to the catch-all below and the runs on either side
        // rendered glued together with no separator at all.
        Node::Break(_) => {
            text = "\n".to_owned();
            paragraph.push(InlineNode::new(&text));
        }
        Node::Link(val) => {
            let link_mark = Some(LinkMark {
                url: val.url.clone().into(),
                title: val.title.clone().map(|s| s.into()),
                ..Default::default()
            });

            let mut child_paragraph = Paragraph::default();
            for child in val.children.iter() {
                text.push_str(&parse_paragraph(&mut child_paragraph, &child, cx));
            }

            // FIXME: GPUI InteractiveText does not support inline images yet.
            // So here we push images to the paragraph directly.
            for child in child_paragraph.children.iter_mut() {
                if let Some(image) = child.image.as_mut() {
                    image.link = link_mark.clone();
                }

                child.marks.push((
                    0..child.text.len(),
                    TextMark {
                        link: link_mark.clone(),
                        ..Default::default()
                    },
                ));
            }

            paragraph.merge(child_paragraph);
        }
        Node::Image(raw) => {
            paragraph.push_image(ImageNode {
                url: raw.url.clone().into(),
                title: raw.title.clone().map(|t| t.into()),
                alt: Some(raw.alt.clone().into()),
                ..Default::default()
            });
        }
        Node::InlineMath(raw) => {
            text = raw.value.clone();
            paragraph.push(
                InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        Node::MdxTextExpression(raw) => {
            text = raw.value.clone();
            paragraph
                .push(InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default())]));
        }
        Node::Html(val) => match super::html::parse(&val.value, cx) {
            Ok(el) => {
                if el.is_break() {
                    text = "\n".to_owned();
                    paragraph.push(InlineNode::new(&text));
                } else {
                    if cfg!(debug_assertions) {
                        tracing::warn!("unsupported inline html tag: {:#?}", el);
                    }
                }
            }
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("failed parsing html: {:#?}", err);
                }

                text.push_str(&val.value);
            }
        },
        Node::FootnoteReference(foot) => {
            let prefix = format!("[{}]", foot.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));
        }
        Node::LinkReference(link) => {
            let mut child_paragraph = Paragraph::default();
            let mut child_text = String::new();
            for child in link.children.iter() {
                child_text.push_str(&parse_paragraph(&mut child_paragraph, child, cx));
            }

            let link_mark = LinkMark {
                url: "".into(),
                title: link.label.clone().map(Into::into),
                identifier: Some(link.identifier.clone().into()),
            };

            paragraph.push(InlineNode::new(&child_text).marks(vec![(
                0..child_text.len(),
                TextMark {
                    link: Some(link_mark),
                    ..Default::default()
                },
            )]));
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported inline node: {:#?}", node);
            }
        }
    }

    text
}

fn ast_to_node(
    value: mdast::Node,
    style: &TextViewStyle,
    cx: &mut NodeContext,
    highlight_theme: &HighlightTheme,
) -> node::Node {
    match value {
        Node::Root(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::Root { children }
        }
        Node::Paragraph(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, cx);
            });

            node::Node::Paragraph(paragraph)
        }
        Node::Blockquote(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::Blockquote { children }
        }
        Node::List(list) => {
            let children = list
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::List {
                ordered: list.ordered,
                spread: list.spread,
                children,
            }
        }
        Node::ListItem(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::ListItem {
                children,
                spread: val.spread,
                checked: val.checked,
            }
        }
        Node::Break(_) => node::Node::Break { html: false },
        Node::Code(raw) => node::Node::CodeBlock(CodeBlock::new(
            raw.value.into(),
            raw.lang.map(|s| s.into()),
            style,
            highlight_theme,
        )),
        Node::Heading(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, cx);
            });

            node::Node::Heading {
                level: val.depth,
                children: paragraph,
            }
        }
        Node::Math(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            None,
            style,
            highlight_theme,
        )),
        Node::Html(val) => match super::html::parse(&val.value, cx) {
            Ok(el) => el,
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("error parsing html: {:#?}", err);
                }

                node::Node::Paragraph(Paragraph::new(val.value))
            }
        },
        Node::MdxFlowExpression(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("mdx".into()),
            style,
            highlight_theme,
        )),
        Node::Yaml(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("yml".into()),
            style,
            highlight_theme,
        )),
        Node::Toml(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("toml".into()),
            style,
            highlight_theme,
        )),
        Node::MdxJsxTextElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, cx);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::MdxJsxFlowElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, cx);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::ThematicBreak(_) => node::Node::Divider,
        Node::Table(val) => {
            let mut table = Table::default();
            table.column_aligns = val
                .align
                .clone()
                .into_iter()
                .map(|align| align.into())
                .collect();
            val.children.iter().for_each(|c| {
                if let Node::TableRow(row) = c {
                    parse_table_row(&mut table, row, cx);
                }
            });

            node::Node::Table(table)
        }
        Node::FootnoteDefinition(def) => {
            let mut paragraph = Paragraph::default();
            let prefix = format!("[{}]: ", def.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));

            def.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c, cx);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::Definition(def) => {
            cx.add_ref(
                def.identifier.clone().into(),
                LinkMark {
                    url: def.url.clone().into(),
                    identifier: Some(def.identifier.clone().into()),
                    title: def.title.clone().map(Into::into),
                },
            );

            node::Node::Definition {
                identifier: def.identifier.clone().into(),
                url: def.url.clone().into(),
                title: def.title.clone().map(|s| s.into()),
            }
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported node: {:#?}", value);
            }
            node::Node::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::highlighter::HighlightTheme;
    use crate::text::TextViewStyle;
    use crate::text::node::{Node, NodeContext, Paragraph};

    fn parse_md(raw: &str) -> Node {
        let style = TextViewStyle::default();
        let mut cx = NodeContext::default();
        let theme = HighlightTheme::default_dark();
        super::parse(raw, &style, &mut cx, &theme).expect("parses")
    }

    /// The first paragraph of a single-paragraph document.
    fn first_paragraph(raw: &str) -> Paragraph {
        match parse_md(raw) {
            Node::Root { children } => match children.into_iter().next() {
                Some(Node::Paragraph(p)) => p,
                other => panic!("expected a leading paragraph, got {other:?}"),
            },
            other => panic!("expected a root, got {other:?}"),
        }
    }

    fn joined_text(p: &Paragraph) -> String {
        p.children.iter().map(|c| c.text.to_string()).collect()
    }

    /// Two trailing spaces are a hard line break; `mdast` reports it as an
    /// inline `Break` child of the paragraph. Dropping it glued the runs on
    /// either side together with no separator at all.
    #[test]
    fn a_hard_line_break_becomes_a_newline_run() {
        let p = first_paragraph("AAAA  \nBBBB");
        assert_eq!(joined_text(&p), "AAAA\nBBBB");
    }

    /// A backslash at end of line is the other hard-break spelling.
    #[test]
    fn a_backslash_line_break_becomes_a_newline_run() {
        let p = first_paragraph("AAAA\\\nBBBB");
        assert_eq!(joined_text(&p), "AAAA\nBBBB");
    }

    /// Emphasis wrapped its children's text into one flat run carrying only
    /// its own mark, so any inline code / link / nested emphasis inside it
    /// lost its formatting. The marks must compose instead.
    #[test]
    fn bold_keeps_the_marks_of_its_children() {
        let p = first_paragraph("**bold `code` here**");
        assert_eq!(joined_text(&p), "bold code here");

        let code = p
            .children
            .iter()
            .find(|c| c.text == "code")
            .expect("the inline-code run survives as its own node");
        assert!(
            code.marks.iter().any(|(_, m)| m.code),
            "inline code inside bold lost its code mark: {:?}",
            code.marks
        );
        assert!(
            p.children
                .iter()
                .all(|c| c.marks.iter().any(|(_, m)| m.bold)),
            "not every run inside the emphasis is bold: {:?}",
            p.children
        );
    }

    /// A link inside emphasis must stay a link.
    #[test]
    fn bold_keeps_a_nested_link() {
        let p = first_paragraph("**see [docs](https://example.com) now**");
        let link = p
            .children
            .iter()
            .find(|c| c.text == "docs")
            .expect("the link text survives as its own node");
        assert!(
            link.marks.iter().any(|(_, m)| m.link.is_some()),
            "link inside bold lost its link mark: {:?}",
            link.marks
        );
    }
}
