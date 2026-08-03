//! Snaps every mermaid edge-label `.background` band onto the glyphs the
//! rasterizer will actually paint.
//!
//! Mermaid builds these labels in two steps: `createFormattedText` emits a
//! `<rect class="background">` plus a `<text y="-10.1">` whose first baseline
//! sits `1.1em` below the text origin, and then — in the browser — reads the
//! rendered text's `getBBox()` and moves the rect onto it. merman has no DOM
//! for that second step, so it emits the markup verbatim and guesses the rect
//! from vendored font metrics. The guess and the glyphs disagree:
//!
//! - the state renderer puts the text origin at the rect *center*, double
//!   counting the `dy="1.1em"` baseline offset (glyphs land ~11px low),
//! - the flowchart/class renderers keep Mermaid's left-anchored rect while
//!   emitting `text-anchor="middle"` text (glyphs land half a label width
//!   left, far enough to be clipped out of the `viewBox`),
//! - a band can still come out narrower than its glyphs whenever merman's
//!   metrics disagree with the resolved face (see `mermaid_text_measurer` for
//!   the Hangul case), so the band is grown to cover them when that happens.
//!
//! This module supplies the missing measurement pass using the same engine
//! that rasterizes (usvg), so the band is centered on its glyphs and never
//! smaller than them.
//!
//! WORKAROUND: the root cause is upstream in merman's `svg/parity/{state,
//! flowchart,class}` label emitters — outside daruda's tree, so the correction
//! lands here. It is self-deactivating: once upstream derives the rect from a
//! measured box, every delta computed here rounds to zero and the rewrite is a
//! no-op.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use resvg::usvg;

/// Padding Mermaid's `createText(..., addSvgBackground = true)` leaves around
/// the measured text box when it snaps the `.background` rect onto it.
const BACKGROUND_PADDING: f32 = 2.0;

/// Corrections under this are invisible even at 2× raster scale; skipping them
/// leaves the SVG byte-identical wherever merman's geometry already agrees.
const NEGLIGIBLE_PX: f32 = 0.05;

/// Attribute prefix for the ids injected to pair a band with its text across
/// the measuring parse. Not present in merman output, so it cannot collide.
const PROBE_PREFIX: &str = "daruda-mermaid-label";

/// Re-center and (if needed) grow every mermaid edge-label background band so
/// it wraps the text usvg will paint. Returns `svg` unchanged when there is
/// nothing to correct, when a label cannot be measured, or when the document
/// does not parse.
pub(in crate::workspace) fn align_label_backgrounds(
    svg: &str,
    options: &usvg::Options<'_>,
) -> String {
    let labels = scan_labels(svg);
    if labels.is_empty() {
        return svg.to_owned();
    }
    let probed = inject_probe_ids(svg, &labels);
    let Some(measured) = measure(&probed, options) else {
        return svg.to_owned();
    };
    rewrite(svg, &labels, &measured)
}

/// A `.background` rect and the `<text>` it is meant to wrap.
struct Label {
    /// Byte range of the `<rect class="background" … />` tag.
    rect_tag: Range<usize>,
    /// The rect's own `x`/`y`/`width`/`height`, before any ancestor transform.
    rect_local: Box2,
    /// Byte range of the `<text …>…</text>` element.
    text_element: Range<usize>,
}

