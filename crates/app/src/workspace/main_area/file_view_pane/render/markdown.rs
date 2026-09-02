//! Markdown preview renderer — block + span tree → GPUI elements,
//! plus block-level click/drag selection plumbed through `Workspace`.

use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use gpui::{
    AnyElement, App, Context, HighlightStyle, ImageSource, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, RenderImage, StyledText, div, img, prelude::*, px,
};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::markdown_viewer::{
    ListItem, MdBlock, MdSpan, lone_image,
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
/// Semantic hues (`SUCCESS` for a ticked task box, `AGENT_RUNNING` for inline
/// code) are deliberately not in here: they carry meaning rather than surface
/// rank, and daruda uses them on pane surfaces elsewhere.
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
    /// The stronger fill, one step above [`Self::fill`] — inline-code chip,
    /// table header (the `BG_RAISED` rung).
    raised: gpui::Hsla,
    /// Code-block border, table lines, the `<hr>` rule.
    line: gpui::Hsla,
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
    render_md_body_layout(blocks, char_selection, &t, |block, block_idx| {
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
        let block_el = div()
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .when(is_sel, |d| d.bg(block_sel_bg))
            .child(render_md_block(block, t));
        col = col.child(decorate_block(block_el, i));
    }
    col
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
fn render_md_prose(spans: &[MdSpan], t: &MdColors) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .w_0()
        .gap(px(theme::MD_BLOCK_GAP))
        .children(
            spans
                .split(|s| matches!(s, MdSpan::ParagraphBreak))
                .map(|run| {
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .children(render_md_spans(run, t))
                }),
        )
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
fn render_list(items: &[ListItem], loose: bool, marker: ListMarker, t: &MdColors) -> AnyElement {
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
                .child(render_md_prose(&item.spans, t)),
        );
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

fn render_md_block(block: &MdBlock, t: &MdColors) -> AnyElement {
    match block {
        MdBlock::Heading { level, spans } => {
            let (size, color, mt) = match level {
                1 => (theme::MD_H1_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                2 => (theme::MD_H2_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                3 => (theme::MD_H3_FONT_SIZE, t.text, theme::MD_HEADING_MARGIN_TOP),
                _ => (theme::MD_H4_FONT_SIZE, t.text, 0.0),
            };
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .w_full()
                .min_w_0()
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
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .children(render_md_spans(spans, t))
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

        MdBlock::BulletList { items, loose } => render_list(items, *loose, ListMarker::Bullet, t),

        MdBlock::OrderedList {
            start,
            items,
            loose,
        } => render_list(items, *loose, ListMarker::Ordered(*start), t),

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
            .child(render_md_prose(spans, t).italic().text_color(t.muted))
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
            .text_size(px(theme::MD_FOOTNOTE_FONT_SIZE))
            .text_color(t.subtle)
            .child(div().flex_none().child(format!("[^{label}]:")))
            .child(render_md_prose(spans, t))
            .into_any_element(),

        MdBlock::HtmlBlock(html) => div()
            .font(gpui::font("monospace"))
            .text_size(px(theme::MD_HTML_FONT_SIZE))
            .text_color(t.subtle)
            .child(html.clone())
            .into_any_element(),

        MdBlock::Table { header, rows } => {
            let table_border = t.line;
            let body_text = t.text;
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
                            .w_full()
                            .min_w_0()
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
                .bg(t.raised)
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
                    // Both rungs were the same colour in the UI palette, so the
                    // stripe never showed; one fill keeps that appearance.
                    let bg = t.fill;
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

fn render_md_spans(spans: &[MdSpan], t: &MdColors) -> Vec<AnyElement> {
    debug_assert!(
        !spans
            .iter()
            .any(|span| matches!(span, MdSpan::ParagraphBreak)),
        "paragraph breaks must be split by render_md_prose"
    );

    let mut els: Vec<AnyElement> = Vec::new();
    let mut plain_text = String::new();

    for span in spans {
        match span {
            MdSpan::Text(text) => plain_text.push_str(text),
            // A Markdown soft break is semantically a space. Keeping it in the
            // same text element lets GPUI shape and wrap the prose as one run;
            // a standalone flex child can otherwise occupy a row by itself.
            MdSpan::SoftBreak => plain_text.push(' '),
            _ => {
                if !plain_text.is_empty() {
                    els.push(render_plain_text(std::mem::take(&mut plain_text)));
                }
                els.push(render_md_span(span, t));
            }
        }
    }
    if !plain_text.is_empty() {
        els.push(render_plain_text(plain_text));
    }
    els
}

fn render_plain_text(text: String) -> AnyElement {
    div()
        .min_w_0()
        .whitespace_normal()
        .when(cfg!(test), |d| d.debug_selector(|| "md-plain".into()))
        .child(text)
        .into_any_element()
}

fn render_md_span(span: &MdSpan, t: &MdColors) -> AnyElement {
    match span {
        MdSpan::Text(s) => render_plain_text(s.clone()),

        MdSpan::Bold(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .min_w_0()
            .font_weight(gpui::FontWeight::BOLD)
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::Italic(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .min_w_0()
            .italic()
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::Code(s) => div()
            .font(gpui::font("monospace"))
            .bg(t.raised)
            .text_color(theme::AGENT_RUNNING)
            .px(px(theme::MD_CODE_INLINE_PAD_X))
            .rounded(px(theme::MD_BLOCK_RADIUS))
            .child(s.clone())
            .into_any_element(),

        MdSpan::Link { text, .. } => div()
            .text_color(t.link)
            .child(text.clone())
            .into_any_element(),

        MdSpan::Strikethrough(inner) => div()
            .flex()
            .flex_row()
            .flex_wrap()
            .min_w_0()
            .text_color(t.subtle)
            .children(render_md_spans(inner, t))
            .into_any_element(),

        MdSpan::SoftBreak => div().child(" ").into_any_element(),
        // A full-width item ends the flex line; it carries no height of its own
        // because `<br>` moves to the next line rather than leaving a gap.
        MdSpan::HardBreak => div().w_full().h_0().into_any_element(),
        // Every block that can hold one goes through `render_md_prose`, which
        // splits the run here instead. Reaching this arm means a caller took
        // the wrapping row directly, and the row's preceding text collapses to
        // zero width — so leave a gap, but keep the split as the way in.
        MdSpan::ParagraphBreak => div().w_full().h(px(theme::MD_BLOCK_GAP)).into_any_element(),

        MdSpan::Footnote(label) => div()
            .text_color(t.subtle)
            .text_size(px(theme::MD_FOOTNOTE_FONT_SIZE))
            .child(format!("[^{label}]"))
            .into_any_element(),

        MdSpan::Html(s) => div()
            .font(gpui::font("monospace"))
            .text_color(t.subtle)
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

#[cfg(test)]
mod tests {
    use super::{MdColors, code_block_text, render_md_spans};
    use crate::workspace::main_area::file_view_pane::markdown_viewer::MdSpan;
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

    #[test]
    #[should_panic(expected = "paragraph breaks must be split by render_md_prose")]
    fn a_direct_span_run_rejects_paragraph_breaks() {
        let colors = MdColors {
            text: RED,
            muted: RED,
            subtle: RED,
            link: RED,
            fill: RED,
            raised: RED,
            line: RED,
        };

        let _ = render_md_spans(&[MdSpan::ParagraphBreak], &colors);
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
    use std::sync::{Arc, Mutex};

    use super::{MdColors, render_md_body_layout};
    use crate::ui::theme;
    use crate::workspace::main_area::file_view_pane::markdown_viewer::parse_markdown;
    use gpui::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _,
        Pixels, Render, StatefulInteractiveElement as _, Styled as _, TestAppContext,
        VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
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

    impl Render for Probe {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            match self.engine {
                Engine::FileViewer => {
                    let colors = MdColors::for_pane(cx);
                    let blocks = parse_markdown(&self.md, "default", false);
                    let body = render_md_body_layout(&blocks, None, &colors, |block, _| block)
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
