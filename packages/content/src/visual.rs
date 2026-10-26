//! Shared visual rasterization for the markdown preview: turns an SVG string
//! (from a mermaid renderer, or any source) into an in-memory BGRA bitmap
//! already in GPUI's byte order.
//!
//! GPUI-free: produces plain [`RasterImage`] data. Wrapping the buffer into a
//! GPUI `RenderImage` happens at the render boundary, not here.

pub(super) use crate::svg_fonts::usvg_options;
use std::io::Cursor;

/// Render diagrams/images at 2× their intrinsic size so they stay crisp when
/// the GPUI render layer scales them to fit the pane on HiDPI displays.
const RASTER_SCALE: f32 = 2.0;

/// Upper bound on either bitmap dimension; guards against a pathological SVG
/// requesting a multi-gigabyte pixmap.
const MAX_DIM: u32 = 8000;

/// Decoded bitmap in BGRA8888, row-major top-to-bottom (`width * height * 4`
/// bytes) — already in GPUI's `RenderImage` byte order, so the render
/// boundary moves this buffer into a `RenderImage` without copying it.
#[derive(Clone)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Device-pixel scale the bitmap was rendered at: 2.0 for HiDPI-crisp SVG
    /// raster, 1.0 for a natively-sized decoded image. The render layer divides
    /// by this to get the logical (point) display size, so a 2× bitmap shows at
    /// its natural size while staying crisp.
    pub scale: f32,
}

impl RasterImage {
    /// Logical (display) size in points = device pixels / [`scale`](Self::scale).
    pub fn logical_size(&self) -> (f32, f32) {
        (
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        )
    }
}

/// Rasterize an SVG document to a [`RasterImage`] at [`RASTER_SCALE`]× its
/// intrinsic size, so the bitmap stays crisp when painted on HiDPI displays.
pub(super) fn rasterize_svg(svg: &str) -> anyhow::Result<RasterImage> {
    let tree = resvg::usvg::Tree::from_str(svg, &usvg_options())?;
    let size = tree.size();
    let width = ((size.width() * RASTER_SCALE).ceil() as u32).clamp(1, MAX_DIM);
    let height = ((size.height() * RASTER_SCALE).ceil() as u32).clamp(1, MAX_DIM);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow::anyhow!("pixmap allocation {width}x{height} failed"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(RASTER_SCALE, RASTER_SCALE),
        &mut pixmap.as_mut(),
    );

    let mut pixels = pixmap.data().to_vec();
    // Straight-alpha conversion reads/writes R and B uniformly, so it does
    // not matter whether it runs before or after the channel swap.
    unpremultiply(&mut pixels);
    swap_rb_in_place(&mut pixels);
    Ok(RasterImage {
        width,
        height,
        bgra: pixels,
        scale: RASTER_SCALE,
    })
}

/// Convert premultiplied-alpha RGBA (tiny-skia's pixmap format) to straight
/// alpha, matching the straight-alpha contract shared with `decode_image`.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        match a {
            0 => {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
            }
            255 => {}
            a => {
                let a = u16::from(a);
                for c in &mut px[..3] {
                    *c = ((u16::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
}

/// Swap the R and B channels of a BGRA/RGBA8888 buffer in place, converting
/// between the two. Shared by every [`RasterImage`] producer so the swap runs
/// once, on the background thread that decodes or rasterizes the bitmap.
fn swap_rb_in_place(bytes: &mut [u8]) {
    for px in bytes.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
}

/// Render a ` ```mermaid ` source to a [`RasterImage`]: merman SVG → label
/// background alignment → raster.
///
/// The single funnel for both mermaid surfaces (file viewer and agent chat), so
/// they cannot drift apart on theme options, panic containment, or the label
/// geometry correction.
pub fn render_mermaid_raster(source: &str, palette: &crate::MermaidPalette) -> Option<RasterImage> {
    let svg = render_mermaid_svg(source, palette)?;
    // Contain a panic across the whole post-render pipeline, not just the
    // renderer: one bad diagram must not fail a file load or take the
    // background executor down.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let options = usvg_options();
        let svg = super::mermaid_node_contrast::force_node_label_contrast(&svg, &options);
        let svg = super::mermaid_label_stroke::suppress_label_text_stroke(&svg);
        let svg = super::mermaid_label_geometry::align_label_backgrounds(&svg, &options);
        rasterize_svg(&svg).ok()
    }))
    .ok()
    .flatten()
}

/// merman's SVG for `source`, with every option the app renders diagrams under.
/// Split out so anything inspecting the diagram (tests, the label geometry pass)
/// sees exactly what ships.
pub(super) fn render_mermaid_svg(source: &str, palette: &crate::MermaidPalette) -> Option<String> {
    use super::mermaid_host_theme::{mermaid_render_profile, mermaid_svg_render_options};

    // Fix up classDef/style/rect declarations merman-render would otherwise
    // leave contrast-unsafe (a light author-declared fill under daruda's
    // forced dark-mode text, or vice versa) before the source ever reaches
    // merman — see the module doc for which gaps this closes and why.
    let source = super::mermaid_contrast::ensure_text_contrast(source);
    let source = source.as_ref();

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Unconditional: the profile decides the diagram's colours, but it also
        // carries the settings resvg needs to paint any label at all.
        let profile = mermaid_render_profile(source, palette);
        merman::render::HeadlessRenderer::new()
            .with_svg_options(mermaid_svg_render_options())
            // Layout has to reserve the width this app's own text stack paints,
            // not what merman's Trebuchet MS table estimates for Hangul.
            .with_text_measurer(super::mermaid_text_measurer::host_text_measurer())
            .with_host_theme(&profile)
            .render_svg_sync(source)
            .ok()
            .flatten()
    }))
    .ok()
    .flatten()
}

