//! Forces label-text contrast on mermaid nodes that carry an author-declared
//! `classDef`/`style` fill, without touching that fill.
//!
//! merman's own CSS emission for a `classDef`/`style` declaration only
//! reaches the label text in the flowchart `classDef` case (a class-scoped
//! `.hl tspan{fill:...}` rule). For flowchart `style`, class diagram
//! `classDef`, and ER diagram `style`, the whole declaration lands as an
//! inline `style="fill:...;stroke:...;color:..."` on the node's SHAPE only —
//! the injected `color:` never reaches the label, so the text keeps
//! whatever daruda's forced dark/light default is, which can collide with a
//! light (or dark) author-declared fill.
//!
//! This module rewrites the *rendered* SVG instead of the mermaid source, so
//! the fix is scoped to exactly the nodes with a custom fill and never
//! touches the fill itself — the box keeps whatever stroke/fill the author
//! declared; only the label text color is corrected. Two shapes of label
//! text exist in practice:
//!
//! - **In-subtree** (flowchart `style`, class diagram `classDef`): the label
//!   lives as a `<tspan>` inside the node's own `<g id="merman-...">`. Fixed
//!   by injecting an ID-scoped `#id tspan{fill:...}` rule into the SVG's
//!   `<style>` block — a `fill` set directly on an element always wins over
//!   an inherited one, regardless of the ancestor rule's specificity.
//! - **Sibling fallback** (ER diagram `style`): merman renders ER labels
//!   through a `foreignObject` HTML fallback path that resvg can't paint, so
//!   it substitutes a plain `<text fill="...">` positioned to coincide with
//!   the node but structurally a *sibling*, not a descendant — a CSS
//!   descendant selector can't reach it. Fixed by matching each fallback
//!   text's absolute position (via a `usvg` measuring parse, same technique
//!   as `mermaid_label_geometry`) against every custom-fill node's absolute
//!   bounding box, then rewriting the `fill="..."` attribute directly.
//!
//! The replacement color is a near-black or near-white tone that keeps the
//! fill's own hue (never literal `#000000`/`#ffffff`) — pure black would
//! collide with `mermaid_host_scoped_css`'s `text[fill="#000"]` force-rewrite
//! (a *different* fix, for merman's separately hardcoded black text
//! elsewhere), silently undoing this one.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use resvg::usvg;

/// Attribute prefix for the ids injected on fallback `<text>` elements so
/// their absolute position can be read back after the measuring parse. Not
/// present in merman output, so it cannot collide.
const PROBE_PREFIX: &str = "daruda-mermaid-node-contrast";

/// A node whose shape carries an author-declared fill (`classDef`/`style`),
/// found on the un-probed source SVG.
struct CustomNode {
    /// merman's own unique id for the node's `<g>`, e.g. `merman-flowchart-B-1`.
    id: String,
    /// Contrast-safe text color for this node's declared fill.
    contrast: String,
    /// Whether the label lives as a `<tspan>` inside this node's own subtree
    /// (flowchart/class) vs. a sibling fallback `<text>` (ER).
    has_tspan: bool,
}

/// A `foreignObject`-fallback label `<text>`, positioned to coincide with its
/// node but not a descendant of it.
struct FallbackText {
    /// Byte range of the `fill="..."` attribute's value, for direct rewrite.
    fill_value: Range<usize>,
    /// Byte range of the opening `<text ...>` tag, for probe injection.
    text_tag: Range<usize>,
}

/// An axis-aligned absolute box, read back from the measuring parse.
#[derive(Clone, Copy)]
struct Box2 {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Box2 {
    fn from_rect(rect: usvg::Rect) -> Self {
        Self {
            x: rect.x(),
            y: rect.y(),
            w: rect.width(),
            h: rect.height(),
        }
    }

