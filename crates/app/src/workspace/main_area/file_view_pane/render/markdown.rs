//! Markdown preview renderer — block + span tree → GPUI elements,
//! plus block-level click/drag selection plumbed through `Workspace`.

use std::rc::Rc;

use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use gpui::{
    AnyElement, App, Context, FontStyle, FontWeight, Global, HighlightStyle, ImageSource,
    InteractiveText, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, RenderImage,
    StrikethroughStyle, StyledText, Window, div, img, prelude::*, px,
};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::markdown_viewer::{
    ListItem, MdBlock, MdSpan, lone_image,
};
use crate::workspace::main_area::file_view_pane::render::prose::{
    CompiledText, InlineStyle, ProsePart, compile_prose,
};
use crate::workspace::main_area::file_view_pane::visual::RasterImage;
use crate::workspace::main_area::file_view_pane::{CharSelection, VisualRow};

/// The colours this preview paints with, resolved against the surface it is
/// painted on.
///
/// The preview sits on `file_viewer_pane_bg`, which mirrors the *terminal*
/// palette (DESIGN.md §AgentChatPane — the file viewer shares that surface so a
/// file opened from a transcript does not land on an unrelated one). Every
/// colour therefore comes from [`PaneSurfaceTokens`], never from the UI theme:
/// `ui_preset` and `terminal_preset` are independent, so a light UI theme over
/// a dark terminal previously painted `#2d2d2d` prose onto a near-black pane
/// while the table and code fills stayed light — the body vanished and the
/// blocks inverted.
///
/// The semantic `SUCCESS` hue for a ticked task box stays outside this surface
/// palette. Inline code uses the pane's own fill and foreground roles.
struct MdColors {
    /// Body prose, and headings — a terminal-mirrored surface has no tone
    /// above its own foreground, so heading rank is carried by size and weight
    /// (the same way the agent-chat markdown renders them).
    text: gpui::Hsla,
    /// Blockquote prose, its bar, and list bullets.
    muted: gpui::Hsla,
    /// Footnotes, HTML passthrough, strikethrough.
    subtle: gpui::Hsla,
    link: gpui::Hsla,
    /// The weaker of the two fills — code block, table body rows (the UI
    /// theme's `BG_PANEL` rung).
    fill: gpui::Hsla,
    /// The stronger fill, one step above [`Self::fill`] — table header (the
    /// `BG_RAISED` rung).
    raised: gpui::Hsla,
    /// Code-block border, table lines, the `<hr>` rule.
    line: gpui::Hsla,
}

type OpenUrl = Rc<dyn Fn(&str, &mut Window, &mut App)>;

#[derive(Clone, Copy)]
struct TableCellPosition {
    is_header: bool,
    is_first: bool,
    is_last: bool,
}

#[derive(Clone, Copy)]
struct PendingBlockSelection {
    block_idx: usize,
    shift: bool,
}

/// Input state that must survive the repaint `InteractiveText` requests on
/// mouse-down. A link consumes the queued block selection on mouse-up.
#[derive(Default)]
struct MarkdownPointerState {
    pressed_button: Option<MouseButton>,
    pending_block_selection: Option<PendingBlockSelection>,
}

impl Global for MarkdownPointerState {}

fn record_markdown_mouse_button(button: MouseButton, cx: &mut App) {
    cx.default_global::<MarkdownPointerState>().pressed_button = Some(button);
}

fn take_markdown_mouse_button(cx: &mut App) -> Option<MouseButton> {
    cx.default_global::<MarkdownPointerState>()
        .pressed_button
        .take()
}

fn queue_block_selection(block_idx: usize, shift: bool, cx: &mut App) {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection = Some(PendingBlockSelection { block_idx, shift });
}

fn take_pending_block_selection(cx: &mut App) -> Option<PendingBlockSelection> {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection
        .take()
}

fn take_pending_block_selection_for_drag(
    active_block_idx: usize,
    cx: &mut App,
) -> Option<PendingBlockSelection> {
    let pointer = cx.default_global::<MarkdownPointerState>();
    if pointer
        .pending_block_selection
        .is_some_and(|pending| pending.block_idx != active_block_idx)
    {
        pointer.pending_block_selection.take()
    } else {
        None
    }
}

fn cancel_pending_block_selection(cx: &mut App) {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection = None;
}

impl MdColors {
    fn for_pane(cx: &App) -> Self {
        let tokens = PaneSurfaceTokens::file_viewer(cx);
        Self {
            text: tokens.foreground,
            muted: tokens.foreground_muted,
            subtle: tokens.foreground_subtle,
            link: theme::file_viewer_pane_link_color(cx),
            fill: tokens.tint,
            raised: tokens.active_tint,
            line: tokens.border_tint,
        }
    }
}

/// The monospace card both a fenced code block and an unrendered mermaid fence
/// sit in.
fn code_surface(t: &MdColors) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .bg(t.fill)
        .border_1()
        .border_color(t.line)
        .rounded(px(theme::MD_CODE_BLOCK_RADIUS))
        .px(px(theme::MD_CODE_BLOCK_PAD_X))
        .py(px(theme::MD_CODE_BLOCK_PAD_Y))
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .font(gpui::font("monospace"))
        .text_color(t.text)
}

/// A code block's whole text plus the byte ranges its syntax colours cover.
///
/// One `StyledText` per block rather than a div per row and per span: gpui
/// shapes and wraps multi-line text itself, so the element count stops tracking
/// the block's line count. This preview builds the whole document every time it
/// renders and the File pane is not `.cached()`, so that count is paid on every
/// repaint. Same shape zed's markdown renderer uses (`flush_text`,
/// `crates/markdown/src/markdown.rs`), which flushes at container boundaries,
/// never per line.
///
/// A row with no spans contributes its plain `content` and no highlight, so it
/// takes the surface's own text colour.
fn code_block_text(rows: &[VisualRow]) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        if ix > 0 {
            text.push('\n');
        }
        if row.spans.is_empty() {
            text.push_str(&row.content);
            continue;
        }
        for span in row.spans.iter().filter(|s| !s.text.is_empty()) {
            let start = text.len();
            text.push_str(&span.text);
            if let Some(color) = span.color {
                highlights.push((
                    start..text.len(),
                    HighlightStyle {
                        color: Some(color),
                        ..Default::default()
                    },
                ));
            }
        }
    }
    (text, highlights)
}

/// Top-level Markdown body: a padded column of selectable blocks.
pub(super) fn render_md_body(
    blocks: &[MdBlock],
    char_selection: Option<&CharSelection>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = MdColors::for_pane(cx);
    let open_url: OpenUrl = Rc::new(|url, _window, cx| {
        if is_allowed_markdown_url(url) {
            cx.open_url(url);
        }
    });
    render_md_body_layout(blocks, char_selection, &t, open_url, |block, block_idx| {
        block_with_selection(block, block_idx, cx)
    })
}

