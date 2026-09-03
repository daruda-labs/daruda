//! What the parser leaves for later about images: loading them, numbering
//! them, and asking about them.
//!
//! [`resolve_images`] and [`resolve_mermaid`] are the second pass over a parsed
//! document — the blocks and spans that only *name* an image or a diagram get
//! a table slot, and the pixels behind it land in a shared [`ImageSlots`].
//! Both are pure traversals; the caller supplies the loading and the mermaid
//! rendering as closures, so this stays free of I/O and of merman.
//!
//! [`lone_image`] is not a pass but a question the renderer asks of a finished
//! paragraph: is this image standalone, and so free to take the pane's width?

use std::collections::HashMap;

use super::{MdBlock, MdSpan};
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

/// Slots handed out by the resolve passes, and the rasters behind them.
///
/// Both passes number into one table because the renderer looks a span's
/// `slot` up in a single per-pane list. Equal image urls share a slot, so a
/// url repeated in one document is decoded once.
#[derive(Default)]
pub(super) struct ImageSlots {
    rasters: Vec<Option<RasterImage>>,
    by_url: HashMap<String, u32>,
}

impl ImageSlots {
    /// Slot for `url`, loading it through `resolve` the first time it is seen.
    /// Sharing a slot between equal urls requires `resolve` to be a pure
    /// function of the url it is handed.
    fn for_url(&mut self, url: &str, resolve: &mut dyn FnMut(&str) -> Option<RasterImage>) -> u32 {
        if let Some(slot) = self.by_url.get(url) {
            return *slot;
        }
        let slot = self.push(resolve(url));
        self.by_url.insert(url.to_owned(), slot);
        slot
    }

    fn push(&mut self, raster: Option<RasterImage>) -> u32 {
        // `FILE_VIEWER_MAX_BYTES` caps the document before it is parsed, so
        // the slot count cannot approach `u32::MAX`.
        let slot = self.rasters.len() as u32;
        self.rasters.push(raster);
        slot
    }

    /// The collected rasters, one per slot, for conversion into GPU images.
    pub(super) fn into_rasters(self) -> Vec<Option<RasterImage>> {
        self.rasters
    }
}

/// Number and load every image and diagram in `blocks`, returning one raster
/// per slot for [`MdImages`](crate::workspace::main_area::file_view_pane::images::MdImages).
///
/// The only way to run the passes: they must share one slot space, because the
/// renderer looks an image span and a diagram block up in the same per-pane
/// table. Running one alone would leave the other kind holding a slot that
/// indexes someone else's image.
pub(in crate::workspace) fn resolve_all(
    blocks: &mut [MdBlock],
    load_image: &mut dyn FnMut(&str) -> Option<RasterImage>,
    render_mermaid: &mut dyn FnMut(&str) -> Option<RasterImage>,
) -> Vec<Option<RasterImage>> {
    let mut slots = ImageSlots::default();
    resolve_images(blocks, &mut slots, load_image);
    resolve_mermaid(blocks, &mut slots, render_mermaid);
    slots.into_rasters()
}

/// Walk every image span in `blocks` (recursing into nested spans and list
/// item children), stamp its `slot` and load its pixels through `resolve`.
/// Pure traversal — the caller supplies the I/O (file read + decode) as the
/// closure.
pub(super) fn resolve_images(
    blocks: &mut [MdBlock],
    slots: &mut ImageSlots,
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Heading { spans, .. }
            | MdBlock::Paragraph(spans)
            | MdBlock::Blockquote(spans)
            | MdBlock::FootnoteDefinition { spans, .. } => {
                resolve_images_in_spans(spans, slots, resolve);
            }
            MdBlock::BulletList { items, .. } | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_images(&mut item.blocks, slots, resolve);
                }
            }
            MdBlock::Table { header, rows } => {
                for cell in header {
                    resolve_images_in_spans(cell, slots, resolve);
                }
                for row in rows {
                    for cell in row {
                        resolve_images_in_spans(cell, slots, resolve);
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
/// `(alt, slot)`. Such a paragraph renders the image block-style (large);
/// otherwise images render inline, sized to the text line.
pub(in crate::workspace) fn lone_image(spans: &[MdSpan]) -> Option<(&str, u32)> {
    let mut found: Option<(&str, u32)> = None;
    for span in spans {
        match span {
            MdSpan::Image { alt, slot, url: _ } => {
                if found.is_some() {
                    return None;
                }
                found = Some((alt.as_str(), *slot));
            }
            MdSpan::SoftBreak | MdSpan::HardBreak | MdSpan::ParagraphBreak => {}
            MdSpan::Text(t) if t.trim().is_empty() => {}
            _ => return None,
        }
    }
    found
}

/// Walk every mermaid block in `blocks` (recursing into list item children),
/// stamp its `slot` and render its pixels through `resolve`. Pure traversal —
/// the caller supplies the rendering (merman → SVG → rasterize) as the
/// closure.
pub(super) fn resolve_mermaid(
    blocks: &mut [MdBlock],
    slots: &mut ImageSlots,
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for block in blocks {
        match block {
            MdBlock::Mermaid { source, slot } => {
                *slot = slots.push(resolve(source));
            }
            MdBlock::BulletList { items, .. } | MdBlock::OrderedList { items, .. } => {
                for item in items {
                    resolve_mermaid(&mut item.blocks, slots, resolve);
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
    slots: &mut ImageSlots,
    resolve: &mut dyn FnMut(&str) -> Option<RasterImage>,
) {
    for span in spans {
        match span {
            MdSpan::Image { url, slot, alt: _ } => {
                *slot = slots.for_url(url, resolve);
            }
            MdSpan::Bold(inner)
            | MdSpan::Italic(inner)
            | MdSpan::Link {
                children: inner, ..
            }
            | MdSpan::Strikethrough(inner) => {
                resolve_images_in_spans(inner, slots, resolve);
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
