//! One [`MdBlock`] → a GPUI element.
//!
//! The dispatch plus the block kinds that need more than a call into
//! [`super::inline`]: the code card, lists and tables.

use gpui::{AnyElement, HighlightStyle, IntoElement, StyledText, div, prelude::*, px};

use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::VisualRow;
use crate::workspace::main_area::file_view_pane::markdown_viewer::{
    ListItem, MdBlock, MdSpan, lone_image,
};

use super::image::{ImageLayout, render_md_image};
use super::inline::{render_md_prose, render_prose_run};
use super::{MdRenderAssets, OpenUrl};

#[derive(Clone, Copy)]
struct TableCellPosition {
    is_header: bool,
    is_first: bool,
    is_last: bool,
}

/// The monospace card both a fenced code block and an unrendered mermaid fence
/// sit in.
fn code_surface(assets: MdRenderAssets<'_>) -> gpui::Div {
    let t = assets.t;
    div()
        .flex()
        .flex_col()
        .bg(t.fill)
        .border_1()
        .border_color(t.line)
        .rounded(px(theme::MD_CODE_BLOCK_RADIUS))
        .px(px(theme::MD_CODE_BLOCK_PAD_X))
        .py(px(theme::MD_CODE_BLOCK_PAD_Y))
        .text_size(px(assets.font_size))
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
pub(super) fn code_block_text(
    rows: &[VisualRow],
) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
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
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let t = assets.t;
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

        // The marker sits beside the item's whole content, so every block —
        // a second paragraph, a fence, a nested list — lines up under the
        // first and keeps its document order. Blocks space out only in a
        // loose list; a tight one keeps a nested sublist flush.
        let mut content = div().flex_1().w_0();
        for (j, block) in item.blocks.iter().enumerate() {
            content = content.child(
                div()
                    .when(j > 0 && loose, |d| d.mt(px(theme::MD_BLOCK_GAP)))
                    .child(render_md_block(
                        block,
                        assets,
                        block_idx,
                        next_part_idx,
                        on_open_url,
                    )),
            );
        }
        list = list.child(
            div()
                .flex()
                .flex_row()
                .gap(px(theme::MD_LIST_ROW_GAP))
                .child(cell)
                .child(content),
        );
    }
    list.into_any_element()
}

pub(super) fn render_md_block(
    block: &MdBlock,
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> AnyElement {
    let t = assets.t;
    match block {
        MdBlock::Heading { level, spans } => {
            let (scale, color, mt) = match level {
                1 => (
                    theme::MD_H1_FONT_SCALE,
                    t.text,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                2 => (
                    theme::MD_H2_FONT_SCALE,
                    t.text,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                3 => (
                    theme::MD_H3_FONT_SCALE,
                    t.text,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                _ => (theme::MD_H4_FONT_SCALE, t.text, 0.0),
            };
            div()
                .w_full()
                .min_w_0()
                .mt(px(mt))
                .text_size(px(assets.font_size * scale))
                .text_color(color)
                .font_weight(gpui::FontWeight::BOLD)
                .child(render_prose_run(
                    spans,
                    assets,
                    block_idx,
                    next_part_idx,
                    on_open_url,
                ))
                .into_any_element()
        }

        MdBlock::Paragraph(spans) => {
            // A paragraph that is just an image renders it block-style (large);
            // an image mixed with text renders inline, sized to the line.
            if let Some((alt, slot)) = lone_image(spans) {
                render_md_image(assets.images.get(slot), alt, ImageLayout::Block, t)
            } else {
                div()
                    .w_full()
                    .min_w_0()
                    .child(render_prose_run(
                        spans,
                        assets,
                        block_idx,
                        next_part_idx,
                        on_open_url,
                    ))
                    .into_any_element()
            }
        }

        MdBlock::CodeBlock { rows, .. } => {
            let (text, highlights) = code_block_text(rows);
            code_surface(assets)
                .child(StyledText::new(text).with_highlights(highlights))
                .into_any_element()
        }

        MdBlock::Mermaid { source, slot } => match assets.images.get(*slot) {
            Some(image) => image.block_diagram(),
            // Rendering failed/pending: fall back to the raw source, styled
            // like a code block.
            None => code_surface(assets)
                .child(StyledText::new(source.clone()))
                .into_any_element(),
        },

        MdBlock::BulletList { items, loose } => render_list(
            items,
            *loose,
            ListMarker::Bullet,
            assets,
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
            assets,
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
                render_md_prose(spans, assets, block_idx, next_part_idx, on_open_url)
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
                assets,
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
                    assets,
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
                        assets,
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
    assets: MdRenderAssets<'_>,
    block_idx: usize,
    next_part_idx: &mut usize,
    on_open_url: &OpenUrl,
) -> gpui::Div {
    let t = assets.t;
    div()
        .flex_1()
        .min_w(px(theme::MD_TABLE_CELL_MIN_W))
        .overflow_hidden()
        .px(px(theme::MD_TABLE_CELL_PAD_X))
        .py(px(theme::MD_TABLE_CELL_PAD_Y))
        .when(!position.is_last, |d| d.border_r_1().border_color(t.line))
        .text_size(px(assets.font_size))
        .text_color(t.text)
        .when(position.is_header, |d| {
            d.font_weight(gpui::FontWeight::BOLD)
        })
        .when(cfg!(test) && position.is_header && position.is_first, |d| {
            d.debug_selector(|| "markdown-table-first-header-cell".into())
        })
        .child(render_prose_run(
            cell,
            assets,
            block_idx,
            next_part_idx,
            on_open_url,
        ))
}
