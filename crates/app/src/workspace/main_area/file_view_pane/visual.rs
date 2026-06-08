//! Shared visual rasterization for the markdown preview: turns an SVG string
//! (from a mermaid renderer, or any source) into an in-memory RGBA bitmap.
//!
//! GPUI-free: produces plain [`RasterImage`] data. Wrapping into a GPUI
//! `RenderImage` happens at the render boundary, not here.

use std::path::Path;
use std::sync::Arc;

/// Render diagrams/images at 2× their intrinsic size so they stay crisp when
/// the GPUI render layer scales them to fit the pane on HiDPI displays.
const RASTER_SCALE: f32 = 2.0;

/// Upper bound on either bitmap dimension; guards against a pathological SVG
/// requesting a multi-gigabyte pixmap.
const MAX_DIM: u32 = 8000;

/// Decoded bitmap in RGBA8888, row-major top-to-bottom (`width * height * 4`
/// bytes). The render boundary converts to GPUI's BGRA `RenderImage` format;
/// alpha handling is normalized there (verified against live rendering).
#[derive(Clone)]
pub(in crate::workspace) struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Rasterize an SVG document to a [`RasterImage`] at [`RASTER_SCALE`]× its
/// intrinsic size, so the bitmap stays crisp when painted on HiDPI displays.
pub(in crate::workspace) fn rasterize_svg(svg: &str) -> anyhow::Result<RasterImage> {
    let mut opt = resvg::usvg::Options::default();
    let mut fontdb = resvg::usvg::fontdb::Database::new();
    fontdb.load_system_fonts();
    opt.fontdb = Arc::new(fontdb);

    let tree = resvg::usvg::Tree::from_str(svg, &opt)?;
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

    let mut rgba = pixmap.data().to_vec();
    unpremultiply(&mut rgba);
    Ok(RasterImage {
        width,
        height,
        rgba,
    })
}

/// Convert premultiplied-alpha RGBA (tiny-skia's pixmap format) to straight
/// alpha, matching the straight-alpha contract shared with `decode_image`.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
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

/// Resolve a markdown image reference to its encoded bytes.
///
/// Policy (locked): only local files (resolved relative to `base_dir`) and
/// `data:` URIs are loaded. Remote `http(s)` references are refused — a local
/// file viewer must not fetch from the network.
pub(in crate::workspace) fn load_image_source(
    url: &str,
    base_dir: &Path,
) -> anyhow::Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, payload) = rest
            .split_once(',')
            .ok_or_else(|| anyhow::anyhow!("malformed data URI"))?;
        if meta.contains("base64") {
            use base64::Engine;
            Ok(base64::engine::general_purpose::STANDARD.decode(payload)?)
        } else {
            Ok(payload.as_bytes().to_vec())
        }
    } else if url.starts_with("http://") || url.starts_with("https://") {
        anyhow::bail!("remote images are not fetched: {url}")
    } else {
        Ok(std::fs::read(base_dir.join(url))?)
    }
}

/// Decode an encoded raster image (PNG/JPEG/GIF/WebP/BMP) into a
/// [`RasterImage`].
pub(in crate::workspace) fn decode_image(bytes: &[u8]) -> anyhow::Result<RasterImage> {
    let img = image::load_from_memory(bytes)?.to_rgba8();
    let (width, height) = img.dimensions();
    Ok(RasterImage {
        width,
        height,
        rgba: img.into_raw(),
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
        // 10×20 SVG, RASTER_SCALE = 2 → 20×40 px, RGBA = 20*40*4 bytes.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#000000"/></svg>"##;
        let img = rasterize_svg(svg).expect("rasterize should succeed");
        assert_eq!((img.width, img.height), (20, 40));
        assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
    }

    #[test]
    fn rejects_non_svg_input() {
        assert!(rasterize_svg("this is not svg at all").is_err());
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
        assert_eq!(img.rgba.len(), (3 * 2 * 4) as usize);
    }

    #[test]
    fn refuses_remote_image_urls() {
        let dir = std::env::temp_dir();
        assert!(load_image_source("https://example.com/a.png", &dir).is_err());
        assert!(load_image_source("http://example.com/a.png", &dir).is_err());
    }

    #[test]
    fn reads_local_image_relative_to_base_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let png = encode_png(2, 2, [1, 2, 3, 255]);
        std::fs::write(dir.path().join("pic.png"), &png).expect("write png");
        let bytes = load_image_source("pic.png", dir.path()).expect("load local");
        assert_eq!(bytes, png);
    }

    #[test]
    fn decodes_base64_data_uri() {
        use base64::Engine;
        let png = encode_png(2, 2, [4, 5, 6, 255]);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let uri = format!("data:image/png;base64,{b64}");
        let bytes = load_image_source(&uri, &std::env::temp_dir()).expect("load data uri");
        assert_eq!(bytes, png);
    }
}
