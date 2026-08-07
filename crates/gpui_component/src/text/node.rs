use std::{
    collections::HashMap,
    ops::Range,
    sync::{Arc, Mutex},
};

use gpui::{
    AnyElement, App, DefiniteLength, Div, Element, ElementId, FontStyle, FontWeight, Half,
    HighlightStyle, InteractiveElement as _, IntoElement, Length, ListState, ObjectFit,
    ParentElement, SharedString, SharedUri, StatefulInteractiveElement, Styled, StyledImage as _,
    Window, div, img, prelude::FluentBuilder as _, px, relative, rems,
};
use markdown::mdast;
use ropey::Rope;

use crate::{
    ActiveTheme as _, Icon, IconName, StyledExt, h_flex,
    highlighter::{HighlightTheme, SyntaxHighlighter},
    text::{
        CodeBlockActionsFn, CodeBlockRenderFn, LinkClickHandlerFn,
        inline::{Inline, InlineState},
    },
    tooltip::Tooltip,
    v_flex,
};

use super::{TextViewStyle, utils::list_item_prefix};

#[allow(unused)]
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LinkMark {
    pub url: SharedString,
    /// Optional identifier for footnotes.
    pub identifier: Option<SharedString>,
    pub title: Option<SharedString>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct TextMark {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<LinkMark>,
}

impl TextMark {
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    pub fn link(mut self, link: impl Into<LinkMark>) -> Self {
        self.link = Some(link.into());
        self
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<Span> for ElementId {
    fn from(value: Span) -> Self {
        ElementId::Name(format!("md-{}:{}", value.start, value.end).into())
    }
}

#[allow(unused)]
#[derive(Debug, Default, Clone)]
pub struct ImageNode {
    pub url: SharedUri,
    pub link: Option<LinkMark>,
    pub title: Option<SharedString>,
    pub alt: Option<SharedString>,
    pub width: Option<DefiniteLength>,
    pub height: Option<DefiniteLength>,
}

impl ImageNode {
    pub fn title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.alt.clone().unwrap_or_default())
            .to_string()
    }
}

impl PartialEq for ImageNode {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.link == other.link
            && self.title == other.title
            && self.alt == other.alt
            && self.width == other.width
            && self.height == other.height
    }
}

#[derive(Default, Clone, Debug)]
pub(crate) struct InlineNode {
    /// The text content.
    pub(crate) text: SharedString,
    pub(crate) image: Option<ImageNode>,
    /// The text styles, each tuple contains the range of the text and the style.
    pub(crate) marks: Vec<(Range<usize>, TextMark)>,

    state: Arc<Mutex<InlineState>>,
}

impl PartialEq for InlineNode {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.image == other.image && self.marks == other.marks
    }
}

impl InlineNode {
    pub(crate) fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            image: None,
            marks: vec![],
            state: Arc::new(Mutex::new(InlineState::default())),
        }
    }

    pub(crate) fn image(image: ImageNode) -> Self {
        let mut this = Self::new("");
        this.image = Some(image);
        this
    }

    pub(crate) fn marks(mut self, marks: Vec<(Range<usize>, TextMark)>) -> Self {
        self.marks = marks;
        self
    }
}

/// The paragraph element, contains multiple text nodes.
///
/// Unlike other Element, this is cloneable, because it is used in the Node AST.
/// We are keep the selection state inside this AST Nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paragraph {
    pub(super) span: Option<Span>,
    pub(super) children: Vec<InlineNode>,
    /// The link references in this paragraph, used for reference links.
    ///
    /// The key is the identifier, the value is the url.
    pub(super) link_refs: HashMap<SharedString, SharedString>,

    pub(crate) state: Arc<Mutex<InlineState>>,
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span
            && self.children == other.children
            && self.link_refs == other.link_refs
    }
}

