//! Markdown images: how they are sized, and the alt-text fallback. The
//! raster → GPU conversion both hosts reuse is `crate::ui::CachedImage`.

use gpui::{AnyElement, IntoElement, div, prelude::*};

use crate::surface::strings;
use crate::ui::CachedImage;

use super::MdColors;

/// How a markdown image is sized.
#[derive(Clone, Copy)]
pub(super) enum ImageLayout {
    /// Standalone decorative image (photo/screenshot): fits the pane width,
    /// height capped so one large embedded photo can't dominate the document.
    Block,
    /// Image embedded in a text run: sized to the line so it flows with text.
    Inline,
}

/// Render an image the load pass already converted for the GPU, or fall back
/// to `[alt]` text when there is none (remote/missing/decode-failed).
/// `object_fit` defaults to `Contain`, preserving aspect ratio; gpui derives
/// the unset dimension from it.
pub(super) fn render_md_image(
    image: Option<&CachedImage>,
    alt: &str,
    layout: ImageLayout,
    t: &MdColors,
) -> AnyElement {
    let Some(image) = image else {
        return div()
            .text_color(t.subtle)
            .child(strings::file_viewer::image_alt(alt))
            .into_any_element();
    };
    match layout {
        // Block-sized, height-capped: decorative images only.
        ImageLayout::Block => image.block(),
        // Sized to the text line; gpui derives width from the aspect ratio.
        ImageLayout::Inline => image.inline(),
    }
}
