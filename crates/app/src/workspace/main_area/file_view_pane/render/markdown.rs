//! Markdown preview renderer — block + span tree → GPUI elements,
//! plus block-level click/drag selection plumbed through `Workspace`.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{
    AnyElement, Context, ImageSource, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    RenderImage, div, img, prelude::*, px,
};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::CharSelection;
use crate::workspace::main_area::file_view_pane::markdown_viewer::{MdBlock, MdSpan, lone_image};
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

/// Top-level Markdown body: a padded column of selectable blocks.
pub(super) fn render_md_body(
    blocks: &[MdBlock],
    char_selection: Option<&CharSelection>,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let t = theme::current(cx);
    let body_text = t.text_body;
    let block_sel_bg = theme::SELECTION_BG;
    let t = t.clone();
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
        let block_el = div()
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .when(is_sel, |d| d.bg(block_sel_bg))
            .child(render_md_block(block, &t));
        col = col.child(block_with_selection(block_el, i, cx));
    }
    col
}

fn render_md_block(block: &MdBlock, t: &DarudaTheme) -> AnyElement {
    match block {
        MdBlock::Heading { level, spans } => {
            let (size, color, mt) = match level {
                1 => (
                    theme::MD_H1_FONT_SIZE,
                    t.md_h1_color,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                2 => (
                    theme::MD_H2_FONT_SIZE,
                    t.md_h2_color,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                3 => (
                    theme::MD_H3_FONT_SIZE,
                    t.md_h3_color,
                    theme::MD_HEADING_MARGIN_TOP,
                ),
                _ => (theme::MD_H4_FONT_SIZE, t.md_h4_color, 0.0),
            };
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .mt(px(mt))
                .text_size(px(size))
                .text_color(color)
                .font_weight(gpui::FontWeight::BOLD)
                .children(render_md_spans(spans, t))
                .into_any_element()
        }

        MdBlock::Paragraph(spans) => {
            // A paragraph that is just an image renders it block-style (large);
            // an image mixed with text renders inline, sized to the line.
            if let Some((alt, raster)) = lone_image(spans) {
                render_md_image(raster, alt, ImageLayout::Block, t)
            } else {
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .children(render_md_spans(spans, t))
                    .into_any_element()
            }
        }

        MdBlock::CodeBlock { rows, .. } => {
            let body_text = t.text_body;
            let mut code_col = div()
                .flex()
                .flex_col()
                .bg(t.md_code_block_bg)
                .border_1()
                .border_color(t.border)
                .rounded(px(theme::MD_CODE_BLOCK_RADIUS))
                .px(px(theme::MD_CODE_BLOCK_PAD_X))
                .py(px(theme::MD_CODE_BLOCK_PAD_Y))
                .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                .font(gpui::font("monospace"));
            for row in rows {
                let line_el = if row.spans.is_empty() {
                    div()
                        .text_color(body_text)
                        .child(row.content.clone())
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .flex_row()
                        .children(row.spans.iter().filter(|s| !s.text.is_empty()).map(|s| {
                            div()
                                .text_color(s.color.unwrap_or(body_text))
                                .child(s.text.clone())
                                .into_any_element()
                        }))
                        .into_any_element()
                };
                code_col = code_col.child(line_el);
            }
            code_col.into_any_element()
        }

        MdBlock::Mermaid { source, raster } => match raster {
            Some(raster) => render_md_image(Some(raster), "", ImageLayout::Diagram, t),
            None => {
                // Rendering failed/pending: fall back to the raw source, styled
                // like a code block.
                let body_text = t.text_body;
                let mut col = div()
                    .flex()
                    .flex_col()
                    .bg(t.md_code_block_bg)
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(theme::MD_CODE_BLOCK_RADIUS))
                    .px(px(theme::MD_CODE_BLOCK_PAD_X))
                    .py(px(theme::MD_CODE_BLOCK_PAD_Y))
                    .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                    .font(gpui::font("monospace"))
                    .text_color(body_text);
                for line in source.lines() {
                    col = col.child(div().child(line.to_owned()).into_any_element());
                }
                col.into_any_element()
            }
        },

        MdBlock::BulletList(items) => {
            let mut list = div().flex().flex_col().gap(px(theme::MD_LIST_ITEM_GAP));
            for item in items {
                let (bullet, bullet_color) = match item.checked {
                    Some(true) => ("☑", theme::SUCCESS),
                    Some(false) => ("☐", t.text_muted),
                    None => ("•", t.text_muted),
                };
                let mut item_col = div().flex().flex_col();
                let row = div()
                    .flex()
                    .flex_row()
                    .gap(px(theme::MD_LIST_ROW_GAP))
                    .child(div().flex_none().text_color(bullet_color).child(bullet))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .children(render_md_spans(&item.spans, t)),
                    );
                item_col = item_col.child(row);
                for child in &item.children {
                    item_col = item_col.child(
                        div()
                            .pl(px(theme::MD_LIST_INDENT))
                            .child(render_md_block(child, t)),
                    );
                }
                list = list.child(item_col);
            }
            list.into_any_element()
        }

        MdBlock::OrderedList { start, items } => {
            let mut list = div().flex().flex_col().gap(px(theme::MD_LIST_ITEM_GAP));
            for (i, item) in items.iter().enumerate() {
                let num = format!("{}.", start + i as u64);
                let mut item_col = div().flex().flex_col();
                let row = div()
                    .flex()
                    .flex_row()
                    .gap(px(theme::MD_LIST_ROW_GAP))
                    .child(
                        div()
                            .flex_none()
                            .min_w(px(theme::MD_LIST_INDENT))
                            .text_color(t.text_muted)
                            .child(num),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .children(render_md_spans(&item.spans, t)),
                    );
                item_col = item_col.child(row);
                for child in &item.children {
                    item_col = item_col.child(
                        div()
                            .pl(px(theme::MD_LIST_INDENT))
                            .child(render_md_block(child, t)),
                    );
                }
                list = list.child(item_col);
            }
            list.into_any_element()
        }

        MdBlock::Blockquote(spans) => div()
            .flex()
            .flex_row()
            .gap(px(theme::MD_BLOCKQUOTE_PAD_L))
            .child(
                div()
                    .flex_none()
                    .w(px(theme::MD_BLOCKQUOTE_BORDER_W))
                    .bg(t.border)
                    .rounded(px(theme::MD_BLOCKQUOTE_BORDER_W / 2.0)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .italic()
                    .text_color(t.text_muted)
                    .children(render_md_spans(spans, t)),
            )
            .into_any_element(),

        MdBlock::Rule => div()
            .w_full()
            .h(px(theme::MD_RULE_H))
            .bg(t.border)
            .my(px(theme::MD_BLOCK_MARGIN_Y))
            .into_any_element(),

        MdBlock::FootnoteDefinition { label, spans } => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(theme::MD_LIST_ROW_GAP))
            .text_size(px(theme::MD_FOOTNOTE_FONT_SIZE))
            .text_color(t.md_footnote_color)
            .child(div().flex_none().child(format!("[^{label}]:")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .children(render_md_spans(spans, t)),
            )
            .into_any_element(),

        MdBlock::HtmlBlock(html) => div()
            .font(gpui::font("monospace"))
            .text_size(px(theme::MD_HTML_FONT_SIZE))
            .text_color(t.text_subtle)
            .child(html.clone())
            .into_any_element(),

        MdBlock::Table { header, rows } => {
            let table_border = t.border;
            let body_text = t.text_body;
            let render_cell = |cell: &[MdSpan], is_header: bool, is_last: bool| {
                let mut d = div()
                    .flex_1()
                    .min_w(px(theme::MD_TABLE_CELL_MIN_W))
                    .overflow_hidden()
                    .px(px(theme::MD_TABLE_CELL_PAD_X))
                    .py(px(theme::MD_TABLE_CELL_PAD_Y))
                    // Only interior cells get a right border; outer border from the table div handles the edge.
                    .when(!is_last, |d| d.border_r_1().border_color(table_border))
                    .text_size(px(theme::FILE_VIEWER_FONT_SIZE))
                    .text_color(body_text)
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .whitespace_normal()
                            .children(render_md_spans(cell, t)),
                    );
                if is_header {
                    d = d.font_weight(gpui::FontWeight::BOLD);
                }
                d
            };

            let col_count = header.len();
            let row_count = rows.len();
            let header_row = div()
                .flex()
                .flex_row()
                .w_full()
                .border_b_1()
                .border_color(table_border)
                .bg(t.md_table_header_bg)
                .children(
                    header
                        .iter()
                        .enumerate()
                        .map(|(i, cell)| render_cell(cell, true, i + 1 == col_count)),
                );

            let body_rows: Vec<AnyElement> = rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let bg = if i % 2 == 0 {
                        t.md_table_row_bg_even
                    } else {
                        t.md_table_row_bg_odd
                    };
                    let is_last_row = i + 1 == row_count;
                    div()
                        .flex()
                        .flex_row()
                        .w_full()
                        // Skip bottom border on the last row — the table's outer border_1 covers it.
                        .when(!is_last_row, |d| d.border_b_1().border_color(table_border))
                        .bg(bg)
                        .children(
                            row.iter()
                                .enumerate()
                                .map(|(j, cell)| render_cell(cell, false, j + 1 == row.len())),
                        )
                        .into_any_element()
                })
                .collect();

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

fn render_md_spans(spans: &[MdSpan], t: &DarudaTheme) -> Vec<AnyElement> {
    let mut els: Vec<AnyElement> = Vec::new();
    for span in spans {
        els.push(render_md_span(span, t));
    }
    els
}

fn render_md_span(span: &MdSpan, t: &DarudaTheme) -> AnyElement {
    match span {
        MdSpan::Text(s) => div().child(s.clone()).into_any_element(),

        MdSpan::Bold(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .font_weight(gpui::FontWeight::BOLD)
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::Italic(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .italic()
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::Code(s) => div()
            .font(gpui::font("monospace"))
            .bg(t.md_code_inline_bg)
            .text_color(theme::AGENT_RUNNING)
            .px(px(theme::MD_CODE_INLINE_PAD_X))
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .child(s.clone())
            .into_any_element(),

        MdSpan::Link { text, .. } => div()
            .text_color(theme::PRIMARY)
            .child(text.clone())
            .into_any_element(),

        MdSpan::Strikethrough(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .text_color(t.text_subtle)
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::SoftBreak => div().child(" ").into_any_element(),
        MdSpan::HardBreak => div()
            .w_full()
            .h(px(theme::FILE_VIEWER_FONT_SIZE))
            .into_any_element(),

        MdSpan::Footnote(label) => div()
            .text_color(t.md_footnote_color)
            .text_size(px(theme::MD_FOOTNOTE_FONT_SIZE))
            .child(format!("[^{label}]"))
            .into_any_element(),

        MdSpan::Html(s) => div()
            .font(gpui::font("monospace"))
            .text_color(t.text_subtle)
            .text_size(px(theme::MD_HTML_FONT_SIZE))
            .child(s.clone())
            .into_any_element(),

        MdSpan::Image { alt, raster, .. } => {
            render_md_image(raster.as_ref(), alt, ImageLayout::Inline, t)
        }
    }
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
    t: &DarudaTheme,
) -> AnyElement {
    let Some(raster) = raster else {
        return div()
            .text_color(t.md_footnote_color)
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
fn block_with_selection(
    block_div: gpui::Div,
    block_idx: usize,
    cx: &mut Context<Workspace>,
) -> gpui::Div {
    let down_handler = cx.listener(move |this, ev: &MouseDownEvent, _window, cx| {
        if let Some(fv) = this.focused_file_view_mut() {
            fv.handle_block_mouse_down(block_idx, ev.modifiers.shift);
            cx.notify();
        }
    });
    let move_handler = cx.listener(move |this, ev: &MouseMoveEvent, _window, cx| {
        if let Some(fv) = this.focused_file_view_mut() {
            let left_pressed = ev.pressed_button == Some(MouseButton::Left);
            if fv.handle_block_mouse_move(block_idx, left_pressed) {
                cx.notify();
            }
        }
    });
    block_div
        .cursor_default()
        .on_mouse_down(MouseButton::Left, down_handler)
        .on_mouse_move(move_handler)
}
