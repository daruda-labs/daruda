//! What the parser leaves for later about images: filling them in, and asking
//! about them.
//!
//! [`resolve_images`] and [`resolve_mermaid`] are the second pass over a parsed
//! document — the blocks and spans that only *name* an image or a diagram get
//! their pixels. Both are pure traversals; the caller supplies the loading and
//! the mermaid rendering as closures, so this stays free of I/O and of merman.
//!
//! [`lone_image`] is not a pass but a question the renderer asks of a finished
//! paragraph: is this image standalone, and so free to take the pane's width?

use super::{MdBlock, MdSpan};
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

/// Walk every image span in `blocks` (recursing into nested spans and list
/// item children) and fill its `raster` via `resolve`. Pure traversal — the
/// caller supplies the I/O (file read + decode) as the closure.
pub(in crate::workspace) fn resolve_images(
    blocks: &mut [MdBlock],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Heading { spans, .. }
            | MdBlock::Paragraph(spans)
            | MdBlock::Blockquote(spans)
            | MdBlock::FootnoteDefinition { spans, .. } => {
                resolve_images_in_spans(spans, resolve);
            }
            MdBlock::BulletList { items, .. } | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_images_in_spans(&mut item.spans, resolve);
                    resolve_images(&mut item.children, resolve);
                }
            }
            MdBlock::Table { header, rows } => {
                for cell in header {
                    resolve_images_in_spans(cell, resolve);
                }
                for row in rows {
                    for cell in row {
                        resolve_images_in_spans(cell, resolve);
                    }
                }
            }
            MdBlock::CodeBlock { .. }
            | MdBlock::Rule
            | MdBlock::HtmlBlock(_)
            | MdBlock::Mermaid { .. } => {}
        }
    }
}

/// If `spans` is a single image (ignoring surrounding whitespace), return its
/// `(alt, raster)`. Such a paragraph renders the image block-style (large);
/// otherwise images render inline, sized to the text line.
pub(in crate::workspace) fn lone_image(spans: &[MdSpan]) -> Option<(&str, Option<&RasterImage>)> {
    let mut found: Option<(&str, Option<&RasterImage>)> = None;
    for span in spans {
        match span {
            MdSpan::Image { alt, raster, .. } => {
                if found.is_some() {
                    return None;
                }
                found = Some((alt.as_str(), raster.as_ref()));
            }
            MdSpan::SoftBreak | MdSpan::HardBreak | MdSpan::ParagraphBreak => {}
            MdSpan::Text(t) if t.trim().is_empty() => {}
            _ => return None,
        }
    }
    found
}

/// Walk every mermaid block in `blocks` (recursing into list item children) and
/// fill its `raster` via `resolve`. Pure traversal — the caller supplies the
/// rendering (merman → SVG → rasterize) as the closure.
pub(in crate::workspace) fn resolve_mermaid(
    blocks: &mut [MdBlock],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Mermaid { source, raster } => {
                if raster.is_none() {
                    *raster = resolve(source);
                }
            }
            MdBlock::BulletList { items, .. } | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_mermaid(&mut item.children, resolve);
                }
            }
            MdBlock::Heading { .. }
            | MdBlock::Paragraph(_)
            | MdBlock::CodeBlock { .. }
            | MdBlock::Blockquote(_)
            | MdBlock::Rule
            | MdBlock::Table { .. }
            | MdBlock::FootnoteDefinition { .. }
            | MdBlock::HtmlBlock(_) => {}
        }
    }
}

fn resolve_images_in_spans(
    spans: &mut [MdSpan],
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for span in spans {
        match span {
            MdSpan::Image { url, raster, .. } => {
                if raster.is_none() {
                    *raster = resolve(url);
                }
            }
            MdSpan::Bold(inner)
            | MdSpan::Italic(inner)
            | MdSpan::Link {
                children: inner, ..
            }
            | MdSpan::Strikethrough(inner) => {
                resolve_images_in_spans(inner, resolve);
            }
            MdSpan::Text(_)
            | MdSpan::Code(_)
            | MdSpan::SoftBreak
            | MdSpan::HardBreak
            | MdSpan::ParagraphBreak
            | MdSpan::Footnote(_)
            | MdSpan::Html(_) => {}
        }
    }
}
