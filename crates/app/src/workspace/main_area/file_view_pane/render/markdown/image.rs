//! Markdown images: how they are sized, and the one-time raster → GPU
//! conversion agent chat reuses.

use gpui::{AnyElement, ImageSource, IntoElement, RenderImage, div, img, prelude::*, px};

use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

use super::MdColors;

/// How a markdown image is sized.
#[derive(Clone, Copy)]
pub(super) enum ImageLayout {
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
pub(super) fn render_md_image(
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
fn raster_block_image(raster: &RasterImage) -> Option<AnyElement> {
    Some(CachedImage::from_raster(raster)?.block())
}

/// Diagram-layout element for a raster, converting fresh for Markdown
/// preview. Agent chat caches [`CachedImage`] instead.
fn raster_diagram_image(raster: &RasterImage) -> Option<AnyElement> {
    Some(CachedImage::from_raster(raster)?.block_diagram())
}
