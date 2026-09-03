//! Markdown images: how they are sized, and the raster → GPU conversion both
//! the file viewer's load pass and agent chat reuse.

use gpui::{AnyElement, ImageSource, IntoElement, RenderImage, div, img, prelude::*, px};

use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::main_area::file_view_pane::visual::RasterImage;

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
            .child(strings::file_viewer_image_alt(alt))
            .into_any_element();
    };
    match layout {
        // Block-sized, height-capped: decorative images only.
        ImageLayout::Block => image.block(),
        // Sized to the text line; gpui derives width from the aspect ratio.
        ImageLayout::Inline => image.inline(),
    }
}

/// Rasterized image converted once so GPUI can reuse the same texture id.
/// Both hosts hold one of these per image — the file viewer in `MdImages`,
/// agent chat in its own tables — because rebuilding per render re-uploads the
/// texture. Exactly one table owns each instance: the owner releases its
/// sprite-atlas tile, so a clone that outlives that table paints a freed tile.
#[derive(Clone)]
pub(in crate::workspace) struct CachedImage {
    image: std::sync::Arc<RenderImage>,
    logical_w: f32,
}

impl CachedImage {
    /// Wrap a raster into a cached GPU image, moving its buffer in. The
    /// producer (`visual.rs`) already emits GPUI's byte order, so there is
    /// nothing to copy or swap here.
    pub(in crate::workspace) fn from_raster(raster: RasterImage) -> Option<Self> {
        let (logical_w, _) = raster.logical_size();
        let buffer = image::RgbaImage::from_raw(raster.width, raster.height, raster.bgra)?;
        let image = std::sync::Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
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

    /// Inline-layout element sized to the text line: height fixed, width
    /// derived from the image's own aspect ratio so it flows with the text.
    pub(in crate::workspace) fn inline(&self) -> AnyElement {
        img(ImageSource::Render(self.image.clone()))
            .h(px(theme::MD_INLINE_IMAGE_HEIGHT))
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

    /// The atlas key the upload was made under, for the owning table's
    /// release path only — see the single-owner rule on [`CachedImage`].
    pub(in crate::workspace::main_area::file_view_pane) fn render_image(
        &self,
    ) -> std::sync::Arc<RenderImage> {
        self.image.clone()
    }
}