/// An axis-aligned box. Mirrors the four SVG rect attributes so local (attribute)
/// and absolute (measured) geometry can be compared side by side.
#[derive(Clone, Copy, PartialEq)]
struct Box2 {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Box2 {
    fn center(self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn from_rect(rect: usvg::Rect) -> Self {
        Self {
            x: rect.x(),
            y: rect.y(),
            w: rect.width(),
            h: rect.height(),
        }
    }
}

/// Absolute geometry read back from the measuring parse, keyed by probe index.
struct Measured {
    backgrounds: HashMap<usize, Box2>,
    texts: HashMap<usize, Box2>,
}

/// Locate every `.background` rect paired with the `<text>` that follows it.
///
/// The pairing is strict: only `<g …>` opening tags may sit between the rect
/// and its text (the state renderer wraps the text in one; the others don't).
/// Anything else — a `</g>`, another element, a second background rect — means
/// the text belongs to a different label, and the candidate is dropped rather
/// than mis-paired.
fn scan_labels(svg: &str) -> Vec<Label> {
    const RECT_OPEN: &str = r#"<rect class="background""#;
    let mut labels = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = svg[from..].find(RECT_OPEN) {
        let rect_start = from + offset;
        from = rect_start + RECT_OPEN.len();
        let Some(rect_end) = tag_end(svg, rect_start) else {
            break;
        };
        let rect_tag = rect_start..rect_end;
        // An id of our own would be re-injected twice; merman emits none, but a
        // future one must not corrupt the document.
        if svg[rect_tag.clone()].contains(" id=") {
            continue;
        }
        let Some(rect_local) = parse_rect_box(&svg[rect_tag.clone()]) else {
            continue;
        };
        let Some(text_element) = text_after(svg, rect_end) else {
            continue;
        };
        if svg[text_element.clone()].contains(" id=") {
            continue;
        }
        from = text_element.end;
        labels.push(Label {
            rect_tag,
            rect_local,
            text_element,
        });
    }
    labels
}

/// End offset (exclusive) of the tag starting at `start`, including its `>`.
fn tag_end(svg: &str, start: usize) -> Option<usize> {
    svg[start..].find('>').map(|i| start + i + 1)
}

/// Byte range of the `<text …>…</text>` element reachable from `at` by stepping
/// over `<g …>` opening tags only. `None` when anything else intervenes.
fn text_after(svg: &str, at: usize) -> Option<Range<usize>> {
    let mut cursor = at;
    loop {
        let rest = &svg[cursor..];
        if rest.starts_with("<text") {
            let end = rest.find("</text>")? + "</text>".len();
            return Some(cursor..cursor + end);
        }
        if rest.starts_with("<g") {
            cursor = tag_end(svg, cursor)?;
            continue;
        }
        return None;
    }
}

/// Parse the four geometry attributes of a `<rect>` tag.
fn parse_rect_box(tag: &str) -> Option<Box2> {
    Some(Box2 {
        x: attr_f32(tag, "x")?,
        y: attr_f32(tag, "y")?,
        w: attr_f32(tag, "width")?,
        h: attr_f32(tag, "height")?,
    })
}

/// Read `name="<number>"` from a tag. Matches on ` name="` so `x` never
/// matches inside `stroke-width` or a `style` payload.
fn attr_f32(tag: &str, name: &str) -> Option<f32> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = start + tag[start..].find('"')?;
    tag[start..end].parse().ok()
}

/// Copy of `svg` with a probe id on every scanned rect and text, so the parsed
/// tree can be paired back to byte ranges without guessing at the structure.
fn inject_probe_ids(svg: &str, labels: &[Label]) -> String {
    let mut out = String::with_capacity(svg.len() + labels.len() * 64);
    let mut copied = 0usize;
    for (index, label) in labels.iter().enumerate() {
        for (span, kind) in [(&label.rect_tag, "bg"), (&label.text_element, "text")] {
            // Both spans start with an element name; the id goes right after it.
            let insert_at = span.start + element_name_len(&svg[span.start..]);
            out.push_str(&svg[copied..insert_at]);
            let _ = write!(out, r#" id="{PROBE_PREFIX}-{kind}-{index}""#);
            copied = insert_at;
        }
    }
    out.push_str(&svg[copied..]);
    out
}

/// Byte length of the leading `<name` of an element tag.
fn element_name_len(tag: &str) -> usize {
    tag.char_indices()
        .find(|(_, c)| c.is_whitespace() || *c == '>' || *c == '/')
        .map_or(tag.len(), |(offset, _)| offset)
}

/// Parse the probed document and read back absolute geometry for each probe.
///
/// `options` must be the same ones the rasterizer uses, or this measures a
/// different font than the one that gets painted.
///
/// Labels whose rect or text carries a non-translation ancestor transform are
/// dropped: the corrections below are plain translations in the outer space,
/// which only transfer verbatim into a translated inner space.
fn measure(probed: &str, options: &usvg::Options<'_>) -> Option<Measured> {
    let tree = usvg::Tree::from_str(probed, options).ok()?;
    let mut measured = Measured {
        backgrounds: HashMap::new(),
        texts: HashMap::new(),
    };
    collect(tree.root(), &mut measured);
    Some(measured)
}

fn collect(group: &usvg::Group, out: &mut Measured) {
    for node in group.children() {
        match node {
            usvg::Node::Path(path) => {
                if let Some(index) = probe_index(path.id(), "bg")
                    && is_translation_only(&path.abs_transform())
                {
                    out.backgrounds
                        .insert(index, Box2::from_rect(path.abs_bounding_box()));
                }
            }
            usvg::Node::Text(text) => {
                if let Some(index) = probe_index(text.id(), "text")
                    && is_translation_only(&text.abs_transform())
                {
                    out.texts
                        .insert(index, Box2::from_rect(text.abs_bounding_box()));
                }
            }
            usvg::Node::Group(inner) => collect(inner, out),
            usvg::Node::Image(_) => {}
        }
    }
}

fn probe_index(id: &str, kind: &str) -> Option<usize> {
    id.strip_prefix(&format!("{PROBE_PREFIX}-{kind}-"))?
        .parse()
        .ok()
}

fn is_translation_only(transform: &usvg::Transform) -> bool {
    let unit = |v: f32| (v - 1.0).abs() < f32::EPSILON;
    let zero = |v: f32| v.abs() < f32::EPSILON;
    unit(transform.sx) && unit(transform.sy) && zero(transform.kx) && zero(transform.ky)
}

/// The correction for one label: where its band belongs and how far its text
/// has to move to sit in the middle of it.
struct Correction {
    /// Band in the rect's own local space — same center as before, never
    /// smaller than the glyphs plus Mermaid's padding.
    rect_local: Box2,
    /// Translation to apply to the `<text>` element.
    text_shift: (f32, f32),
}

/// Center the band's glyphs in it, growing the band when the glyphs are wider
/// or taller than merman's estimate. The band's center is left alone: it is
/// where the layout reserved space for the label and where the edge line it
/// masks passes, so the text moves to the band, never the other way round.
fn correction(rect_local: Box2, rect_abs: Box2, text_abs: Box2) -> Option<Correction> {
    let (center_x, center_y) = rect_abs.center();
    let padded = 2.0 * BACKGROUND_PADDING;
    let w = rect_abs.w.max(text_abs.w + padded);
    let h = rect_abs.h.max(text_abs.h + padded);
    let (text_center_x, text_center_y) = text_abs.center();
    let text_shift = (center_x - text_center_x, center_y - text_center_y);
    let grown = Box2 {
        // Local and absolute differ by a pure translation, so a size change
        // plus the matching origin shift transfers directly.
        x: rect_local.x + (center_x - w / 2.0 - rect_abs.x),
        y: rect_local.y + (center_y - h / 2.0 - rect_abs.y),
        w,
        h,
    };
    let unchanged = box_eq(grown, rect_local)
        && text_shift.0.abs() < NEGLIGIBLE_PX
        && text_shift.1.abs() < NEGLIGIBLE_PX;
    (!unchanged).then_some(Correction {
        rect_local: grown,
        text_shift,
    })
}

fn box_eq(a: Box2, b: Box2) -> bool {
    let near = |l: f32, r: f32| (l - r).abs() < NEGLIGIBLE_PX;
    near(a.x, b.x) && near(a.y, b.y) && near(a.w, b.w) && near(a.h, b.h)
}

/// Apply every correction to the original document.
fn rewrite(svg: &str, labels: &[Label], measured: &Measured) -> String {
    let mut out = String::with_capacity(svg.len() + labels.len() * 48);
    let mut copied = 0usize;
    for (index, label) in labels.iter().enumerate() {
        let Some((rect_abs, text_abs)) = measured
            .backgrounds
            .get(&index)
            .zip(measured.texts.get(&index))
        else {
            continue;
        };
        let Some(fix) = correction(label.rect_local, *rect_abs, *text_abs) else {
            continue;
        };
        out.push_str(&svg[copied..label.rect_tag.start]);
        out.push_str(&resized_rect_tag(
            &svg[label.rect_tag.clone()],
            fix.rect_local,
        ));
        out.push_str(&svg[label.rect_tag.end..label.text_element.start]);
        let _ = write!(
            out,
            r#"<g transform="translate({}, {})">"#,
            round(fix.text_shift.0),
            round(fix.text_shift.1)
        );
        out.push_str(&svg[label.text_element.clone()]);
        out.push_str("</g>");
        copied = label.text_element.end;
    }
    out.push_str(&svg[copied..]);
    out
}

/// The same `<rect>` tag with its four geometry attributes replaced, so the
/// class/style merman put there (band fill, `stroke: none`) survives.
fn resized_rect_tag(tag: &str, geometry: Box2) -> String {
    let mut out = tag.to_owned();
    for (name, value) in [
        ("x", geometry.x),
        ("y", geometry.y),
        ("width", geometry.w),
        ("height", geometry.h),
    ] {
        out = replace_attr(&out, name, round(value));
    }
    out
}

fn replace_attr(tag: &str, name: &str, value: f64) -> String {
    let needle = format!(" {name}=\"");
    let Some(start) = tag.find(&needle).map(|i| i + needle.len()) else {
        return tag.to_owned();
    };
    let Some(end) = tag[start..].find('"').map(|i| start + i) else {
        return tag.to_owned();
    };
    format!("{}{value}{}", &tag[..start], &tag[end..])
}

/// Trim measurement noise so the emitted numbers stay short and stable.
///
/// Rounds in `f64`: an `f32` mantissa can't hold `coordinate * 10_000` past
/// ~1677px, and diagram coordinates run well beyond that.
fn round(value: f32) -> f64 {
    (f64::from(value) * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal reproduction of merman's state-diagram shape: a 23px band at the
    /// origin, with the text origin pushed to the band's center on top of the
    /// `dy="1.1em"` baseline offset the markup already carries.
    const STATE_SHAPE: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">"#,
        r#"<style>text{font-family:Arial;font-size:16px}</style>"#,
        r#"<g class="edgeLabel" transform="translate(100, 30)">"#,
        r#"<g class="label" transform="translate(-40, -11.5)"><g>"#,
        r#"<rect class="background" style="stroke: none" x="0" y="0" width="80" height="23"/>"#,
        r#"<g transform="translate(40, 11.5)">"#,
        r#"<text y="-10.1" text-anchor="middle"><tspan x="0" y="-0.1em" dy="1.1em">hello</tspan></text>"#,
        r#"</g></g></g></g></svg>"#,
    );

    /// merman's flowchart/class shape: Mermaid's left-anchored band kept as-is
    /// while the text is centered, so the glyphs sit half a label width left.
    const FLOWCHART_SHAPE: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60">"#,
        r#"<style>text{font-family:Arial;font-size:16px}</style>"#,
        r#"<g class="edgeLabel" transform="translate(100, 30)">"#,
        r#"<g class="label" transform="translate(-40, -11.5)"><g>"#,
        r#"<rect class="background" x="-2" y="-1" width="80" height="23"/>"#,
        r#"<text y="-10.1" text-anchor="middle"><tspan x="0" y="-0.1em" dy="1.1em">hello</tspan></text>"#,
        r#"</g></g></g></svg>"#,
    );

    /// The rasterizer's own options, so a test measures what the app paints.
    fn options() -> usvg::Options<'static> {
        super::super::visual::usvg_options()
    }