/// Layout half of [`render_md_body`]. Selection listeners are supplied by the
/// host so layout probes can exercise this exact path without constructing a
/// `Workspace` merely to obtain its `Context`.
fn render_md_body_layout(
    blocks: &[MdBlock],
    char_selection: Option<&CharSelection>,
    t: &MdColors,
    on_open_url: OpenUrl,
    mut decorate_block: impl FnMut(gpui::Div, usize) -> gpui::Div,
) -> gpui::Div {
    let body_text = t.text;
    let block_sel_bg = theme::SELECTION_BG;
    let mut col = div()
        .flex()
        .flex_col()
        .px(px(theme::MD_BODY_PAD_X))
        .py(px(theme::MD_BODY_PAD_Y))
        .gap(px(theme::MD_BLOCK_GAP))
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .text_color(body_text);

    for (i, block) in blocks.iter().enumerate() {
        let is_sel = is_block_selected(char_selection, i);
        let mut next_part_idx = 0;
        let block_el = div()
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .when(is_sel, |d| d.bg(block_sel_bg))
            .child(render_md_block(
                block,
                t,
                i,
                &mut next_part_idx,
                &on_open_url,
            ));
        col = col.child(decorate_block(block_el, i));
    }
    col
}

fn is_allowed_markdown_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

/// Vertical gap between the items of a list. A loose list is made of
/// paragraphs, so its items take the gap this renderer puts between blocks —
/// the same step [`MdSpan::ParagraphBreak`] already opens *inside* one item.
fn list_item_gap(loose: bool) -> f32 {
    if loose {
        theme::MD_BLOCK_GAP
    } else {
        theme::MD_LIST_ITEM_GAP
    }
}

/// A run of spans as a column of wrapping rows, one per paragraph.
///
/// The split is what makes a multi-paragraph item work. A break rendered as a
/// full-width flex item *inside* the wrapping row left the text before it laid
/// out at zero width, wrapping a character at a time over the item below.
/// `flex_1().w_0()` is the shape zed gives prose beside a bullet cell
/// (`push_markdown_list_item`); measured here it ties with `w_full().min_w_0()`.
fn render_md_prose(
    spans: &[MdSpan],
    t: &MdColors,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    let mut prose = div()
        .flex()
        .flex_col()
        .flex_1()
        .w_0()
        .gap(px(theme::MD_BLOCK_GAP));
    for run in spans.split(|span| matches!(span, MdSpan::ParagraphBreak)) {
        prose = prose.child(render_prose_run(
            run,
            t,
            block_idx,
            next_part_idx,
            on_open_url,
        ));
    }
    prose
}

fn render_prose_run(
    spans: &[MdSpan],
    t: &MdColors,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .w_full()
        .min_w_0()
        .items_center()
        .whitespace_normal();
    let parts = compile_prose(spans);
    let text_fills_row = matches!(parts.as_slice(), [ProsePart::Text(_)]);
    for part in parts {
        let part_idx = *next_part_idx;
        *next_part_idx += 1;
        row = match part {
            ProsePart::Text(text) => row.child(render_compiled_text(
                text,
                t,
                block_idx,
                part_idx,
                text_fills_row,
                on_open_url,
            )),
            ProsePart::Image(image) => row.child(render_inline_md_image(
                image.raster,
                image.alt,
                image.link_url,
                t,
                block_idx,
                part_idx,
                on_open_url,
            )),
        };
    }
    row
}

fn render_compiled_text(
    compiled: CompiledText,
    t: &MdColors,
    block_idx: usize,
    part_idx: usize,
    fill_width: bool,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let CompiledText {
        text,
        style_runs,
        link_ranges,
        link_urls,
    } = compiled;
    let highlights = style_runs
        .iter()
        .map(|run| (run.range.clone(), highlight_for(run.style, t)))
        .collect::<Vec<_>>();
    let monospace_ranges = style_runs
        .iter()
        .filter(|run| run.style.code || run.style.html)
        .map(|run| (run.range.clone(), gpui::SharedString::from("monospace")))
        .collect::<Vec<_>>();
    let styled = StyledText::new(text)
        .with_highlights(highlights)
        .with_font_family_overrides(monospace_ranges);

    // Only a URL this host will actually open becomes a clickable range. A
    // range that fails the allowlist later would still take the pointer cursor
    // and swallow the block selection the click would otherwise have started,
    // leaving a link that does nothing at all.
    let (link_ranges, link_urls): (Vec<_>, Vec<_>) = link_ranges
        .into_iter()
        .zip(link_urls)
        .filter(|(_, url)| is_allowed_markdown_url(url))
        .unzip();

    let text: AnyElement = if link_ranges.is_empty() {
        styled.into_any_element()
    } else {
        let on_open_url = on_open_url.clone();
        let interactive =
            InteractiveText::new(format!("markdown-prose-{block_idx}-{part_idx}"), styled)
                .on_click(link_ranges, move |range_idx, window, cx| {
                    let is_primary = take_markdown_mouse_button(cx) == Some(MouseButton::Left);
                    if !is_primary {
                        return;
                    }
                    cancel_pending_block_selection(cx);
                    if let Some(url) = link_urls.get(range_idx) {
                        on_open_url(url, window, cx);
                    }
                });
        div()
            .capture_any_mouse_down(|event, _, cx| {
                record_markdown_mouse_button(event.button, cx);
            })
            .child(interactive)
            .into_any_element()
    };

    div()
        .when(fill_width, |d| d.flex_1().w_0())
        .min_w_0()
        .whitespace_normal()
        .when(cfg!(test), |d| d.debug_selector(|| "md-plain".into()))
        .child(text)
        .into_any_element()
}

fn render_inline_md_image(
    raster: Option<&RasterImage>,
    alt: &str,
    link_url: Option<&str>,
    t: &MdColors,
    block_idx: usize,
    part_idx: usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let image = render_md_image(raster, alt, ImageLayout::Inline, t);
    let Some(url) = link_url else {
        return image;
    };

    let url = url.to_owned();
    let on_open_url = on_open_url.clone();
    div()
        .id(format!("markdown-image-link-{block_idx}-{part_idx}"))
        .cursor_pointer()
        .when(cfg!(test), |d| {
            d.debug_selector(|| "markdown-linked-image".into())
        })
        .on_click(move |_, window, cx| {
            cancel_pending_block_selection(cx);
            on_open_url(&url, window, cx);
        })
        .child(image)
        .into_any_element()
}

fn highlight_for(style: InlineStyle, t: &MdColors) -> HighlightStyle {
    // Link wins over code: `[`CLAUDE.md`](…)` is the common form in this repo's
    // own docs, and without the link colour it is a click with no affordance —
    // `underline` is deliberately off (DESIGN.md), so colour is the only cue.
    let color = if style.link {
        Some(t.link)
    } else if style.code {
        Some(t.text)
    } else if style.strikethrough || style.footnote || style.html {
        Some(t.subtle)
    } else {
        None
    };
    HighlightStyle {
        color,
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        background_color: style.code.then_some(t.fill),
        underline: None,
        strikethrough: style.strikethrough.then_some(StrikethroughStyle {
            thickness: px(theme::MD_STRIKETHROUGH_H),
            color: None,
        }),
        fade_out: None,
    }
}