    fn contains_point(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// Rewrite `svg` so every custom-fill node's label text is contrast-safe.
/// Returns `svg` unchanged when no node carries an author-declared fill.
pub(in crate::workspace) fn force_node_label_contrast(
    svg: &str,
    options: &usvg::Options<'_>,
) -> String {
    let nodes = scan_custom_fill_nodes(svg);
    if nodes.is_empty() {
        return svg.to_owned();
    }
    let fallbacks = scan_fallback_texts(svg);

    let boxes = if fallbacks.is_empty() {
        HashMap::new()
    } else {
        let probed = inject_probes(svg, &fallbacks);
        measure_boxes(&probed, options)
    };

    let mut rewrites: Vec<(Range<usize>, &str)> = Vec::new();
    for (i, fallback) in fallbacks.iter().enumerate() {
        let Some(text_box) = boxes.get(&format!("{PROBE_PREFIX}-{i}")) else {
            continue;
        };
        let (cx, cy) = text_box.center();
        let hit = nodes
            .iter()
            .find(|n| boxes.get(&n.id).is_some_and(|b| b.contains_point(cx, cy)));
        if let Some(node) = hit {
            rewrites.push((fallback.fill_value.clone(), node.contrast.as_str()));
        }
    }

    let rewritten = apply_rewrites(svg, &rewrites);

    let css_rules: String = nodes
        .iter()
        .filter(|n| n.has_tspan)
        .map(|n| format!("#{} tspan{{fill:{} !important;}}", n.id, n.contrast))
        .collect();
    if css_rules.is_empty() {
        rewritten
    } else {
        inject_rules(&rewritten, &css_rules)
    }
}

/// Locate every node `<g>` whose shape carries an inline `style="...fill:...`
/// — merman's signature for an author `classDef`/`style` declaration; a
/// default-themed node gets its fill from a global CSS class rule instead.
fn scan_custom_fill_nodes(svg: &str) -> Vec<CustomNode> {
    let mut nodes = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = find_group_open(&svg[from..]) {
        let start = from + rel;
        let Some(end) = tag_end(svg, start) else {
            break;
        };
        let tag = &svg[start..end];
        from = end;
        let Some(id) = extract_attr(tag, "id") else {
            continue;
        };
        if !id.starts_with("merman-") {
            continue;
        }
        if !extract_attr(tag, "class").is_some_and(|class| has_class_token(class, "node")) {
            continue;
        }
        let body_end = if tag.trim_end().ends_with("/>") {
            end
        } else {
            let Some(close) = matching_g_close(svg, end) else {
                continue;
            };
            close
        };
        let body = &svg[end..body_end];
        let Some(fill) = extract_style_fill(body) else {
            continue;
        };
        let Some((r, g, b, _a)) = super::mermaid_contrast::parse_color_with_alpha(&fill) else {
            continue;
        };
        nodes.push(CustomNode {
            id: id.to_owned(),
            contrast: contrast_text_color(r, g, b),
            has_tspan: body.contains("<tspan"),
        });
        from = body_end;
    }
    nodes
}

/// Locate every `foreignObject`-fallback label `<text>` — the sibling-text
/// path ER diagram labels (and any other markdown/HTML node label) render
/// through when resvg can't paint the real `foreignObject`.
fn scan_fallback_texts(svg: &str) -> Vec<FallbackText> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = svg[from..].find("<text") {
        let start = from + rel;
        match svg.as_bytes().get(start + 5) {
            Some(b' ' | b'>' | b'/') => {}
            _ => {
                from = start + 5;
                continue;
            }
        }
        let Some(end) = tag_end(svg, start) else {
            break;
        };
        let tag = &svg[start..end];
        from = end;
        if !extract_attr(tag, "class")
            .is_some_and(|class| has_class_token(class, "merman-foreignobject-fallback-text"))
        {
            continue;
        }
        let needle = " fill=\"";
        let Some(rel_fill) = tag.find(needle) else {
            continue;
        };
        let value_start = start + rel_fill + needle.len();
        let Some(len) = svg[value_start..].find('"') else {
            continue;
        };
        out.push(FallbackText {
            fill_value: value_start..value_start + len,
            text_tag: start..end,
        });
    }
    out
}

/// End offset (exclusive) of the tag starting at `start`, including its `>`.
fn tag_end(svg: &str, start: usize) -> Option<usize> {
    svg[start..].find('>').map(|i| start + i + 1)
}

/// Byte offset of the next `<g ...>`/`<g>`/`<g/...>` group-tag start in `s`,
/// skipping any `<g` substring that isn't actually a group-tag boundary
/// (there are none in merman output today, but the check is cheap).
fn find_group_open(s: &str) -> Option<usize> {
    s.match_indices("<g")
        .find(|(i, _)| matches!(s.as_bytes().get(i + 2), Some(b' ' | b'>' | b'/')))
        .map(|(i, _)| i)
}

/// Byte offset just past the `</g>` matching the group tag ending at
/// `open_tag_end`, honoring nested `<g>`/`</g>` pairs. `None` if unbalanced.
fn matching_g_close(svg: &str, open_tag_end: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut cursor = open_tag_end;
    loop {
        let rest = &svg[cursor..];
        let next_open = find_group_open(rest);
        let next_close = rest.find("</g>");
        match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => {
                depth += 1;
                cursor += o + 2;
            }
            (_, Some(c)) => {
                depth -= 1;
                cursor += c + "</g>".len();
                if depth == 0 {
                    return Some(cursor);
                }
            }
            _ => return None,
        }
    }
}

/// Read `name="value"` from a tag.
fn extract_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = start + tag[start..].find('"')?;
    Some(&tag[start..end])
}