    /// Absolute band and glyph boxes of the single label in `svg`.
    fn geometry(svg: &str) -> (Box2, Box2) {
        let tree = usvg::Tree::from_str(svg, &options()).expect("parses");
        let (mut rect, mut text) = (None, None);
        fn walk(group: &usvg::Group, rect: &mut Option<Box2>, text: &mut Option<Box2>) {
            for node in group.children() {
                match node {
                    usvg::Node::Path(p) => *rect = Some(Box2::from_rect(p.abs_bounding_box())),
                    usvg::Node::Text(t) => *text = Some(Box2::from_rect(t.abs_bounding_box())),
                    usvg::Node::Group(inner) => walk(inner, rect, text),
                    usvg::Node::Image(_) => {}
                }
            }
        }
        walk(tree.root(), &mut rect, &mut text);
        (rect.expect("band"), text.expect("glyphs"))
    }

    fn assert_glyphs_inside_band(svg: &str) {
        let (band, glyphs) = geometry(svg);
        assert!(
            glyphs.x >= band.x - NEGLIGIBLE_PX
                && glyphs.y >= band.y - NEGLIGIBLE_PX
                && glyphs.x + glyphs.w <= band.x + band.w + NEGLIGIBLE_PX
                && glyphs.y + glyphs.h <= band.y + band.h + NEGLIGIBLE_PX,
            "glyphs ({}, {}, {}x{}) escape band ({}, {}, {}x{})",
            glyphs.x,
            glyphs.y,
            glyphs.w,
            glyphs.h,
            band.x,
            band.y,
            band.w,
            band.h
        );
    }