/// Decode an encoded raster image (PNG/JPEG/GIF/WebP/BMP) into a
/// [`RasterImage`].
pub fn decode_image(bytes: &[u8]) -> anyhow::Result<RasterImage> {
    let img = image::load_from_memory(bytes)?.to_rgba8();
    let (width, height) = img.dimensions();
    let mut bgra = img.into_raw();
    swap_rb_in_place(&mut bgra);
    Ok(RasterImage {
        width,
        height,
        bgra,
        scale: 1.0,
    })
}

/// Decode untrusted image bytes with caller-selected allocation and dimension
/// limits. Kept separate from [`decode_image`] so a host-specific safety policy
/// does not silently narrow existing Markdown and file-viewer behavior.
pub fn decode_image_bounded(
    bytes: &[u8],
    max_dimension: u32,
    max_alloc: u64,
) -> anyhow::Result<RasterImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dimension);
    limits.max_image_height = Some(max_dimension);
    limits.max_alloc = Some(max_alloc);
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    let img = reader.decode()?.to_rgba8();
    let (width, height) = img.dimensions();
    let mut bgra = img.into_raw();
    swap_rb_in_place(&mut bgra);
    Ok(RasterImage {
        width,
        height,
        bgra,
        scale: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_png(width: u32, height: u32, px: [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(width, height);
        for p in img.pixels_mut() {
            *p = image::Rgba(px);
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .expect("encode png");
        buf.into_inner()
    }

    #[test]
    fn rasterizes_svg_at_scaled_dimensions() {
        // 10×20 SVG, RASTER_SCALE = 2 → 20×40 px, BGRA = 20*40*4 bytes.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#000000"/></svg>"##;
        let img = rasterize_svg(svg).expect("rasterize should succeed");
        assert_eq!((img.width, img.height), (20, 40));
        assert_eq!(img.bgra.len(), (img.width * img.height * 4) as usize);
    }

    #[test]
    fn rejects_non_svg_input() {
        assert!(rasterize_svg("this is not svg at all").is_err());
    }

    /// Regression guard: `rasterize_svg` must hand back GPUI's BGRA byte
    /// order, not RGBA. A red fill (R≠B) makes a silent channel swap visible.
    #[test]
    fn rasterize_svg_produces_bgra_byte_order() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#ff0000"/></svg>"##;
        let img = rasterize_svg(svg).expect("rasterize should succeed");
        assert_eq!(
            &img.bgra[..4],
            &[0, 0, 255, 255],
            "opaque red must land as B=0, G=0, R=255 in BGRA order"
        );
    }

    #[test]
    fn logical_size_divides_pixels_by_scale() {
        let hidpi = RasterImage {
            width: 40,
            height: 80,
            bgra: vec![],
            scale: 2.0,
        };
        assert_eq!(hidpi.logical_size(), (20.0, 40.0));

        let native = RasterImage {
            width: 30,
            height: 30,
            bgra: vec![],
            scale: 1.0,
        };
        assert_eq!(native.logical_size(), (30.0, 30.0));
    }

    #[test]
    fn unpremultiply_restores_straight_alpha() {
        let mut opaque = [10u8, 20, 30, 255];
        unpremultiply(&mut opaque);
        assert_eq!(opaque, [10, 20, 30, 255], "opaque pixels unchanged");

        let mut clear = [9u8, 9, 9, 0];
        unpremultiply(&mut clear);
        assert_eq!(clear, [0, 0, 0, 0], "fully transparent pixels zeroed");

        let mut semi = [64u8, 0, 0, 128];
        unpremultiply(&mut semi);
        assert!(
            semi[0] > 64 && semi[3] == 128,
            "semi-transparent channel scaled up, alpha preserved: {semi:?}"
        );
    }

    #[test]
    fn decodes_png_to_expected_dimensions() {
        let png = encode_png(3, 2, [10, 20, 30, 255]);
        let img = decode_image(&png).expect("decode should succeed");
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(img.bgra.len(), (3 * 2 * 4) as usize);
        // Encoded RGBA 10,20,30 must come back as BGRA.
        assert_eq!(&img.bgra[..4], &[30, 20, 10, 255]);
    }

    #[test]
    fn bounded_decode_does_not_narrow_the_existing_decoder() {
        let png = encode_png(3, 2, [10, 20, 30, 255]);
        assert!(decode_image_bounded(&png, 2, 1024).is_err());
        assert_eq!(decode_image(&png).unwrap().width, 3);
    }
}