/// What fills a list item's marker cell.
enum ListMarker {
    /// A checkbox when the item is a task, a dot otherwise.
    Bullet,
    /// Counting from the list's start number.
    Ordered(u64),
}

/// One list. Both kinds share the row, its wrap and the nested-block indent —
/// only the marker cell differs, so that is all `marker` decides.
fn render_list(
    items: &[ListItem],
    loose: bool,
    marker: ListMarker,
    t: &MdColors,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let mut list = div().flex().flex_col().gap(px(list_item_gap(loose)));
    for (i, item) in items.iter().enumerate() {
        let cell = match marker {
            ListMarker::Bullet => {
                let (glyph, color) = match item.checked {
                    Some(true) => ("☑", theme::SUCCESS),
                    Some(false) => ("☐", t.muted),
                    None => ("•", t.muted),
                };
                div()
                    .flex_none()
                    .text_color(color)
                    .when(cfg!(test) && item.checked.is_some(), |d| {
                        d.debug_selector(|| "markdown-task-checkbox".into())
                    })
                    .child(glyph)
            }
            ListMarker::Ordered(start) => div()
                .flex_none()
                .min_w(px(theme::MD_LIST_INDENT))
                .text_color(t.muted)
                .child(format!("{}.", start + i as u64)),
        };

        let mut item_col = div().flex().flex_col().child(
            div()
                .flex()
                .flex_row()
                .gap(px(theme::MD_LIST_ROW_GAP))
                .child(cell)
                .child(render_md_prose(
                    &item.spans,
                    t,
                    block_idx,
                    next_part_idx,
                    on_open_url,
                )),
        );
        for child in &item.children {
            item_col = item_col.child(div().pl(px(theme::MD_LIST_INDENT)).child(render_md_block(
                child,
                t,
                block_idx,
                next_part_idx,
                on_open_url,
            )));
        }
        list = list.child(item_col);
    }
    list.into_any_element()
}

