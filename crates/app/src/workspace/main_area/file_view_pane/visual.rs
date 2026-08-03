//! Shared visual rasterization for the markdown preview: turns an SVG string
//! (from a mermaid renderer, or any source) into an in-memory RGBA bitmap.
//!
//! GPUI-free: produces plain [`RasterImage`] data. Wrapping into a GPUI
//! `RenderImage` happens at the render boundary, not here.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

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
pub(in crate::workspace) fn rasterize_svg(svg: &str) -> anyhow::Result<RasterImage> {
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

    let mut rgba = pixmap.data().to_vec();
    unpremultiply(&mut rgba);
    Ok(RasterImage {
        width,
        height,
        rgba,
        scale: RASTER_SCALE,
    })
}

/// Parse options for every SVG daruda reads, so a document is measured and
/// rasterized through exactly one text stack. Anything that inspects geometry
/// before rasterizing (see `mermaid_label_geometry`) must parse with these too,
/// or it measures a different font than the one that gets painted.
pub(in crate::workspace) fn usvg_options() -> resvg::usvg::Options<'static> {
    let mut options = resvg::usvg::Options {
        fontdb: shared_fontdb(),
        ..Default::default()
    };
    options.font_resolver.select_font = case_insensitive_font_selector();
    options
}

/// Resolve `font-family` names the way CSS does: case-insensitively.
///
/// `fontdb` compares family names byte-for-byte, and mermaid emits its stack
/// lowercased (`font-family:"trebuchet ms",verdana,arial,sans-serif`), so every
/// named family missed and text fell through to the generic `sans-serif` face.
/// Diagrams rendered in Arial while merman had sized every label from its
/// Trebuchet MS metrics — up to 14px of disagreement per label, which shows up
/// as text overflowing node boxes and label bands.
fn case_insensitive_font_selector() -> resvg::usvg::FontSelectionFn<'static> {
    use resvg::usvg::fontdb;

    Box::new(|font, db| {
        // `fontdb::Family::Name` borrows, so the canonical spellings have to
        // outlive the query built from them.
        let canonical: Vec<String> = font
            .families()
            .iter()
            .filter_map(|family| match family {
                resvg::usvg::FontFamily::Named(name) => Some(
                    canonical_family_names()
                        .get(&name.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| name.clone()),
                ),
                _ => None,
            })
            .collect();
        let mut named = canonical.iter();
        let mut families: Vec<fontdb::Family> = font
            .families()
            .iter()
            .map(|family| match family {
                resvg::usvg::FontFamily::Serif => fontdb::Family::Serif,
                resvg::usvg::FontFamily::SansSerif => fontdb::Family::SansSerif,
                resvg::usvg::FontFamily::Cursive => fontdb::Family::Cursive,
                resvg::usvg::FontFamily::Fantasy => fontdb::Family::Fantasy,
                resvg::usvg::FontFamily::Monospace => fontdb::Family::Monospace,
                resvg::usvg::FontFamily::Named(_) => {
                    fontdb::Family::Name(named.next().map_or("", std::string::String::as_str))
                }
            })
            .collect();
        // Same last-resort family usvg's own selector appends.
        families.push(fontdb::Family::Serif);

        db.query(&fontdb::Query {
            families: &families,
            weight: fontdb::Weight(font.weight()),
            stretch: match font.stretch() {
                resvg::usvg::FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
                resvg::usvg::FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
                resvg::usvg::FontStretch::Condensed => fontdb::Stretch::Condensed,
                resvg::usvg::FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
                resvg::usvg::FontStretch::Normal => fontdb::Stretch::Normal,
                resvg::usvg::FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
                resvg::usvg::FontStretch::Expanded => fontdb::Stretch::Expanded,
                resvg::usvg::FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
                resvg::usvg::FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
            },
            style: match font.style() {
                resvg::usvg::FontStyle::Normal => fontdb::Style::Normal,
                resvg::usvg::FontStyle::Italic => fontdb::Style::Italic,
                resvg::usvg::FontStyle::Oblique => fontdb::Style::Oblique,
            },
        })
    })
}

/// Lowercased family name → the spelling the installed face declares. Built
/// once from the shared database; first declaration wins so a later face can't
/// shadow the canonical name.
fn canonical_family_names() -> &'static HashMap<String, String> {
    static NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = HashMap::new();
        for face in shared_fontdb().faces() {
            for (family, _) in &face.families {
                names
                    .entry(family.to_lowercase())
                    .or_insert_with(|| family.clone());
            }
        }
        names
    })
}

/// System font database, loaded once and shared across all `rasterize_svg`
/// calls. `load_system_fonts` scans and parses every installed face, so doing
/// it per render would dominate the cost of drawing a diagram.
fn shared_fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTDB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTDB
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
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