    #[test]
    fn state_shape_paints_glyphs_below_its_band_before_alignment() {
        let (band, glyphs) = geometry(STATE_SHAPE);
        assert!(
            glyphs.y + glyphs.h > band.y + band.h + 1.0,
            "expected the unaligned state shape to overflow downwards"
        );
    }

    #[test]
    fn state_shape_glyphs_land_inside_the_band() {
        assert_glyphs_inside_band(&align_label_backgrounds(STATE_SHAPE, &options()));
    }

    #[test]
    fn flowchart_shape_paints_glyphs_left_of_its_band_before_alignment() {
        let (band, glyphs) = geometry(FLOWCHART_SHAPE);
        assert!(
            glyphs.x < band.x - 1.0,
            "expected the unaligned flowchart shape to overflow leftwards"
        );
    }

    #[test]
    fn flowchart_shape_glyphs_land_inside_the_band() {
        assert_glyphs_inside_band(&align_label_backgrounds(FLOWCHART_SHAPE, &options()));
    }

    #[test]
    fn band_center_is_preserved_so_it_keeps_masking_its_edge() {
        for shape in [STATE_SHAPE, FLOWCHART_SHAPE] {
            let before = geometry(shape).0.center();
            let after = geometry(&align_label_backgrounds(shape, &options()))
                .0
                .center();
            assert!(
                (before.0 - after.0).abs() < NEGLIGIBLE_PX
                    && (before.1 - after.1).abs() < NEGLIGIBLE_PX,
                "band center moved from {before:?} to {after:?}"
            );
        }
    }