fn has_class_token(class: &str, token: &str) -> bool {
    class.split_whitespace().any(|t| t == token)
}

/// The first `fill:` value inside any `style="..."` attribute in `body`.
fn extract_style_fill(body: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(r#"style=""#) {
        let value_start = from + rel + r#"style=""#.len();
        let len = body[value_start..].find('"')?;
        let value = &body[value_start..value_start + len];
        if let Some(fill_rel) = value.find("fill:") {
            let fill_start = fill_rel + "fill:".len();
            let end = value[fill_start..]
                .find(';')
                .map_or(value.len(), |i| fill_start + i);
            return Some(value[fill_start..end].trim().to_owned());
        }
        from = value_start + len + 1;
    }
    None
}

/// Copy of `svg` with a probe id on every fallback `<text>`, so its absolute
/// position can be paired back to the byte range after the measuring parse.
fn inject_probes(svg: &str, texts: &[FallbackText]) -> String {
    let mut out = String::with_capacity(svg.len() + texts.len() * 48);
    let mut copied = 0usize;
    for (i, fallback) in texts.iter().enumerate() {
        let insert_at = fallback.text_tag.start + "<text".len();
        out.push_str(&svg[copied..insert_at]);
        let _ = write!(out, r#" id="{PROBE_PREFIX}-{i}""#);
        copied = insert_at;
    }
    out.push_str(&svg[copied..]);
    out
}

/// Parse the probed document and read back every element's absolute bounding
/// box, keyed by its `id` (merman's own node ids and this module's probe ids
/// both survive the parse, so one map serves both lookups).
fn measure_boxes(probed: &str, options: &usvg::Options<'_>) -> HashMap<String, Box2> {
    let Ok(tree) = usvg::Tree::from_str(probed, options) else {
        return HashMap::new();
    };
    let mut boxes = HashMap::new();
    collect(tree.root(), &mut boxes);
    boxes
}

fn collect(group: &usvg::Group, out: &mut HashMap<String, Box2>) {
    if !group.id().is_empty() {
        out.insert(
            group.id().to_owned(),
            Box2::from_rect(group.abs_bounding_box()),
        );
    }
    for node in group.children() {
        match node {
            usvg::Node::Text(text) => {
                if !text.id().is_empty() {
                    out.insert(
                        text.id().to_owned(),
                        Box2::from_rect(text.abs_bounding_box()),
                    );
                }
            }
            usvg::Node::Group(inner) => collect(inner, out),
            usvg::Node::Path(_) | usvg::Node::Image(_) => {}
        }
    }
}

/// Apply a set of `fill="..."` attribute-value replacements to the original
/// (un-probed) document.
fn apply_rewrites(svg: &str, rewrites: &[(Range<usize>, &str)]) -> String {
    if rewrites.is_empty() {
        return svg.to_owned();
    }
    let mut sorted: Vec<&(Range<usize>, &str)> = rewrites.iter().collect();
    sorted.sort_by_key(|(range, _)| range.start);
    let mut out = String::with_capacity(svg.len());
    let mut copied = 0usize;
    for (range, color) in sorted {
        out.push_str(&svg[copied..range.start]);
        out.push_str(color);
        copied = range.end;
    }
    out.push_str(&svg[copied..]);
    out
}

/// Append `css` immediately before the document's closing `</style>`.
/// Returns `svg` unchanged if it carries no `<style>` block (never happens
/// for a merman render, but `force_node_label_contrast` only reaches this
/// path when there is at least one custom-fill node to style).
fn inject_rules(svg: &str, css: &str) -> String {
    let Some(pos) = svg.rfind("</style>") else {
        return svg.to_owned();
    };
    let mut out = String::with_capacity(svg.len() + css.len());
    out.push_str(&svg[..pos]);
    out.push_str(css);
    out.push_str(&svg[pos..]);
    out
}

/// A near-black or near-white text color (WCAG luminance crossover at
/// `0.179`, the standard black-vs-white text decision point) that keeps
/// `(r, g, b)`'s own hue — deliberately never pure `#000000`/`#ffffff`, so it
/// can't collide with `mermaid_host_scoped_css`'s literal-black override.
fn contrast_text_color(r: u8, g: u8, b: u8) -> String {
    const NEAR_BLACK: f64 = 0.08;
    const NEAR_WHITE: f64 = 0.92;
    let (h, s, _l) = rgb_to_hsl(r, g, b);
    let target_l = if relative_luminance(r, g, b) > 0.179 {
        NEAR_BLACK
    } else {
        NEAR_WHITE
    };
    let (tr, tg, tb) = hsl_to_rgb(h, s, target_l);
    format!("#{tr:02x}{tg:02x}{tb:02x}")
}

fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    if s.abs() < f64::EPSILON {
        let v = (l * 255.0).round() as u8;
        return (v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |p: f64, q: f64, t: f64| {
        let t = if t < 0.0 {
            t + 1.0
        } else if t > 1.0 {
            t - 1.0
        } else {
            t
        };
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (
        (hue_to_rgb(p, q, h + 1.0 / 3.0) * 255.0).round() as u8,
        (hue_to_rgb(p, q, h) * 255.0).round() as u8,
        (hue_to_rgb(p, q, h - 1.0 / 3.0) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> usvg::Options<'static> {
        super::super::visual::usvg_options()
    }

    #[test]
    fn contrast_text_color_is_dark_on_a_light_fill() {
        // #d4f8d4 is a light green; luminance is well above the crossover.
        let color = contrast_text_color(0xd4, 0xf8, 0xd4);
        let (r, g, _b) = (
            u8::from_str_radix(&color[1..3], 16).unwrap(),
            u8::from_str_radix(&color[3..5], 16).unwrap(),
            u8::from_str_radix(&color[5..7], 16).unwrap(),
        );
        assert!(r < 60 && g < 60, "expected a dark tone, got {color}");
        assert_ne!(color, "#000000", "must not land on literal black");
    }

    #[test]
    fn contrast_text_color_is_light_on_a_dark_fill() {
        let color = contrast_text_color(0x1a, 0x33, 0x1a);
        let r = u8::from_str_radix(&color[1..3], 16).unwrap();
        assert!(r > 200, "expected a light tone, got {color}");
        assert_ne!(color, "#ffffff", "must not land on literal white");
    }

    #[test]
    fn has_class_token_matches_whole_tokens_only() {
        assert!(has_class_token("node default", "node"));
        assert!(!has_class_token("nodeLabel", "node"));
        assert!(!has_class_token("edgeLabel node", "nodee"));
    }

    #[test]
    fn documents_without_custom_fill_nodes_are_returned_untouched() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50">"#,
            r#"<g class="node default" id="merman-flowchart-A-0">"#,
            r#"<rect fill="#414243"/></g></svg>"#,
        );
        assert_eq!(force_node_label_contrast(svg, &options()), svg);
    }

    /// Synthetic reproduction of the flowchart/class-diagram shape: the
    /// label's `<tspan>` lives inside the styled node's own `<g id>`.
    const IN_SUBTREE_SHAPE: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node default" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan class="text-inner-tspan">Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );

    #[test]
    fn in_subtree_label_gets_an_id_scoped_css_rule() {
        let out = force_node_label_contrast(IN_SUBTREE_SHAPE, &options());
        assert!(
            out.contains("#merman-flowchart-B-1 tspan{fill:"),
            "expected an id-scoped tspan rule, got: {out}"
        );
        // The declared fill itself must survive untouched.
        assert!(out.contains("fill:#d4f8d4;stroke:#2d8a2d"));
    }

    #[test]
    fn a_node_without_a_custom_fill_gets_no_rule() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
            r#"<style></style>"#,
            r#"<g class="node default" id="merman-flowchart-A-0" transform="translate(50, 50)">"#,
            r#"<rect fill="#414243" x="-40" y="-20" width="80" height="40"/>"#,
            r#"<g class="label"><text><tspan>Start</tspan></text></g>"#,
            r#"</g></svg>"#,
        );
        assert_eq!(force_node_label_contrast(svg, &options()), svg);
    }

    /// Synthetic reproduction of the ER shape: the styled node's own label
    /// group is empty, and the real text is a sibling `foreignObject`
    /// fallback positioned to coincide with the node.
    const SIBLING_FALLBACK_SHAPE: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">"#,
        r#"<style></style>"#,
        r#"<g id="merman-entity-CUSTOMER-0" class="node default" transform="translate(100, 100)">"#,
        r#"<rect class="basic label-container" style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-60" y="-40" width="120" height="80"/>"#,
        r#"<g class="label" transform="translate(-40, -20)"></g>"#,
        r#"</g>"#,
        r#"<g data-merman-foreignobject="fallback" class="merman-foreignobject-fallback root nodes node default label nodeLabel">"#,
        r#"<text x="100" y="80" fill="#d5d7db" class="merman-foreignobject-fallback-text root nodes node default nodeLabel">CUSTOMER</text>"#,
        r#"</g></svg>"#,
    );

    #[test]
    fn sibling_fallback_text_fill_is_rewritten_when_it_overlaps_a_custom_fill_node() {
        let out = force_node_label_contrast(SIBLING_FALLBACK_SHAPE, &options());
        assert!(
            !out.contains(r#"fill="#d5d7db""#),
            "the fallback text's original fill should have been replaced: {out}"
        );
        // No in-subtree tspan in this shape, so no CSS rule should be injected.
        assert!(!out.contains("tspan{fill:"));
    }

    #[test]
    fn sibling_fallback_text_outside_any_node_box_is_left_alone() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200">"#,
            r#"<style></style>"#,
            r#"<g id="merman-entity-CUSTOMER-0" class="node default" transform="translate(100, 100)">"#,
            r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-60" y="-40" width="120" height="80"/>"#,
            r#"</g>"#,
            r#"<g data-merman-foreignobject="fallback" class="merman-foreignobject-fallback root nodes node default label nodeLabel">"#,
            r#"<text x="350" y="10" fill="#d5d7db" class="merman-foreignobject-fallback-text root nodes node default nodeLabel">Unrelated</text>"#,
            r#"</g></svg>"#,
        );
        let out = force_node_label_contrast(svg, &options());
        assert!(out.contains(r#"fill="#d5d7db""#));
    }

    /// Every diagram/directive shape the investigation found broken,
    /// rendered through the real production path — proof the fix reaches
    /// merman's actual output, not just the synthetic fixtures above.
    #[test]
    fn real_broken_diagrams_get_a_contrast_fix() {
        let palette = super::super::mermaid_theme::MermaidPalette::default();
        let samples: &[(&str, &str, &str)] = &[
            (
                "flowchart style",
                "flowchart TD\n  A[Start] --> B[Highlighted]\n  style B fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px\n",
                "tspan{fill:",
            ),
            (
                "class diagram classDef",
                "classDiagram\n  class Dog\n  class Dog:::highlighted\n  classDef highlighted fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px\n",
                "tspan{fill:",
            ),
            (
                "er diagram style",
                "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  style CUSTOMER fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px\n",
                "",
            ),
        ];
        for (name, source, must_contain) in samples {
            let svg = super::super::visual::render_mermaid_svg(source, &palette)
                .unwrap_or_else(|| panic!("{name}: should render"));
            let fixed = force_node_label_contrast(&svg, &options());
            assert_ne!(fixed, svg, "{name}: expected a rewrite");
            if !must_contain.is_empty() {
                assert!(
                    fixed.contains(must_contain),
                    "{name}: expected to find {must_contain:?}"
                );
            }
        }
    }
}