impl Paragraph {
    pub(crate) fn new(text: String) -> Self {
        Self {
            span: None,
            children: vec![InlineNode::new(&text)],
            link_refs: HashMap::new(),
            state: Arc::new(Mutex::new(InlineState::default())),
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();

        for c in self.children.iter() {
            let state = c.state.lock().unwrap();
            if let Some(selection) = &state.selection {
                let part_text = state.text.clone();
                text.push_str(&part_text[selection.start..selection.end]);
            }
        }

        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let all_text = state.text.clone();
            text.push_str(&all_text[selection.start..selection.end]);
        }

        text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Table {
    pub children: Vec<TableRow>,
    pub column_aligns: Vec<ColumnumnAlign>,
}

impl Table {
    pub(crate) fn column_align(&self, index: usize) -> ColumnumnAlign {
        self.column_aligns.get(index).copied().unwrap_or_default()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub(crate) enum ColumnumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl From<mdast::AlignKind> for ColumnumnAlign {
    fn from(value: mdast::AlignKind) -> Self {
        match value {
            mdast::AlignKind::None => ColumnumnAlign::Left,
            mdast::AlignKind::Left => ColumnumnAlign::Left,
            mdast::AlignKind::Center => ColumnumnAlign::Center,
            mdast::AlignKind::Right => ColumnumnAlign::Right,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableRow {
    pub children: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableCell {
    pub children: Paragraph,
    pub width: Option<DefiniteLength>,
}

impl Paragraph {
    pub(crate) fn take(&mut self) -> Paragraph {
        std::mem::replace(
            self,
            Paragraph {
                span: None,
                children: vec![],
                link_refs: Default::default(),
                state: Arc::new(Mutex::new(InlineState::default())),
            },
        )
    }

    pub(crate) fn is_image(&self) -> bool {
        false
    }

    pub(crate) fn set_span(&mut self, span: Span) {
        self.span = Some(span);
    }

    pub(crate) fn push_str(&mut self, text: &str) {
        self.children.push(
            InlineNode::new(text.to_string()).marks(vec![(0..text.len(), TextMark::default())]),
        );
    }

    pub(crate) fn push(&mut self, text: InlineNode) {
        self.children.push(text);
    }

    pub(crate) fn push_image(&mut self, image: ImageNode) {
        self.children.push(InlineNode::image(image));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.children.is_empty()
            || self
                .children
                .iter()
                .all(|node| node.text.is_empty() && node.image.is_none())
    }

    /// Return length of children text.
    pub(crate) fn text_len(&self) -> usize {
        self.children
            .iter()
            .map(|node| node.text.len())
            .sum::<usize>()
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.children.extend(other.children);
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    lang: Option<SharedString>,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    state: Arc<Mutex<InlineState>>,
}

impl PartialEq for CodeBlock {
    fn eq(&self, other: &Self) -> bool {
        self.lang == other.lang && self.styles == other.styles
    }
}

impl CodeBlock {
    /// Get the language of the code block.
    pub fn lang(&self) -> Option<SharedString> {
        self.lang.clone()
    }

    /// Get the code content of the code block.
    pub fn code(&self) -> SharedString {
        self.state.lock().unwrap().text.clone()
    }

    pub(crate) fn new(
        code: SharedString,
        lang: Option<SharedString>,
        _: &TextViewStyle,
        highlight_theme: &HighlightTheme,
    ) -> Self {
        let mut styles = vec![];
        if let Some(lang) = &lang {
            let mut highlighter = SyntaxHighlighter::new(&lang);
            highlighter.update(None, &Rope::from_str(code.as_str()));
            styles = highlighter.styles(&(0..code.len()), highlight_theme);
        };

        let state = Arc::new(Mutex::new(InlineState::default()));
        state.lock().unwrap().set_text(code);

        Self {
            lang,
            styles,
            state,
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let part_text = state.text.clone();
            text.push_str(&part_text[selection.start..selection.end]);
        }
        text
    }

    fn render(
        &self,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        _link_click_handler: Option<&Arc<LinkClickHandlerFn>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if let Some(render) = node_cx.code_block_render.as_ref()
            && let Some(el) = render(self, window, cx)
        {
            return el;
        }

        let style = &node_cx.style;

        div()
            .when(!options.is_first, |this| this.pt(rems(0.5)))
            .when(!options.is_last, |this| this.pb(style.paragraph_gap))
            .child(
                div()
                    .id("codeblock")
                    // Hover anchor for the actions overlay's group-hover reveal
                    // (daruda's markdown code-block copy button).
                    .group("gpui-code-block")
                    .p_3()
                    .rounded(cx.theme().radius)
                    // Background-derived tint instead of the fixed
                    // `muted`/`border` surface, so the block tracks the pane
                    // background on any theme and lets the pane opacity show
                    // through. White over a dark surface, black over a light
                    // one. The fill mirrors the inline-code tint and the host's
                    // `theme::agent_chat_tint` (tool cards); the border shares
                    // the structural-line tint used by the table + rule.
                    .bg(if cx.theme().background.l < 0.5 {
                        gpui::hsla(0., 0., 1., 0.05)
                    } else {
                        gpui::hsla(0., 0., 0., 0.05)
                    })
                    .border_1()
                    .border_color(if cx.theme().background.l < 0.5 {
                        gpui::hsla(0., 0., 1., 0.28)
                    } else {
                        gpui::hsla(0., 0., 0., 0.28)
                    })
                    .font_family(cx.theme().mono_font_family.clone())
                    // daruda patch: no fixed `.text_size(mono_font_size)` — inherit the host's ambient size instead.
                    .relative()
                    .refine_style(&style.code_block)
                    .child(Inline::new(
                        "code",
                        self.state.clone(),
                        vec![],
                        self.styles.clone(),
                        None,
                    ))
                    .when_some(node_cx.code_block_actions.clone(), |this, actions| {
                        this.child(
                            // Hover-reveal the whole overlay (background chip +
                            // action) against the `gpui-code-block` group, so an
                            // idle code block shows no persistent chip.
                            div()
                                .absolute()
                                .top_2()
                                .right_2()
                                .invisible()
                                .group_hover("gpui-code-block", |s| s.visible())
                                .bg(cx.theme().muted)
                                .rounded(cx.theme().radius)
                                .child(actions(&self, window, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// A context for rendering nodes, contains link references.
#[derive(Default, Clone)]
pub(crate) struct NodeContext {
    pub(crate) link_refs: HashMap<SharedString, LinkMark>,
    pub(crate) style: TextViewStyle,
    pub(crate) code_block_actions: Option<Arc<CodeBlockActionsFn>>,
    pub(crate) code_block_render: Option<Arc<CodeBlockRenderFn>>,
}

impl NodeContext {
    pub(super) fn add_ref(&mut self, identifier: SharedString, link: LinkMark) {
        self.link_refs.insert(identifier, link);
    }
}

impl PartialEq for NodeContext {
    fn eq(&self, other: &Self) -> bool {
        self.link_refs == other.link_refs && self.style == other.style
        // Note: code_block_buttons is intentionally not compared (closures can't be compared)
    }
}

/// The AST Node of the rich text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Root {
        children: Vec<Node>,
    },
    Paragraph(Paragraph),
    Heading {
        level: u8,
        children: Paragraph,
    },
    Blockquote {
        children: Vec<Node>,
    },
    List {
        /// Only contains ListItem, others will be ignored
        children: Vec<Node>,
        ordered: bool,
    },
    ListItem {
        children: Vec<Node>,
        spread: bool,
        /// Whether the list item is checked, if None, it's not a checkbox
        checked: Option<bool>,
    },
    CodeBlock(CodeBlock),
    Table(Table),
    Break {
        html: bool,
    },
    Divider,
    /// Use for to_markdown get raw definition
    Definition {
        identifier: SharedString,
        url: SharedString,
        title: Option<SharedString>,
    },
    Unknown,
}

impl Node {
    pub(super) fn is_list_item(&self) -> bool {
        matches!(self, Self::ListItem { .. })
    }

    pub(super) fn is_break(&self) -> bool {
        matches!(self, Self::Break { .. })
    }

    /// Combine all children, omitting the empt parent nodes.
    pub(super) fn compact(self) -> Node {
        match self {
            Self::Root { mut children } if children.len() == 1 => children.remove(0).compact(),
            _ => self,
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        match self {
            Node::Root { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Paragraph(paragraph) => {
                let mut block_text = String::new();
                block_text.push_str(&paragraph.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Heading { children, .. } => {
                let mut block_text = String::new();
                block_text.push_str(&children.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::List { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::ListItem { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::Blockquote { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Table(table) => {
                let mut block_text = String::new();
                for row in table.children.iter() {
                    let mut row_texts = vec![];
                    for cell in row.children.iter() {
                        row_texts.push(cell.children.selected_text());
                    }
                    if !row_texts.is_empty() {
                        block_text.push_str(&row_texts.join(" "));
                        block_text.push('\n');
                    }
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::CodeBlock(code_block) => {
                let block_text = code_block.selected_text();
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Definition { .. } | Node::Break { .. } | Node::Divider | Node::Unknown => {}
        }

        text
    }
}

impl Paragraph {
    fn render(
        &self,
        node_cx: &NodeContext,
        link_click_handler: Option<&Arc<LinkClickHandlerFn>>,
        _window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let span = self.span;
        let children = &self.children;

        let mut child_nodes: Vec<AnyElement> = vec![];

        let mut text = String::new();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = vec![];
        let mut links: Vec<(Range<usize>, LinkMark)> = vec![];
        let mut offset = 0;

        let mut ix = 0;
        for inline_node in children {
            let text_len = inline_node.text.len();
            text.push_str(&inline_node.text);

            if let Some(image) = &inline_node.image {
                if text.len() > 0 {
                    inline_node
                        .state
                        .lock()
                        .unwrap()
                        .set_text(text.clone().into());
                    child_nodes.push(
                        Inline::new(
                            ix,
                            inline_node.state.clone(),
                            links.clone(),
                            highlights.clone(),
                            link_click_handler.cloned(),
                        )
                        .into_any_element(),
                    );
                }
                child_nodes.push(
                    img(image.url.clone())
                        .id(ix)
                        .object_fit(ObjectFit::Contain)
                        .max_w(relative(1.))
                        .when_some(image.width, |this, width| this.w(width))
                        .when_some(image.link.clone(), |this, link| {
                            let title = image.title();
                            let link_click_handler = link_click_handler.cloned();
                            this.cursor_pointer()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(title.clone()).build(window, cx)
                                })
                                .on_click(move |_, window, cx| {
                                    cx.stop_propagation();
                                    if link_click_handler.as_ref().is_some_and(|handler| {
                                        handler(link.url.as_ref(), window, cx)
                                    }) {
                                        return;
                                    }
                                    cx.open_url(&link.url);
                                })
                        })
                        .into_any_element(),
                );

                text.clear();
                links.clear();
                highlights.clear();
                offset = 0;
            } else {
                let mut node_highlights = vec![];
                for (range, style) in &inline_node.marks {
                    let inner_range = (offset + range.start)..(offset + range.end);

                    let mut highlight = HighlightStyle::default();
                    if style.bold {
                        highlight.font_weight = Some(FontWeight::BOLD);
                    }
                    if style.italic {
                        highlight.font_style = Some(FontStyle::Italic);
                    }
                    if style.strikethrough {
                        highlight.strikethrough = Some(gpui::StrikethroughStyle {
                            thickness: gpui::px(1.),
                            ..Default::default()
                        });
                    }
                    if style.code {
                        // Inline code: a background-derived translucent tint
                        // instead of the chromatic `accent` (a scarce signal
                        // color — active lane / focus / CTA — that reads as
                        // noise repeated across inline-code spans). White over a
                        // dark surface, black over a light one, picked by the
                        // theme background lightness, so the chip reads one step
                        // off the background on any theme and lets the pane
                        // opacity show through. Mirrors the host's
                        // `theme::agent_chat_tint` (tool cards).
                        let tint = if cx.theme().background.l < 0.5 {
                            gpui::hsla(0., 0., 1., 0.08)
                        } else {
                            gpui::hsla(0., 0., 0., 0.08)
                        };
                        highlight.background_color = Some(tint);
                    }

                    if let Some(mut link_mark) = style.link.clone() {
                        highlight.color = Some(cx.theme().link);
                        highlight.underline = Some(gpui::UnderlineStyle {
                            thickness: gpui::px(1.),
                            ..Default::default()
                        });

                        // convert link references, replace link
                        if let Some(identifier) = link_mark.identifier.as_ref() {
                            if let Some(mark) = node_cx.link_refs.get(identifier) {
                                link_mark = mark.clone();
                            }
                        }

                        links.push((inner_range.clone(), link_mark));
                    }

                    node_highlights.push((inner_range, highlight));
                }

                highlights = gpui::combine_highlights(highlights, node_highlights).collect();
                offset += text_len;
            }
            ix += 1;
        }

        // Add the last text node
        if text.len() > 0 {
            self.state.lock().unwrap().set_text(text.into());
            child_nodes.push(
                Inline::new(
                    ix,
                    self.state.clone(),
                    links,
                    highlights,
                    link_click_handler.cloned(),
                )
                .into_any_element(),
            );
        }

        div().id(span.unwrap_or_default()).children(child_nodes)
    }
}

#[derive(Default, Clone, Copy)]
struct NodeRenderOptions {
    in_list: bool,
    todo: bool,
    ordered: bool,
    depth: usize,
    is_first: bool,
    is_last: bool,
}

impl NodeRenderOptions {
    fn is_first(mut self, is_first: bool) -> Self {
        self.is_first = is_first;
        self
    }

    fn is_last(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

impl Paragraph {
    fn to_markdown(&self) -> String {
        let mut text = self
            .children
            .iter()
            .map(|text_node| {
                let mut text = text_node.text.to_string();
                for (range, style) in &text_node.marks {
                    if style.bold {
                        text = format!("**{}**", &text_node.text[range.clone()]);
                    }
                    if style.italic {
                        text = format!("*{}*", &text_node.text[range.clone()]);
                    }
                    if style.strikethrough {
                        text = format!("~~{}~~", &text_node.text[range.clone()]);
                    }
                    if style.code {
                        text = format!("`{}`", &text_node.text[range.clone()]);
                    }
                    if let Some(link) = &style.link {
                        text = format!("[{}]({})", &text_node.text[range.clone()], link.url);
                    }
                }

                if let Some(image) = &text_node.image {
                    let alt = image.alt.clone().unwrap_or_default();
                    let title = image
                        .title
                        .clone()
                        .map_or(String::new(), |t| format!(" \"{}\"", t));
                    text.push_str(&format!("![{}]({}{})", alt, image.url, title))
                }

                text
            })
            .collect::<Vec<_>>()
            .join("");

        text.push_str("\n\n");
        text
    }
}

impl Node {
    /// Converts the node to markdown format.
    ///
    /// This is used to generate markdown for test.
    #[allow(dead_code)]
    pub(crate) fn to_markdown(&self) -> String {
        match self {
            Node::Root { children } => children
                .iter()
                .map(|child| child.to_markdown())
                .collect::<Vec<_>>()
                .join("\n\n"),
            Node::Paragraph(paragraph) => paragraph.to_markdown(),
            Node::Heading { level, children } => {
                let hashes = "#".repeat(*level as usize);
                format!("{} {}", hashes, children.to_markdown())
            }
            Node::Blockquote { children } => {
                let content = children
                    .iter()
                    .map(|child| child.to_markdown())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                content
                    .lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Node::List { children, ordered } => children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let prefix = if *ordered {
                        format!("{}. ", i + 1)
                    } else {
                        "- ".to_string()
                    };
                    format!("{}{}", prefix, child.to_markdown())
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Node::ListItem {
                children, checked, ..
            } => {
                let checkbox = if let Some(checked) = checked {
                    if *checked { "[x] " } else { "[ ] " }
                } else {
                    ""
                };
                format!(
                    "{}{}",
                    checkbox,
                    children
                        .iter()
                        .map(|child| child.to_markdown())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            Node::CodeBlock(code_block) => {
                format!(
                    "```{}\n{}\n```",
                    code_block.lang.clone().unwrap_or_default(),
                    code_block.code()
                )
            }
            Node::Table(table) => {
                let header = table
                    .children
                    .first()
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                let alignments = table
                    .column_aligns
                    .iter()
                    .map(|align| {
                        match align {
                            ColumnumnAlign::Left => ":--",
                            ColumnumnAlign::Center => ":-:",
                            ColumnumnAlign::Right => "--:",
                        }
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                let rows = table
                    .children
                    .iter()
                    .skip(1)
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}\n{}", header, alignments, rows)
            }
            Node::Break { html } => {
                if *html {
                    "<br>".to_string()
                } else {
                    "\n".to_string()
                }
            }
            Node::Divider => "---".to_string(),
            Node::Definition {
                identifier,
                url,
                title,
            } => {
                if let Some(title) = title {
                    format!("[{}]: {} \"{}\"", identifier, url, title)
                } else {
                    format!("[{}]: {}", identifier, url)
                }
            }
            Node::Unknown => "".to_string(),
        }
        .trim()
        .to_string()
    }
}

/// How a single child of a `ListItem` should be laid out.
///
/// Split out as a pure function because the render path only produces opaque
/// `AnyElement`s (untestable), while the *decision* of which children render —
/// and how — is exactly where the "code block inside a list item vanishes" bug
/// lived: the old inline `match` handled only `Paragraph` / `List` and dropped
/// every other block on a `_ => {}` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListItemChildLayout {
    /// Leading paragraph — carries the bullet / number prefix.
    LeadParagraph,
    /// A follow-on paragraph directly after another paragraph — merged into the
    /// previous line's container so continuation text stacks.
    MergedParagraph,
    /// A follow-on paragraph after a non-paragraph block — its own continuation
    /// line with no prefix, so it neither merges into that block's container
    /// (e.g. a code block) nor sprouts a spurious bullet.
    ContinuationParagraph,
    /// A nested list — indented.
    NestedList,
    /// Any other block child (code block, blockquote, table, heading, rule).
    /// Rendered indented; MUST NOT be dropped.
    Block,
}

/// Classify each child of a `ListItem` for layout. See [`ListItemChildLayout`].
fn classify_list_item_children(children: &[Node]) -> Vec<ListItemChildLayout> {
    use ListItemChildLayout::*;
    children
        .iter()
        .enumerate()
        .map(|(ix, child)| match child {
            Node::Paragraph(_) => {
                if ix == 0 {
                    LeadParagraph
                } else {
                    match &children[ix - 1] {
                        Node::Paragraph(_) => MergedParagraph,
                        // A paragraph after a nested list keeps the historical
                        // prefixed leading-line treatment.
                        Node::List { .. } => LeadParagraph,
                        _ => ContinuationParagraph,
                    }
                }
            }
            Node::List { .. } => NestedList,
            _ => Block,
        })
        .collect()
}

impl Node {
    fn render_list_item(
        item: &Node,
        ix: usize,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        link_click_handler: Option<&Arc<LinkClickHandlerFn>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match item {
            Node::ListItem {
                children,
                spread,
                checked,
            } => v_flex()
                .id("li")
                .when(*spread, |this| this.child(div()))
                .children({
                    let mut items: Vec<Div> = Vec::with_capacity(children.len());

                    let layouts = classify_list_item_children(children);
                    for (child, layout) in children.iter().zip(layouts) {
                        let block = child.render_block(
                            NodeRenderOptions {
                                depth: options.depth + 1,
                                todo: checked.is_some(),
                                is_last: true,
                                ..options
                            },
                            node_cx,
                            link_click_handler,
                            window,
                            cx,
                        );

                        match layout {
                            // Merge continuation prose into the previous line's
                            // container so tight-list paragraphs stack.
                            ListItemChildLayout::MergedParagraph => {
                                if let Some(item_item) = items.last_mut() {
                                    item_item.extend(vec![
                                        div().overflow_hidden().child(block).into_any_element(),
                                    ]);
                                }
                            }
                            ListItemChildLayout::LeadParagraph => {
                                items.push(
                                    h_flex()
                                        .flex_1()
                                        .relative()
                                        .items_start()
                                        .content_start()
                                        .when(!options.todo && checked.is_none(), |this| {
                                            this.child(list_item_prefix(
                                                ix,
                                                options.ordered,
                                                options.depth,
                                            ))
                                        })
                                        .when_some(*checked, |this, checked| {
                                            // Todo list checkbox
                                            this.child(
                                                div()
                                                    .flex()
                                                    .mt(rems(0.4))
                                                    .mr_1p5()
                                                    .size(rems(0.875))
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(cx.theme().radius.half())
                                                    .border_1()
                                                    .border_color(cx.theme().primary)
                                                    .text_color(cx.theme().primary_foreground)
                                                    .when(checked, |this| {
                                                        this.bg(cx.theme().primary).child(
                                                            Icon::new(IconName::Check)
                                                                .size_2()
                                                                .text_xs(),
                                                        )
                                                    }),
                                            )
                                        })
                                        .child(
                                            div().flex_1().min_w_0().overflow_hidden().child(block),
                                        ),
                                );
                            }
                            // Nested lists, follow-on prose after a block, and
                            // any other block child (code block, blockquote,
                            // table, heading, rule) all render as an indented
                            // continuation line — the last of these is the fix
                            // for blocks that were previously dropped, so a
                            // fenced code block inside a list item now renders.
                            //
                            // `min_w_0` overrides the flex default `min-width:
                            // auto` so the wrapper shrinks to the list's width
                            // instead of laying its child out at intrinsic
                            // max-content width and overflowing / clipping at
                            // wider panes (the same width-dependent class the
                            // lead-paragraph and table-cell patches fix).
                            ListItemChildLayout::NestedList
                            | ListItemChildLayout::ContinuationParagraph
                            | ListItemChildLayout::Block => {
                                items.push(div().ml(rems(1.)).min_w_0().child(block));
                            }
                        }
                    }
                    items
                })
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_table(
        item: &Node,
        node_cx: &NodeContext,
        link_click_handler: Option<&Arc<LinkClickHandlerFn>>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        const DEFAULT_LENGTH: usize = 5;
        const MAX_LENGTH: usize = 150;
        let col_lens = match item {
            Node::Table(table) => {
                let mut col_lens = vec![];
                for row in table.children.iter() {
                    for (ix, cell) in row.children.iter().enumerate() {
                        if col_lens.len() <= ix {
                            col_lens.push(DEFAULT_LENGTH);
                        }

                        let len = cell.children.text_len();
                        if len > col_lens[ix] {
                            col_lens[ix] = len;
                        }
                    }
                }
                col_lens
            }
            _ => vec![],
        };

        // Background-derived table lines instead of the fixed `border` color,
        // so the outer frame, row, and cell separators track the pane
        // background on any theme. White over a dark surface, black over a
        // light one — the shared structural-line tint (same alpha as the
        // code-block border + the `<hr>` rule). The fixed hairline is
        // near-invisible against the agent-chat pane's mirrored terminal bg.
        let line_color = if cx.theme().background.l < 0.5 {
            gpui::hsla(0., 0., 1., 0.28)
        } else {
            gpui::hsla(0., 0., 0., 0.28)
        };

        match item {
            Node::Table(table) => div()
                .pb(rems(1.))
                .w_full()
                .child(
                    div()
                        .id("table")
                        .w_full()
                        .border_1()
                        .border_color(line_color)
                        .rounded(cx.theme().radius)
                        .children({
                            let mut rows = Vec::with_capacity(table.children.len());
                            for (row_ix, row) in table.children.iter().enumerate() {
                                rows.push(
                                    div()
                                        .id("row")
                                        .w_full()
                                        .when(row_ix < table.children.len() - 1, |this| {
                                            this.border_b_1()
                                        })
                                        .border_color(line_color)
                                        .flex()
                                        .flex_row()
                                        .children({
                                            let mut cells = Vec::with_capacity(row.children.len());
                                            for (ix, cell) in row.children.iter().enumerate() {
                                                let align = table.column_align(ix);
                                                let is_last_col = ix == row.children.len() - 1;
                                                let len = col_lens
                                                    .get(ix)
                                                    .copied()
                                                    .unwrap_or(MAX_LENGTH)
                                                    .min(MAX_LENGTH);

                                                cells.push(
                                                    div()
                                                        .id("cell")
                                                        .flex()
                                                        .when(
                                                            align == ColumnumnAlign::Center,
                                                            |this| this.justify_center(),
                                                        )
                                                        .when(
                                                            align == ColumnumnAlign::Right,
                                                            |this| this.justify_end(),
                                                        )
                                                        .w(Length::Definite(relative(len as f32)))
                                                        // Let the cell shrink to its proportional
                                                        // width inside the flex row instead of
                                                        // holding its min-content size; without
                                                        // this the row overflows and cells never
                                                        // reach a width the text can wrap into.
                                                        .min_w_0()
                                                        .px_2()
                                                        .py_1()
                                                        .when(!is_last_col, |this| {
                                                            this.border_r_1()
                                                                .border_color(line_color)
                                                        })
                                                        // Wrap the text (not `.truncate()`, which
                                                        // forces `white-space: nowrap` + ellipsis):
                                                        // a `min_w_0` inner div can shrink below its
                                                        // content so the text wraps to the cell
                                                        // width, while the cell's `justify_*` still
                                                        // aligns it when it's narrower than the cell.
                                                        .child(
                                                            div()
                                                                .min_w_0()
                                                                .overflow_hidden()
                                                                .child(cell.children.render(
                                                                    node_cx,
                                                                    link_click_handler,
                                                                    window,
                                                                    cx,
                                                                )),
                                                        ),
                                                )
                                            }
                                            cells
                                        }),
                                )
                            }
                            rows
                        }),
                )
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    pub(super) fn render_root(
        &self,
        list_state: Option<ListState>,
        node_cx: &NodeContext,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let options = NodeRenderOptions {
            is_last: true,
            ..Default::default()
        };

        let Some(list_state) = list_state else {
            return self
                .render_block(options, node_cx, link_click_handler.as_ref(), window, cx)
                .into_any_element();
        };

        let children = match self {
            Node::Root { children } => children,
            _ => return div().into_any_element(),
        };

        let children = children.clone();
        let node_cx = node_cx.clone();
        let link_click_handler = link_click_handler.clone();

        if list_state.item_count() != children.len() {
            list_state.reset(children.len());
        }

        gpui::list(list_state, move |ix, window, cx| {
            let is_last = ix + 1 == children.len();
            children[ix]
                .render_block(
                    options.is_last(is_last),
                    &node_cx,
                    link_click_handler.as_ref(),
                    window,
                    cx,
                )
                .into_any_element()
        })
        .size_full()
        .into_any()
    }

    fn render_block(
        &self,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        link_click_handler: Option<&Arc<LinkClickHandlerFn>>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let mb = if options.in_list || options.is_last {
            rems(0.)
        } else {
            node_cx.style.paragraph_gap
        };

        match self {
            Node::Root { children } => {
                let children_len = children.len();
                div()
                    .id("div")
                    .children(children.into_iter().enumerate().map(move |(index, node)| {
                        node.render_block(
                            options
                                .is_first(index == 0)
                                .is_last(index == children_len - 1),
                            node_cx,
                            link_click_handler,
                            window,
                            cx,
                        )
                    }))
                    .into_any_element()
            }
            Node::Paragraph(paragraph) => div()
                .id("p")
                .pb(mb)
                .line_height(rems(1.3))
                .child(paragraph.render(node_cx, link_click_handler, window, cx))
                .into_any_element(),
            Node::Heading { level, children } => {
                let (text_size, font_weight) = match level {
                    1 => (rems(2.), FontWeight::BOLD),
                    2 => (rems(1.5), FontWeight::SEMIBOLD),
                    3 => (rems(1.25), FontWeight::SEMIBOLD),
                    4 => (rems(1.125), FontWeight::SEMIBOLD),
                    5 => (rems(1.), FontWeight::SEMIBOLD),
                    6 => (rems(1.), FontWeight::MEDIUM),
                    _ => (rems(1.), FontWeight::NORMAL),
                };

                let mut text_size = text_size.to_pixels(node_cx.style.heading_base_font_size);
                if let Some(f) = node_cx.style.heading_font_size.as_ref() {
                    text_size = (f)(*level, node_cx.style.heading_base_font_size);
                }

                h_flex()
                    .id(("h", *level as usize))
                    .when(!options.is_first, |this| {
                        this.pt(node_cx.style.paragraph_gap)
                    })
                    .pb(rems(0.5))
                    .whitespace_normal()
                    .text_size(text_size)
                    .font_weight(font_weight)
                    // `min_w_0` overrides the flex-row default `min-width: auto`
                    // so the text wrapper shrinks with the pane instead of
                    // holding its intrinsic max-content width — the same
                    // width-dependent class as the list-item and table-cell
                    // wrap fixes above.
                    .child(div().flex_1().min_w_0().child(children.render(
                        node_cx,
                        link_click_handler,
                        window,
                        cx,
                    )))
                    .into_any_element()
            }
            Node::Blockquote { children } => div()
                .w_full()
                .pb(mb)
                .child(
                    div()
                        .id("blockquote")
                        .w_full()
                        .text_color(cx.theme().muted_foreground)
                        .border_l_3()
                        // The quote bar tracks the muted text color it accompanies.
                        // The upstream `secondary_active` maps to daruda's canvas
                        // (near-black), which is invisible on the agent-chat pane's
                        // mirrored terminal background.
                        .border_color(cx.theme().muted_foreground)
                        .px_4()
                        .children({
                            let children_len = children.len();
                            children.into_iter().enumerate().map(move |(index, c)| {
                                c.render_block(
                                    options
                                        .is_first(index == 0)
                                        .is_last(index == children_len - 1),
                                    node_cx,
                                    link_click_handler,
                                    window,
                                    cx,
                                )
                            })
                        }),
                )
                .into_any_element(),
            Node::List { children, ordered } => v_flex()
                .id(if *ordered { "ol" } else { "ul" })
                .pb(mb)
                .children({
                    let mut items = Vec::with_capacity(children.len());
                    let mut ix = 0;
                    for item in children.into_iter() {
                        let is_item = item.is_list_item();

                        items.push(Self::render_list_item(
                            item,
                            ix,
                            NodeRenderOptions {
                                ordered: *ordered,
                                ..options
                            },
                            node_cx,
                            link_click_handler,
                            window,
                            cx,
                        ));

                        if is_item {
                            ix += 1;
                        }
                    }
                    items
                })
                .into_any_element(),
            Node::CodeBlock(code_block) => {
                code_block.render(&options, node_cx, link_click_handler, window, cx)
            }
            Node::Table { .. } => {
                Self::render_table(self, node_cx, link_click_handler, window, cx).into_any_element()
            }
            Node::Divider => {
                // Background-derived rule instead of the fixed `border` hairline,
                // which is near-invisible on the agent-chat pane's mirrored
                // terminal background. Shares the structural-line tint (same
                // alpha as the table lines + code-block border).
                let rule_color = if cx.theme().background.l < 0.5 {
                    gpui::hsla(0., 0., 1., 0.28)
                } else {
                    gpui::hsla(0., 0., 0., 0.28)
                };
                div()
                    .pt(rems(0.5))
                    .when(!options.is_last, |this| this.pb(rems(0.5)))
                    .child(div().id("divider").bg(rule_color).h(px(1.)))
                    .into_any_element()
            }
            Node::Break { .. } => div().id("break").into_any_element(),
            Node::Unknown | Node::Definition { .. } => div().into_any_element(),
            _ => {
                if cfg!(debug_assertions) {
                    tracing::warn!("unknown implementation: {:?}", self);
                }

                div().into_any_element()
            }
        }
    }
}

// NOTE: this crate's lib-test target does not build in-repo — the vendor trim
// left two independent, pre-existing blockers (`tree.rs` uses `#[gpui::test]`
// without `gpui/test-support` in dev-deps; `dock/state.rs` `include_str!`s a
// trimmed `tests/fixtures/layout.json`). These tests are correct and run once
// the harness is repaired; until then the render fix is verified visually, the
// established practice for the sibling `TextView` render patches.
#[cfg(test)]
mod tests {
    use super::{
        ListItemChildLayout::{self, *},
        Node, Paragraph, classify_list_item_children,
    };

    fn p() -> Node {
        Node::Paragraph(Paragraph::default())
    }

    fn ul() -> Node {
        Node::List {
            children: vec![],
            ordered: false,
        }
    }

    // `Divider` stands in for any non-paragraph / non-list block (code block,
    // blockquote, table, heading, rule) — they all share the `_ => Block` arm.
    fn block() -> Node {
        Node::Divider
    }

    #[test]
    fn list_item_keeps_block_children_between_paragraphs() {
        // The exact shape of the regression: paragraph, fenced code block,
        // paragraph. The middle block must render (not be dropped on `_ => {}`),
        // and the trailing paragraph must start its own line rather than merge
        // into the block's container.
        let children = vec![p(), block(), p()];
        assert_eq!(
            classify_list_item_children(&children),
            vec![LeadParagraph, Block, ContinuationParagraph],
        );
    }

    #[test]
    fn list_item_block_child_alone_is_not_dropped() {
        let children = vec![block()];
        assert_eq!(classify_list_item_children(&children), vec![Block]);
    }

    #[test]
    fn list_item_merges_consecutive_paragraphs_and_indents_nested_list() {
        // Locks the pre-existing behaviour the fix must preserve: consecutive
        // paragraphs merge; a nested list indents; a paragraph after a nested
        // list keeps its prefixed leading-line treatment.
        let children = vec![p(), p(), ul(), p()];
        assert_eq!(
            classify_list_item_children(&children),
            vec![LeadParagraph, MergedParagraph, NestedList, LeadParagraph],
        );
    }

    #[test]
    fn empty_list_item_classifies_to_nothing() {
        let children: Vec<Node> = vec![];
        assert_eq!(
            classify_list_item_children(&children),
            Vec::<ListItemChildLayout>::new(),
        );
    }
}