    #[test]
    fn band_grows_to_cover_glyphs_wider_than_the_estimate() {
        // A band deliberately measured 20px too narrow for its text, the way a
        // CJK label lands when merman's metrics have no data for the glyphs.
        let narrow = FLOWCHART_SHAPE.replace(r#"width="80""#, r#"width="20""#);
        let (before, glyphs) = geometry(&narrow);
        assert!(glyphs.w > before.w, "fixture must start too narrow");
        let after = align_label_backgrounds(&narrow, &options());
        assert_glyphs_inside_band(&after);
        assert!(
            geometry(&after).0.w >= glyphs.w + 2.0 * BACKGROUND_PADDING - NEGLIGIBLE_PX,
            "band should grow to the glyph box plus mermaid's padding"
        );
    }

    #[test]
    fn band_is_never_shrunk_below_the_reserved_slot() {
        let wide = FLOWCHART_SHAPE.replace(r#"width="80""#, r#"width="300""#);
        let after = align_label_backgrounds(&wide, &options());
        assert!(
            (geometry(&after).0.w - 300.0).abs() < NEGLIGIBLE_PX,
            "a band wider than its glyphs is the layout's reserved slot, not an error"
        );
    }

    #[test]
    fn documents_without_a_background_band_are_returned_untouched() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50">"#,
            r#"<rect x="0" y="0" width="10" height="10"/></svg>"#,
        );
        assert_eq!(align_label_backgrounds(svg, &options()), svg);
    }

    #[test]
    fn already_aligned_documents_are_returned_untouched() {
        let aligned = align_label_backgrounds(STATE_SHAPE, &options());
        assert_eq!(align_label_backgrounds(&aligned, &options()), aligned);
    }