/// Render a ` ```mermaid ` source to a [`RasterImage`]: merman SVG → label
/// background alignment → raster.
///
/// The single funnel for both mermaid surfaces (file viewer and agent chat), so
/// they cannot drift apart on theme options, panic containment, or the label
/// geometry correction.
pub(in crate::workspace) fn render_mermaid_raster(
    source: &str,
    palette: &super::mermaid_theme::MermaidPalette,
) -> Option<RasterImage> {
    let svg = render_mermaid_svg(source, palette)?;
    // Contain a panic across the whole post-render pipeline, not just the
    // renderer: one bad diagram must not fail a file load or take the
    // background executor down.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let svg = super::mermaid_label_geometry::align_label_backgrounds(&svg, &usvg_options());
        rasterize_svg(&svg).ok()
    }))
    .ok()
    .flatten()
}

/// merman's SVG for `source`, with every option the app renders diagrams under.
/// Split out so anything inspecting the diagram (tests, the label geometry pass)
/// sees exactly what ships.
pub(in crate::workspace) fn render_mermaid_svg(
    source: &str,
    palette: &super::mermaid_theme::MermaidPalette,
) -> Option<String> {
    use super::markdown_viewer::{
        mermaid_host_theme_profile, mermaid_svg_render_options, source_has_own_theme_directive,
    };

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut renderer = merman::render::HeadlessRenderer::new()
            .with_svg_options(mermaid_svg_render_options())
            // Layout has to reserve the width this app's own text stack paints,
            // not what merman's Trebuchet MS table estimates for Hangul.
            .with_text_measurer(super::mermaid_text_measurer::host_text_measurer());
        // Match the diagram theme to the host appearance so every diagram type —
        // not just flowchart nodes — stays legible (dark UI → dark chrome).
        if !source_has_own_theme_directive(source) {
            renderer = renderer.with_host_theme(&mermaid_host_theme_profile(palette));
        }
        renderer.render_svg_sync(source).ok().flatten()
    }))
    .ok()
    .flatten()
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
        scale: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ink width of `text` laid out in `family`, through the app's own options.
    fn ink_width(text: &str, family: &str) -> f32 {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4000" height="200"><style>text{{font-family:{family};font-size:16px}}</style><text x="0" y="100">{text}</text></svg>"#
        );
        let tree = resvg::usvg::Tree::from_str(&svg, &usvg_options()).expect("parses");
        fn find(group: &resvg::usvg::Group) -> Option<f32> {
            for node in group.children() {
                match node {
                    resvg::usvg::Node::Text(text) => return Some(text.abs_bounding_box().width()),
                    resvg::usvg::Node::Group(inner) => {
                        if let Some(width) = find(inner) {
                            return Some(width);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        find(tree.root()).expect("laid out text")
    }

    /// CSS family matching is case-insensitive. Pick an installed family whose
    /// canonical spelling actually changes the layout, and check that asking for
    /// it in lowercase — the way mermaid's CSS does — lands on the same face.
    #[test]
    fn a_lowercase_font_family_resolves_like_its_canonical_spelling() {
        const PROBE: &str = "Handgloves 123";
        let fallback = ink_width(PROBE, "\"daruda-no-such-font\"");
        let distinctive = canonical_family_names().values().find(|name| {
            name.is_ascii()
                && name.chars().any(char::is_uppercase)
                && (ink_width(PROBE, &format!("\"{name}\"")) - fallback).abs() > 0.5
        });
        let Some(canonical) = distinctive else {
            // No installed family both differs from the fallback and has a case
            // to normalize, so there is nothing this test can prove here.
            return;
        };
        assert_eq!(
            ink_width(PROBE, &format!("\"{}\"", canonical.to_lowercase())),
            ink_width(PROBE, &format!("\"{canonical}\"")),
            "lowercase {canonical:?} must resolve to the same face"
        );
    }

    /// The case the fix exists for: mermaid's own stack, verbatim. Without
    /// case-insensitive resolution every name misses and text lands on the
    /// generic `sans-serif` face, disagreeing with merman's Trebuchet MS
    /// metrics by up to 14px per label.
    #[test]
    fn the_mermaid_font_stack_resolves_to_trebuchet_ms() {
        if !canonical_family_names().contains_key("trebuchet ms") {
            // Trebuchet MS ships with macOS (the verified target) but not with
            // every Linux image; nothing to assert without it.
            return;
        }
        const PROBE: &str = "schedule tick";
        assert!(
            (ink_width(PROBE, r#""trebuchet ms",verdana,arial,sans-serif"#)
                - ink_width(PROBE, r#""Trebuchet MS""#))
            .abs()
                < 0.05,
            "mermaid's lowercased stack must land on Trebuchet MS"
        );
    }

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
    fn shared_fontdb_is_loaded_once() {
        let a = shared_fontdb();
        let b = shared_fontdb();
        assert!(
            Arc::ptr_eq(&a, &b),
            "repeated calls must return the same cached database"
        );
    }

    #[test]
    fn logical_size_divides_pixels_by_scale() {
        let hidpi = RasterImage {
            width: 40,
            height: 80,
            rgba: vec![],
            scale: 2.0,
        };
        assert_eq!(hidpi.logical_size(), (20.0, 40.0));

        let native = RasterImage {
            width: 30,
            height: 30,
            rgba: vec![],
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