fn render_md_block(
    block: &MdBlock,
    t: &MdColors,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    match block {
        MdBlock::Heading { level, spans } => {
            let (size, color, mt) = match level {
                1 => (theme::MD_H1_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                2 => (theme::MD_H2_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                3 => (theme::MD_H3_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                _ => (theme::MD_H4_FONT_SIZE, t.text, 0.0),
            };
            div()
                .w_full()
                .min_w_0()
                .mt(px(mt))
                .text_size(px(size))
                .text_color(color)
                .font_weight(gpui::FontWeight::BOLD)
                .child(render_prose_run(
                    spans,
                    t,
                    block_idx,
                    next_part_idx,
                    on_open_url,
                ))
                .into_any_element()
        }

        MdBlock::Paragraph(spans) => {
            // A paragraph that is just an image renders it block-style (large);
            // an image mixed with text renders inline, sized to the line.
            if let Some((alt, raster)) = lone_image(spans) {
                render_md_image(raster, alt, ImageLayout::Block, t)
            } else {
                div()
                    .w_full()
                    .min_w_0()
                    .child(render_prose_run(
                        spans,
                        t,
                        block_idx,
                        next_part_idx,
                        on_open_url,
                    ))
                    .into_any_element()
            }
        }

        MdBlock::CodeBlock { rows, .. } => {
            let (text, highlights) = code_block_text(rows);
            code_surface(t)
                .child(StyledText::new(text).with_highlights(highlights))
                .into_any_element()
        }

        MdBlock::Mermaid { source, raster } => match raster {
            Some(raster) => render_md_image(Some(raster), "", ImageLayout::Diagram, t),
            // Rendering failed/pending: fall back to the raw source, styled
            // like a code block.
            None => code_surface(t)
                .child(StyledText::new(source.clone()))
                .into_any_element(),
        },

        MdBlock::BulletList { items, loose } => render_list(
            items,
            *loose,
            ListMarker::Bullet,
            t,
            block_idx,
            next_part_idx,
            on_open_url,
        ),

        MdBlock::OrderedList {
            start,
            items,
            loose,
        } => render_list(
            items,
            *loose,
            ListMarker::Ordered(*start),
            t,
            block_idx,
            next_part_idx,
            on_open_url,
        ),

        MdBlock::Blockquote(spans) => div()
            .flex()
            .flex_row()
            .gap(px(theme::MD_BLOCKQUOTE_PAD_L))
            .child(
                div()
                    .flex_none()
                    .w(px(theme::MD_BLOCKQUOTE_BORDER_W))
                    .bg(t.line)
                    .rounded(px(theme::MD_BLOCKQUOTE_BORDER_W / 2.0)),
            )
            .child(
                render_md_prose(spans, t, block_idx, next_part_idx, on_open_url)
                    .italic()
                    .text_color(t.muted),
            )
            .into_any_element(),

        MdBlock::Rule => div()
            .w_full()
            .h(px(theme::MD_RULE_H))
            .bg(t.line)
            .my(px(theme::MD_BLOCK_MARGIN_Y))
            .into_any_element(),

        MdBlock::FootnoteDefinition { label, spans } => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .w_full()
            .min_w_0()
            .gap(px(theme::MD_LIST_ROW_GAP))
            .text_color(t.subtle)
            .child(div().flex_none().child(format!("[^{label}]:")))
            .child(render_md_prose(
                spans,
                t,
                block_idx,
                next_part_idx,
                on_open_url,
            ))
            .into_any_element(),

        MdBlock::HtmlBlock(html) => div()
            .font(gpui::font("monospace"))
            .text_color(t.subtle)
            .child(html.clone())
            .into_any_element(),

        MdBlock::Table { header, rows } => {
            let table_border = t.line;
            let col_count = header.len();
            let row_count = rows.len();
            let mut header_row = div()
                .flex()
                .flex_row()
                .w_full()
                .border_b_1()
                .border_color(table_border)
                .bg(t.raised);
            for (cell_idx, cell) in header.iter().enumerate() {
                header_row = header_row.child(render_table_cell(
                    cell,
                    TableCellPosition {
                        is_header: true,
                        is_first: cell_idx == 0,
                        is_last: cell_idx + 1 == col_count,
                    },
                    t,
                    block_idx,
                    next_part_idx,
                    on_open_url,
                ));
            }

            let mut body_rows = Vec::with_capacity(rows.len());
            for (row_idx, row) in rows.iter().enumerate() {
                let is_last_row = row_idx + 1 == row_count;
                let mut row_div = div()
                    .flex()
                    .flex_row()
                    .w_full()
                    .when(!is_last_row, |d| d.border_b_1().border_color(table_border))
                    .bg(t.fill);
                for (cell_idx, cell) in row.iter().enumerate() {
                    row_div = row_div.child(render_table_cell(
                        cell,
                        TableCellPosition {
                            is_header: false,
                            is_first: false,
                            is_last: cell_idx + 1 == row.len(),
                        },
                        t,
                        block_idx,
                        next_part_idx,
                        on_open_url,
                    ));
                }
                body_rows.push(row_div.into_any_element());
            }

            div()
                .w_full()
                .border_1()
                .border_color(table_border)
                .rounded(px(theme::MD_BLOCK_RADIUS))
                .overflow_hidden()
                .my(px(theme::MD_BLOCK_MARGIN_Y))
                .child(header_row)
                .children(body_rows)
                .into_any_element()
        }
    }
}

fn render_table_cell(
    cell: &[MdSpan],
    position: TableCellPosition,
    t: &MdColors,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    div()
        .flex_1()
        .min_w(px(theme::MD_TABLE_CELL_MIN_W))
        .overflow_hidden()
        .px(px(theme::MD_TABLE_CELL_PAD_X))
        .py(px(theme::MD_TABLE_CELL_PAD_Y))
        .when(!position.is_last, |d| d.border_r_1().border_color(t.line))
        .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
        .text_color(t.text)
        .when(position.is_header, |d| {
            d.font_weight(gpui::FontWeight::BOLD)
        })
        .when(cfg!(test) && position.is_header && position.is_first, |d| {
            d.debug_selector(|| "markdown-table-first-header-cell".into())
        })
        .child(render_prose_run(
            cell,
            t,
            block_idx,
            next_part_idx,
            on_open_url,
        ))
}

/// How a markdown image is sized.
#[derive(Clone, Copy)]
enum ImageLayout {
    /// Standalone decorative image (photo/screenshot): fits the pane width,
    /// height capped so one large embedded photo can't dominate the document.
    Block,
    /// Mermaid diagram: fits the pane width, height uncapped. A diagram is
    /// structured information to be read, not decorative — capping its
    /// height shrinks a tall flowchart (e.g. many vertical steps) until its
    /// text is unreadable. The containing document already scrolls, so a
    /// tall diagram just takes more scroll room instead of being squeezed.
    Diagram,
    /// Image embedded in a text run: sized to the line so it flows with text.
    Inline,
}

/// Render a resolved image bitmap, or fall back to `[alt]` text when the image
/// was not loaded (remote/missing/decode-failed). `object_fit` defaults to
/// `Contain`, preserving aspect ratio; gpui derives the unset dimension from it.
fn render_md_image(
    raster: Option<&RasterImage>,
    alt: &str,
    layout: ImageLayout,
    t: &MdColors,
) -> AnyElement {
    let Some(raster) = raster else {
        return div()
            .text_color(t.subtle)
            .child(format!("[{alt}]"))
            .into_any_element();
    };
    match layout {
        // Block-sized, height-capped: decorative images only.
        ImageLayout::Block => raster_block_image(raster)
            .unwrap_or_else(|| div().child(format!("[{alt}]")).into_any_element()),
        // Width-capped only: shared with the agent-chat mermaid renderer.
        ImageLayout::Diagram => raster_diagram_image(raster)
            .unwrap_or_else(|| div().child(format!("[{alt}]")).into_any_element()),
        // Sized to the text line; gpui derives width from the aspect ratio.
        ImageLayout::Inline => {
            let mut bgra = raster.rgba.clone();
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            match image::RgbaImage::from_raw(raster.width, raster.height, bgra)
                .map(|buf| std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
            {
                Some(render_image) => img(ImageSource::Render(render_image))
                    .h(px(theme::MD_INLINE_IMAGE_HEIGHT))
                    .into_any_element(),
                None => div().child(format!("[{alt}]")).into_any_element(),
            }
        }
    }
}

/// Rasterized image converted once so GPUI can reuse the same texture id.
/// Agent chat caches this for image-heavy markdown; rebuilding per render would
/// force repeated GPU uploads.
#[derive(Clone)]
pub(in crate::workspace) struct CachedImage {
    image: std::sync::Arc<RenderImage>,
    logical_w: f32,
}

impl CachedImage {
    /// Convert a raster once, swapping RGBA to GPUI's BGRA byte order.
    pub(in crate::workspace) fn from_raster(raster: &RasterImage) -> Option<Self> {
        let mut bgra = raster.rgba.clone();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let buffer = image::RgbaImage::from_raw(raster.width, raster.height, bgra)?;
        let image = std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        let (logical_w, _) = raster.logical_size();
        Some(Self { image, logical_w })
    }

    /// Block-layout element at logical size, capped to the container and max
    /// image height while preserving the cached texture id. For decorative
    /// images only — see [`Self::block_diagram`] for diagrams.
    pub(in crate::workspace) fn block(&self) -> AnyElement {
        img(ImageSource::Render(self.image.clone()))
            .w(px(self.logical_w))
            .max_w_full()
            .max_h(px(theme::MD_IMAGE_MAX_HEIGHT))
            .into_any_element()
    }

    /// Diagram-layout element: capped to the container width only, height
    /// uncapped. A diagram is read, not decorative — the containing document
    /// already scrolls, so a tall one just takes more scroll room instead of
    /// being squeezed to `MD_IMAGE_MAX_HEIGHT` like a decorative image.
    pub(in crate::workspace) fn block_diagram(&self) -> AnyElement {
        img(ImageSource::Render(self.image.clone()))
            .w(px(self.logical_w))
            .max_w_full()
            .into_any_element()
    }

    /// Logical (point) width the diagram lays out at — the lightbox uses it to
    /// size the dialog to the content.
    pub(in crate::workspace) fn logical_width(&self) -> f32 {
        self.logical_w
    }

    /// Uncapped element at natural logical size — for the lightbox body, where
    /// the surrounding scroll container (not the image) handles overflow.
    /// `block_diagram`'s `max_w_full` would re-shrink the image to the modal.
    pub(in crate::workspace) fn natural(&self) -> AnyElement {
        img(ImageSource::Render(self.image.clone()))
            .flex_none()
            .w(px(self.logical_w))
            .into_any_element()
    }
}

/// Block-layout element for a raster, converting fresh for Markdown preview.
/// Agent chat caches [`CachedImage`] instead. Decorative images only — see
/// [`raster_diagram_image`] for diagrams.
pub(in crate::workspace) fn raster_block_image(raster: &RasterImage) -> Option<AnyElement> {
    Some(CachedImage::from_raster(raster)?.block())
}

/// Diagram-layout element for a raster, converting fresh for Markdown
/// preview. Agent chat caches [`CachedImage`] instead.
pub(in crate::workspace) fn raster_diagram_image(raster: &RasterImage) -> Option<AnyElement> {
    Some(CachedImage::from_raster(raster)?.block_diagram())
}

/// Returns true when `block_idx` falls within the char-selection row range.
/// Used only by the Markdown preview block-level selection.
fn is_block_selected(char_selection: Option<&CharSelection>, block_idx: usize) -> bool {
    let Some(sel) = char_selection else {
        return false;
    };
    let (start, end) = sel.ordered();
    block_idx >= start.row && block_idx <= end.row
}

/// Attach block-level click/drag selection handlers to a Markdown block div.
/// Selection waits until mouse-up or until the pointer enters another block,
/// giving a link in the original block a chance to consume an ordinary click.
fn block_with_selection(
    block_div: gpui::Div,
    block_idx: usize,
    cx: &mut Context<Workspace>,
) -> gpui::Div {
    let down_handler = cx.listener(move |_this, ev: &MouseDownEvent, _window, cx| {
        queue_block_selection(block_idx, ev.modifiers.shift, cx);
    });
    let move_handler = cx.listener(move |this, ev: &MouseMoveEvent, _window, cx| {
        let left_pressed = ev.pressed_button == Some(MouseButton::Left);
        let pending = if left_pressed {
            take_pending_block_selection_for_drag(block_idx, cx)
        } else {
            cancel_pending_block_selection(cx);
            None
        };
        if let Some(fv) = this.focused_file_view_mut() {
            let mut changed = false;
            if let Some(pending) = pending {
                fv.handle_block_mouse_down(pending.block_idx, pending.shift);
                changed = true;
            }
            changed |= fv.handle_block_mouse_move(block_idx, left_pressed);
            if changed {
                cx.notify();
            }
        }
    });
    let up_handler = cx.listener(move |this, _ev, _window, cx| {
        let pending = take_pending_block_selection(cx);
        if let Some(fv) = this.focused_file_view_mut() {
            let mut changed = false;
            if let Some(pending) = pending {
                fv.handle_block_mouse_down(pending.block_idx, pending.shift);
                if !pending.shift && pending.block_idx != block_idx {
                    fv.handle_block_mouse_move(block_idx, true);
                }
                changed = true;
            }
            changed |= fv.end_selection_drag();
            if changed {
                cx.notify();
            }
        }
    });
    block_div
        .cursor_default()
        .on_mouse_down(MouseButton::Left, down_handler)
        .on_mouse_up(MouseButton::Left, up_handler)
        .on_mouse_move(move_handler)
}

#[cfg(test)]
mod tests {
    use super::{MdColors, code_block_text, highlight_for, is_allowed_markdown_url};
    use crate::workspace::main_area::file_view_pane::render::prose::InlineStyle;
    use crate::workspace::main_area::file_view_pane::{HighlightedSpan, VisualRow, VisualRowKind};

    fn row(content: &str, spans: Vec<HighlightedSpan>) -> VisualRow {
        VisualRow {
            kind: VisualRowKind::Context,
            line_no_left: String::new(),
            line_no_right: String::new(),
            content: content.to_string(),
            header_context: String::new(),
            spans,
            word_changes: Vec::new(),
        }
    }

    fn span(text: &str, color: Option<gpui::Hsla>) -> HighlightedSpan {
        HighlightedSpan {
            text: text.to_string(),
            color,
            style: Default::default(),
        }
    }

    const RED: gpui::Hsla = gpui::Hsla {
        h: 0.0,
        s: 1.0,
        l: 0.5,
        a: 1.0,
    };

    /// One distinct hue per slot. With every slot the same colour an assertion
    /// like `color == colors.link` passes for `text` and `subtle` too, so the
    /// contract this fixture exists to pin would go unchecked.
    fn colors() -> MdColors {
        let hue = |h: f32| gpui::Hsla {
            h,
            s: 1.0,
            l: 0.5,
            a: 1.0,
        };
        MdColors {
            text: hue(0.0),
            muted: hue(0.1),
            subtle: hue(0.2),
            link: hue(0.3),
            fill: hue(0.4),
            raised: hue(0.5),
            line: hue(0.6),
        }
    }

    #[test]
    fn inline_style_mapping_matches_the_design_contract() {
        let colors = colors();
        let code = highlight_for(
            InlineStyle {
                code: true,
                ..Default::default()
            },
            &colors,
        );
        assert_eq!(code.color, Some(colors.text));
        assert_eq!(code.background_color, Some(colors.fill));
        assert!(code.underline.is_none());

        let strike = highlight_for(
            InlineStyle {
                strikethrough: true,
                ..Default::default()
            },
            &colors,
        );
        assert_eq!(strike.color, Some(colors.subtle));
        assert!(strike.strikethrough.is_some());

        let link = highlight_for(
            InlineStyle {
                link: true,
                ..Default::default()
            },
            &colors,
        );
        assert_eq!(link.color, Some(colors.link));
        assert!(link.underline.is_none());

        // `[`CLAUDE.md`](…)` — code inside a link. Without the link colour it
        // is indistinguishable from ordinary inline code, and underline is off,
        // so nothing marks it clickable. Code still contributes its fill.
        let linked_code = highlight_for(
            InlineStyle {
                code: true,
                link: true,
                ..Default::default()
            },
            &colors,
        );
        assert_eq!(linked_code.color, Some(colors.link));
        assert_eq!(linked_code.background_color, Some(colors.fill));
    }

    #[test]
    fn markdown_links_allow_only_external_safe_schemes() {
        for url in [
            "https://example.com/path",
            "http://localhost:3000",
            "mailto:dev@example.com",
        ] {
            assert!(is_allowed_markdown_url(url), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "file:///tmp/secret",
            "../relative.md",
            "data:text/html,hello",
        ] {
            assert!(!is_allowed_markdown_url(url), "{url}");
        }
    }

    #[test]
    fn rows_join_with_newlines_so_one_element_covers_the_block() {
        let (text, highlights) =
            code_block_text(&[row("fn a() {}", vec![]), row("fn b() {}", vec![])]);
        assert_eq!(text, "fn a() {}\nfn b() {}");
        assert!(
            highlights.is_empty(),
            "an unhighlighted row takes the surface colour"
        );
    }

    #[test]
    fn a_highlight_range_addresses_the_joined_text_not_its_own_row() {
        let (text, highlights) = code_block_text(&[
            row("let x", vec![]),
            row("", vec![span("let", Some(RED)), span(" y", None)]),
        ]);
        assert_eq!(text, "let x\nlet y");
        assert_eq!(highlights.len(), 1);
        assert_eq!(
            highlights[0].0,
            6..9,
            "offset counts the earlier row and its newline"
        );
        assert_eq!(highlights[0].1.color, Some(RED));
    }

    #[test]
    fn an_empty_span_contributes_no_range() {
        let (text, highlights) =
            code_block_text(&[row("", vec![span("", Some(RED)), span("x", None)])]);
        assert_eq!(text, "x");
        assert!(highlights.is_empty());
    }

    #[test]
    fn every_highlight_lands_on_a_char_boundary() {
        // `StyledText::with_highlights` debug-asserts this, and multi-byte
        // source is ordinary in a code block.
        let (text, highlights) = code_block_text(&[row(
            "",
            vec![span("사과", Some(RED)), span("=1", Some(RED))],
        )]);
        for (range, _) in &highlights {
            assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
        }
    }

    use crate::ui::theme::{
        apply_ui_theme, contrast_ratio, current, file_viewer_pane_bg, init_if_missing,
        set_agent_chat_bg, set_agent_chat_fg,
    };

    /// Every colour this preview paints comes from the surface it is painted
    /// on. It used to come from the UI theme, and the file viewer's surface
    /// mirrors the *terminal* palette — with `ui_preset` and `terminal_preset`
    /// being independent config keys, a light UI theme over a dark terminal put
    /// `#2d2d2d` prose on a near-black pane.
    #[gpui::test]
    fn the_preview_reads_on_the_pane_it_is_painted_on(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            init_if_missing(cx);
            // The combination the defect needed: light chrome, dark terminal.
            apply_ui_theme("daruda_light", cx);
            let mut ui_source_failures = 0;

            for preset in daruda_config::theme_presets::PRESETS {
                let Some(colors) = daruda_config::theme_presets::colors_for_preset(preset.name)
                else {
                    continue;
                };
                let (bg, fg) = (colors.background, colors.foreground);
                set_agent_chat_bg(cx, bg.r, bg.g, bg.b);
                set_agent_chat_fg(cx, fg.r, fg.g, fg.b);
                let pane = file_viewer_pane_bg(cx);
                let t = MdColors::for_pane(cx);

                let body = contrast_ratio(t.text, pane);
                assert!(
                    body >= 4.5,
                    "{}: body prose measures {body:.2}:1",
                    preset.name
                );
                let quote = contrast_ratio(t.muted, pane);
                assert!(
                    quote >= 3.0,
                    "{}: quote prose measures {quote:.2}:1",
                    preset.name
                );
                // Fills and lines have to separate from the surface at all —
                // they are decoration, so this is a visibility floor, not
                // DESIGN.md's 3:1 affordance floor.
                for (what, color) in [("fill", t.fill), ("raised", t.raised), ("line", t.line)] {
                    let ratio = contrast_ratio(color, pane);
                    assert!(
                        ratio > 1.0,
                        "{}: the {what} does not separate from the pane",
                        preset.name
                    );
                }

                // What the UI theme would have painted on this same surface.
                if contrast_ratio(current(cx).text_body, pane) < 4.5 {
                    ui_source_failures += 1;
                }
            }

            assert!(
                ui_source_failures > 0,
                "if the UI theme's body tone were legible on every terminal preset the \
                 pane-derived colours would be unnecessary"
            );
            apply_ui_theme("daruda_dark", cx);
        });
    }
}

#[cfg(test)]
mod layout_tests {
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use super::{
        MdColors, OpenUrl, queue_block_selection, render_md_body_layout,
        take_pending_block_selection, take_pending_block_selection_for_drag,
    };
    use crate::ui::theme;
    use crate::workspace::main_area::file_view_pane::markdown_viewer::parse_markdown;
    use gpui::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers,
        MouseButton, ParentElement as _, Pixels, Render, StatefulInteractiveElement as _,
        Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div,
        point, px, size,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Engine {
        FileViewer,
        AgentChat,
    }

    impl Engine {
        const ALL: [Self; 2] = [Self::FileViewer, Self::AgentChat];

        fn label(self) -> &'static str {
            match self {
                Self::FileViewer => "file viewer",
                Self::AgentChat => "agent chat",
            }
        }
    }

    struct Probe {
        md: String,
        engine: Engine,
        first_inline_bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    }

    struct LinkProbe {
        md: String,
        opened: Arc<Mutex<Vec<String>>>,
        block_mouse_downs: Arc<Mutex<usize>>,
    }

    impl Render for LinkProbe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let colors = MdColors::for_pane(cx);
            let blocks = parse_markdown(&self.md, "default", false);
            let opened = self.opened.clone();
            let on_open_url: OpenUrl = Rc::new(move |url, _, _| {
                opened.lock().unwrap().push(url.to_owned());
            });
            let block_mouse_downs = self.block_mouse_downs.clone();
            let body = render_md_body_layout(
                &blocks,
                None,
                &colors,
                on_open_url,
                move |block, block_idx| {
                    let committed = block_mouse_downs.clone();
                    let dragged = block_mouse_downs.clone();
                    block
                        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                            queue_block_selection(block_idx, event.modifiers.shift, cx);
                        })
                        .on_mouse_move(move |event, _, cx| {
                            if event.pressed_button == Some(MouseButton::Left)
                                && take_pending_block_selection_for_drag(block_idx, cx).is_some()
                            {
                                *dragged.lock().unwrap() += 1;
                            }
                        })
                        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                            if take_pending_block_selection(cx).is_some() {
                                *committed.lock().unwrap() += 1;
                            }
                        })
                },
            )
            .id("md-link-probe")
            .debug_selector(|| "md-link-probe".into());

            div()
                .size_full()
                .font_family("Menlo")
                .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                .child(body)
        }
    }

    impl Render for Probe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            match self.engine {
                Engine::FileViewer => {
                    let colors = MdColors::for_pane(cx);
                    let blocks = parse_markdown(&self.md, "default", false);
                    let body = render_md_body_layout(
                        &blocks,
                        None,
                        &colors,
                        Rc::new(|_, _, _| {}),
                        |block, _| block,
                    )
                    .id("md-probe")
                    .debug_selector(|| "md-probe".into());
                    // Faithful to the real pane, outermost first: the walker's
                    // `flex_1().min_h(0).overflow_hidden()` slot in a column, the pane
                    // root (`relative().size_full()` + the configured font), the
                    // toolbar-offset absolute frame, then `body.rs`'s scroll container.
                    div().flex().flex_col().size_full().child(
                        div().flex_1().min_h(px(0.)).overflow_hidden().child(
                            div().relative().size_full().font_family("Menlo").child(
                                div()
                                    .absolute()
                                    .top(px(theme::FILE_VIEWER_HEADER_H))
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .id("probe-scroll")
                                    .overflow_y_scroll()
                                    .child(body),
                            ),
                        ),
                    )
                }
                Engine::AgentChat => {
                    let first_inline_bounds = self.first_inline_bounds.clone();
                    div().size_full().font_family("Menlo").child(
                        div()
                            .id("md-probe")
                            .debug_selector(|| "md-probe".into())
                            .w_full()
                            .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                            .child(
                                crate::ui::markdown::markdown("conformance", self.md.clone())
                                    .debug_inline_bounds(move |bounds| {
                                        let mut first = first_inline_bounds.lock().unwrap();
                                        if first.is_none() {
                                            *first = Some(bounds);
                                        }
                                    }),
                            ),
                    )
                }
            }
        }
    }

    struct ProbeMeasurement {
        selected: Option<gpui::Size<Pixels>>,
        first_inline: Option<gpui::Size<Pixels>>,
    }

    /// Painted sizes from a `width`-wide probe: the requested div and the
    /// agent renderer's first actual `Inline` prose run, when it has one.
    fn measure_engine(
        cx: &mut TestAppContext,
        engine: Engine,
        md: &str,
        width: Pixels,
        selector: &'static str,
    ) -> ProbeMeasurement {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(width, px(4000.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let md = md.to_string();
        let first_inline_bounds = Arc::new(Mutex::new(None));
        let window = cx
            .update(|cx| {
                let first_inline_bounds = first_inline_bounds.clone();
                cx.open_window(opts, |_w, cx| {
                    cx.new(|_cx| Probe {
                        md,
                        engine,
                        first_inline_bounds,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        *first_inline_bounds.lock().unwrap() = None;
        vcx.update(|w, _| w.refresh());
        vcx.run_until_parked();
        let selected = vcx.debug_bounds(selector).map(|bounds| bounds.size);
        let first_inline = first_inline_bounds
            .lock()
            .unwrap()
            .as_ref()
            .map(|bounds| bounds.size);
        ProbeMeasurement {
            selected,
            first_inline,
        }
    }

    fn bounds_for_engine(
        cx: &mut TestAppContext,
        engine: Engine,
        md: &str,
        width: Pixels,
        selector: &'static str,
    ) -> Option<gpui::Size<Pixels>> {
        measure_engine(cx, engine, md, width, selector).selected
    }

    fn bounds_of(
        cx: &mut TestAppContext,
        md: &str,
        width: Pixels,
        selector: &'static str,
    ) -> gpui::Size<Pixels> {
        bounds_for_engine(cx, Engine::FileViewer, md, width, selector).expect("painted")
    }

    fn height(cx: &mut TestAppContext, md: &str, width: Pixels) -> Pixels {
        bounds_of(cx, md, width, "md-probe").height
    }

    #[gpui::test]
    fn a_blockquote_soft_break_wraps_like_a_space(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let split = "> A blockquote reads one step down from the prose around it, on the bar it is\n\
                     > marked with. It must stay legible on the pane background.";
        let joined = "> A blockquote reads one step down from the prose around it, on the bar it is \
                      marked with. It must stay legible on the pane background.";

        assert_eq!(height(cx, split, px(430.)), height(cx, joined, px(430.)));
    }

    #[gpui::test]
    fn style_boundaries_do_not_change_prose_layout(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let width = px(330.);
        for (label, plain, styled) in [
            (
                "paragraph",
                "alpha beta gamma delta epsilon zeta eta theta",
                "[alpha](https://a.test) [beta](https://b.test) gamma delta epsilon zeta eta theta",
            ),
            (
                "heading",
                "## alpha beta gamma delta epsilon",
                "## [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
            ),
            (
                "list",
                "- alpha beta gamma delta epsilon",
                "- [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
            ),
            (
                "blockquote",
                "> alpha beta gamma delta epsilon",
                "> [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
            ),
            (
                "footnote",
                "[^n]: alpha beta gamma delta epsilon",
                "[^n]: [alpha](https://a.test) [beta](https://b.test) gamma delta epsilon",
            ),
        ] {
            assert_eq!(
                bounds_of(cx, plain, width, "md-probe"),
                bounds_of(cx, styled, width, "md-probe"),
                "{label}"
            );
        }
    }

    #[gpui::test]
    fn table_columns_ignore_inline_style_boundaries(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let plain = "| alpha beta gamma delta | short |\n| --- | --- |\n| body | value |";
        let styled = "| [alpha](https://a.test) [beta](https://b.test) gamma delta | short |\n\
                      | --- | --- |\n| body | value |";

        assert_eq!(
            bounds_of(cx, plain, px(430.), "markdown-table-first-header-cell"),
            bounds_of(cx, styled, px(430.), "markdown-table-first-header-cell")
        );
    }

    #[gpui::test]
    fn inline_images_do_not_split_surrounding_text_into_equal_columns(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let plain = "alpha beta gamma delta epsilon zeta eta theta iota kappa [thumbnail] tail";
        let image = "alpha beta gamma delta epsilon zeta eta theta iota kappa \
                     ![thumbnail](missing.png) tail";

        assert_eq!(height(cx, plain, px(635.)), height(cx, image, px(635.)));
    }

    /// A URL this host will not open must not be clickable at all. Left in the
    /// range list it would take the pointer cursor and consume the click,
    /// discarding the block selection the press would otherwise have started —
    /// a link that does nothing whatsoever.
    #[gpui::test]
    fn a_link_this_host_will_not_open_is_not_clickable(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(430.), px(220.)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let opened = Arc::new(Mutex::new(Vec::new()));
        let block_mouse_downs = Arc::new(Mutex::new(0));
        let window = cx
            .update(|cx| {
                let opened = opened.clone();
                let block_mouse_downs = block_mouse_downs.clone();
                cx.open_window(options, |_window, cx| {
                    cx.new(|_| LinkProbe {
                        md: "[docs](./other.md) tail".into(),
                        opened,
                        block_mouse_downs,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let text = vcx.debug_bounds("md-plain").expect("text painted");
        let inside_link = point(text.origin.x + px(2.), text.center().y);

        vcx.simulate_click(inside_link, Modifiers::none());
        assert!(
            opened.lock().unwrap().is_empty(),
            "a relative link must not reach the URL handler"
        );
        assert!(
            *block_mouse_downs.lock().unwrap() > 0,
            "the click falls through to block selection instead of vanishing"
        );
    }

    #[gpui::test]
    fn interactive_text_opens_only_a_pressed_link_range(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(430.), px(220.)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let opened = Arc::new(Mutex::new(Vec::new()));
        let block_mouse_downs = Arc::new(Mutex::new(0));
        let window = cx
            .update(|cx| {
                let opened = opened.clone();
                let block_mouse_downs = block_mouse_downs.clone();
                cx.open_window(options, |_window, cx| {
                    cx.new(|_| LinkProbe {
                        md: "[open](https://example.com/right) tail".into(),
                        opened,
                        block_mouse_downs,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let text = vcx.debug_bounds("md-plain").expect("text painted");
        let inside_link = point(text.origin.x + px(2.), text.center().y);
        let outside_link = point(text.origin.x + text.size.width - px(2.), text.center().y);

        vcx.simulate_click(inside_link, Modifiers::none());
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            ["https://example.com/right"]
        );
        assert_eq!(*block_mouse_downs.lock().unwrap(), 0);

        let prior_block_downs = *block_mouse_downs.lock().unwrap();
        vcx.simulate_click(outside_link, Modifiers::none());
        assert_eq!(opened.lock().unwrap().len(), 1);
        assert!(*block_mouse_downs.lock().unwrap() > prior_block_downs);

        vcx.simulate_mouse_down(inside_link, MouseButton::Left, Modifiers::none());
        vcx.simulate_mouse_up(outside_link, MouseButton::Left, Modifiers::none());
        assert_eq!(opened.lock().unwrap().len(), 1);

        let prior_block_downs = *block_mouse_downs.lock().unwrap();
        let moved_inside_link = point(inside_link.x + px(1.), inside_link.y);
        vcx.simulate_mouse_down(inside_link, MouseButton::Left, Modifiers::none());
        vcx.simulate_mouse_move(
            moved_inside_link,
            Some(MouseButton::Left),
            Modifiers::none(),
        );
        vcx.simulate_mouse_up(moved_inside_link, MouseButton::Left, Modifiers::none());
        assert_eq!(opened.lock().unwrap().len(), 2);
        assert_eq!(*block_mouse_downs.lock().unwrap(), prior_block_downs);

        let prior_block_downs = *block_mouse_downs.lock().unwrap();
        vcx.simulate_mouse_down(inside_link, MouseButton::Right, Modifiers::none());
        vcx.simulate_mouse_up(inside_link, MouseButton::Right, Modifiers::none());
        assert_eq!(opened.lock().unwrap().len(), 2);
        assert_eq!(*block_mouse_downs.lock().unwrap(), prior_block_downs);
    }

    #[gpui::test]
    fn linked_images_open_without_committing_block_selection(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(430.), px(220.)));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let opened = Arc::new(Mutex::new(Vec::new()));
        let block_mouse_downs = Arc::new(Mutex::new(0));
        let window = cx
            .update(|cx| {
                let opened = opened.clone();
                let block_mouse_downs = block_mouse_downs.clone();
                cx.open_window(options, |_window, cx| {
                    cx.new(|_| LinkProbe {
                        md: "[![thumbnail](missing.png)](https://example.com/full)".into(),
                        opened,
                        block_mouse_downs,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        let image = vcx
            .debug_bounds("markdown-linked-image")
            .expect("linked image painted");

        vcx.simulate_click(image.center(), Modifiers::none());
        assert_eq!(
            opened.lock().unwrap().as_slice(),
            ["https://example.com/full"]
        );
        assert_eq!(*block_mouse_downs.lock().unwrap(), 0);

        vcx.simulate_mouse_down(image.center(), MouseButton::Right, Modifiers::none());
        vcx.simulate_mouse_up(image.center(), MouseButton::Right, Modifiers::none());
        assert_eq!(opened.lock().unwrap().len(), 1);
    }

    /// The gap a loose list takes is the one between blocks, so three items
    /// grow by exactly two steps. Asserting the step and not just `>` is what
    /// catches the gap being swapped for some other constant.
    #[gpui::test]
    fn a_loose_list_spaces_its_items(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let step = px(theme::MD_BLOCK_GAP - theme::MD_LIST_ITEM_GAP);

        for engine in Engine::ALL {
            for (label, tight, loose) in [
                (
                    "bullet",
                    "- AAAA\n- BBBB\n- CCCC",
                    "- AAAA\n\n- BBBB\n\n- CCCC",
                ),
                (
                    "ordered",
                    "1. AAAA\n2. BBBB\n3. CCCC",
                    "1. AAAA\n\n2. BBBB\n\n3. CCCC",
                ),
            ] {
                let tight = bounds_for_engine(cx, engine, tight, px(430.), "md-probe")
                    .expect("body painted")
                    .height;
                let loose = bounds_for_engine(cx, engine, loose, px(430.), "md-probe")
                    .expect("body painted")
                    .height;
                assert!(
                    loose > tight,
                    "{} {label}: loose={loose:?} tight={tight:?}",
                    engine.label()
                );
                if engine == Engine::FileViewer {
                    assert_eq!(loose - tight, step * 2., "{label}");
                }
            }
        }
    }

    /// CommonMark has two loose-list spellings. Both renderers must treat an
    /// item containing two blocks like the equivalent list whose following
    /// item is also separated by a blank line.
    #[gpui::test]
    fn both_engines_recognize_a_multi_block_item_as_loose(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);

        for engine in Engine::ALL {
            for (label, multi_block, blank_between) in [
                (
                    "bullet",
                    "- AAAA\n\n  cont\n- BBBB",
                    "- AAAA\n\n  cont\n\n- BBBB",
                ),
                (
                    "ordered",
                    "1. AAAA\n\n   cont\n2. BBBB",
                    "1. AAAA\n\n   cont\n\n2. BBBB",
                ),
            ] {
                let multi = bounds_for_engine(cx, engine, multi_block, px(430.), "md-probe")
                    .expect("body painted")
                    .height;
                let blank = bounds_for_engine(cx, engine, blank_between, px(430.), "md-probe")
                    .expect("body painted")
                    .height;
                assert_eq!(
                    multi,
                    blank,
                    "{} {label}: multi-block={multi:?} blank-line={blank:?}",
                    engine.label()
                );
            }
        }
    }

    #[gpui::test]
    fn both_engines_render_task_markers_as_checkboxes(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);

        for engine in Engine::ALL {
            for md in ["- [ ] unchecked", "- [x] checked"] {
                assert!(
                    bounds_for_engine(cx, engine, md, px(430.), "markdown-task-checkbox").is_some(),
                    "{} did not paint a checkbox for {md:?}",
                    engine.label()
                );
            }
            assert!(
                bounds_for_engine(cx, engine, "- plain", px(430.), "markdown-task-checkbox")
                    .is_none(),
                "{} painted a checkbox for a plain item",
                engine.label()
            );
        }
    }

    /// A paragraph break is block-level. Left inside a wrapping row as a
    /// full-width flex item, it collapsed the *preceding* text element to zero
    /// width — the prose then wrapped one character per line and ran over
    /// whatever sat below. Block height never moved, so only the text element's
    /// own painted width catches it.
    ///
    /// Every block that can hold a break is covered: the list item was fixed
    /// first and the blockquote stayed broken behind it. The assertion is the
    /// collapse itself, not a width the three would share — the footnote's
    /// inline label leaves it a different amount of room.
    #[gpui::test]
    fn a_second_paragraph_does_not_squeeze_the_first(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let long = "the quick brown fox jumps over the lazy dog and keeps running far";
        let w = px(635.);

        for engine in Engine::ALL {
            for (label, two) in [
                ("list item", format!("- {long}\n\n  {long}")),
                ("blockquote", format!("> {long}\n>\n> {long}")),
                ("footnote", format!("[^a]: {long}\n\n    {long}")),
            ] {
                let first = match engine {
                    Engine::FileViewer => bounds_for_engine(cx, engine, &two, w, "md-plain"),
                    Engine::AgentChat => {
                        measure_engine(cx, engine, &two, w, "md-probe").first_inline
                    }
                }
                .unwrap_or_else(|| panic!("{} {label}: no prose painted", engine.label()))
                .width;
                assert!(
                    first > px(0.),
                    "{} {label}: a second paragraph squeezed the first to {first:?}",
                    engine.label()
                );
            }
        }
    }
}