    #[test]
    fn scan_skips_a_band_whose_text_sits_outside_its_group() {
        // `</g>` between the rect and the text means the text belongs to the
        // next label, so pairing them would move the wrong glyphs.
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50">"#,
            r#"<g><rect class="background" x="0" y="0" width="10" height="10"/></g>"#,
            r#"<text y="0">x</text></svg>"#,
        );
        assert!(scan_labels(svg).is_empty());
    }

    #[test]
    fn scan_pairs_a_band_across_the_state_text_wrapper() {
        let labels = scan_labels(STATE_SHAPE);
        assert_eq!(labels.len(), 1);
        assert!(STATE_SHAPE[labels[0].text_element.clone()].starts_with("<text"));
        assert_eq!(labels[0].rect_local.w, 80.0);
    }

    /// Diagram kinds that emit a `.background` band, plus a Hangul label — the
    /// case merman's metric tables have no data for.
    const REAL_DIAGRAMS: &[(&str, &str)] = &[
        (
            "state",
            "stateDiagram-v2\n  A --> B: schedule tick\n  B --> C: payload received\n",
        ),
        (
            "state hangul",
            "stateDiagram-v2\n  A --> B: 스케줄 틱 발생\n  B --> C: 검증 통과\n",
        ),
        (
            "state multiline",
            "stateDiagram-v2\n  A --> B: first line<br/>second line\n",
        ),
        (
            "flowchart",
            "flowchart TD\n  A[start] -->|schedule tick| B[next]\n",
        ),
        (
            "class",
            "classDiagram\n  Collector --> Validator : payload received\n",
        ),
        ("er", "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n"),
    ];

    /// The SVG the app actually ships for `source`, before alignment.
    fn render_real(source: &str) -> String {
        super::super::visual::render_mermaid_svg(
            source,
            &super::super::mermaid_theme::MermaidPalette::default(),
        )
        .expect("diagram should render")
    }

    /// Absolute band and glyph boxes of every label, paired through the module's
    /// own probe ids.
    fn label_boxes(svg: &str) -> Vec<(Box2, Box2)> {
        let labels = scan_labels(svg);
        let measured = measure(&inject_probe_ids(svg, &labels), &options()).expect("probed parse");
        (0..labels.len())
            .filter_map(|i| {
                measured
                    .backgrounds
                    .get(&i)
                    .zip(measured.texts.get(&i))
                    .map(|(r, t)| (*r, *t))
            })
            .collect()
    }

    /// How far the glyphs escape their band, worst side.
    fn worst_overflow(band: Box2, glyphs: Box2) -> f32 {
        [
            band.x - glyphs.x,
            glyphs.x + glyphs.w - (band.x + band.w),
            band.y - glyphs.y,
            glyphs.y + glyphs.h - (band.y + band.h),
        ]
        .into_iter()
        .fold(0.0, f32::max)
    }

    #[test]
    fn real_diagram_labels_paint_inside_their_band_after_alignment() {
        for (name, source) in REAL_DIAGRAMS {
            let aligned = align_label_backgrounds(&render_real(source), &options());
            let boxes = label_boxes(&aligned);
            assert!(!boxes.is_empty(), "{name}: no label band was measured");
            for (band, glyphs) in boxes {
                assert!(
                    worst_overflow(band, glyphs) <= NEGLIGIBLE_PX,
                    "{name}: glyphs escape their band by {}px",
                    worst_overflow(band, glyphs)
                );
            }
        }
    }

    /// Signal for retiring this module: merman still mis-places its label bands,
    /// so the alignment above is load-bearing. When this fails, upstream has
    /// fixed the geometry and [`align_label_backgrounds`] can be dropped along
    /// with its call in `visual::render_mermaid_raster`.
    #[test]
    fn merman_still_mis_places_label_bands_without_alignment() {
        let worst = REAL_DIAGRAMS
            .iter()
            .flat_map(|(_, source)| label_boxes(&render_real(source)))
            .map(|(band, glyphs)| worst_overflow(band, glyphs))
            .fold(0.0, f32::max);
        assert!(
            worst > 1.0,
            "merman now places label bands on their glyphs (worst overflow {worst}px) — \
             drop this module and its call in visual::render_mermaid_raster"
        );
    }

    #[test]
    fn attr_f32_does_not_match_a_longer_attribute_name() {
        let tag = r#"<rect stroke-width="3" x="7" width="9"/>"#;
        assert_eq!(attr_f32(tag, "x"), Some(7.0));
        assert_eq!(attr_f32(tag, "width"), Some(9.0));
    }

    #[test]
    fn resized_rect_tag_keeps_the_bands_class_and_style() {
        let tag =
            r#"<rect class="background" style="stroke: none" x="0" y="0" width="80" height="23"/>"#;
        let out = resized_rect_tag(
            tag,
            Box2 {
                x: -1.5,
                y: -2.0,
                w: 90.0,
                h: 25.0,
            },
        );
        assert!(out.contains(r#"class="background""#));
        assert!(out.contains(r#"style="stroke: none""#));
        assert!(out.contains(r#"x="-1.5""#));
        assert!(out.contains(r#"height="25""#));
    }
}
