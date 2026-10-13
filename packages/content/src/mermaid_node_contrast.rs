//! Forces label-text contrast on mermaid nodes that carry an author-declared
//! `classDef`/`style` fill, without touching that fill.
//!
//! merman's own CSS emission for a `classDef`/`style` declaration only
//! reaches the label text for a flowchart `classDef` that also declares an
//! explicit `color:` (a class-scoped `.hl tspan{fill:...}` rule reflecting
//! that color). Without an explicit `color:`, flowchart `classDef` is just
//! as broken as the other three paths below. For flowchart `style`, class
//! diagram `classDef`, and ER diagram `style`, the whole declaration lands
//! as an inline `style="fill:...;stroke:...;color:..."` on the node's SHAPE
//! only — the injected `color:` never reaches the label, so the text keeps
//! whatever daruda's forced dark/light default is, which can collide with a
//! light (or dark) author-declared fill.
//!
//! This module rewrites the *rendered* SVG instead of the mermaid source, so
//! the fix is scoped to exactly the nodes with a custom fill and never
//! touches the fill itself — the box keeps whatever stroke/fill the author
//! declared; only the label text color is corrected. Two shapes of label
//! text exist in practice:
//!
//! - **In-subtree** (flowchart `style`, class diagram `classDef`, state
//!   diagram `classDef`): the label lives as a `<tspan>` inside the node's
//!   own `<g id="...">`. Fixed by injecting an ID-scoped `#id tspan{fill:...}`
//!   rule into the SVG's `<style>` block — a `fill` set directly on an
//!   element always wins over an inherited one, regardless of the ancestor
//!   rule's specificity. Node ids aren't scoped to a `merman-` prefix (state
//!   diagrams use `state-<name>-<n>`, not `merman-<kind>-<name>-<n>`), so
//!   detection keys on the node's `class="node ..."` token instead.
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
//!
//! This module only ever sets `fill`. A `stroke` bleeding into label text
//! from an ancestor's `classDef`/`style` — a separate bug some diagram
//! types' CSS emission causes — is `mermaid_label_stroke`'s job.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::Range;

use resvg::usvg;

/// Attribute prefix for the ids injected on fallback `<text>` elements so
/// their absolute position can be read back after the measuring parse. Not
/// present in merman output, so it cannot collide.
const PROBE_PREFIX: &str = "daruda-mermaid-node-contrast";

/// Below this alpha, a declared fill barely tints the surface — computing a
/// black-or-white decision against its raw RGB would answer the wrong
/// question (contrast against a color that isn't really what's rendered;
/// the host canvas dominates instead). Nodes this faint are left to whatever
/// text color the host default already provides for them.
const MIN_SIGNIFICANT_ALPHA: f64 = 0.2;

/// A node whose shape carries an author-declared fill (`classDef`/`style`),
/// found on the un-probed source SVG.
struct CustomNode {
    /// merman's own unique id for the node's `<g>`, e.g. `merman-flowchart-B-1`
    /// or (state diagrams) `state-Running-2`.
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

    fn area(&self) -> f32 {
        self.w * self.h
    }
}

/// Rewrite `svg` so every custom-fill node's label text is contrast-safe.
/// Returns `svg` unchanged when no node carries an author-declared fill.
pub(super) fn force_node_label_contrast(svg: &str, options: &usvg::Options<'_>) -> String {
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
        // Prefer the smallest containing box, not the first match: a
        // subgraph/cluster can be a custom-fill node itself and wrap a
        // nested custom-fill node, so more than one box may contain the
        // point — the innermost one is the label's actual owner.
        let hit = nodes
            .iter()
            .filter_map(|n| boxes.get(&n.id).map(|b| (n, b)))
            .filter(|(_, b)| b.contains_point(cx, cy))
            .min_by(|(_, a), (_, b)| a.area().total_cmp(&b.area()));
        if let Some((node, _)) = hit {
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
        if id.is_empty() {
            // An empty id can't anchor `#id tspan{...}` (a bare `#` isn't a
            // valid CSS selector), and merman never emits one on a real node.
            continue;
        }
        let Some(class) = extract_attr(tag, "class") else {
            continue;
        };
        if !has_class_token(class, "node") {
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
        let Some((r, g, b, a)) = super::mermaid_contrast::parse_color_with_alpha(&fill) else {
            continue;
        };
        // Below the crossover the fill barely tints the surface, so the
        // host's default text color already reads fine against it — nothing
        // to fix.
        if a < MIN_SIGNIFICANT_ALPHA {
            continue;
        }
        // An author who declared an explicit `color:` in a flowchart
        // `classDef` already gets a correctly-wired `.CLASS tspan{fill:...}`
        // rule from merman itself (the one case it wires end-to-end) —
        // respect that choice instead of overriding it with our own.
        if class_already_has_tspan_fill_rule(svg, class) {
            continue;
        }
        nodes.push(CustomNode {
            id: id.to_owned(),
            contrast: contrast_text_color(r, g, b),
            has_tspan: body.contains("<tspan"),
        });
        from = body_end;
    }
    nodes
}

/// merman's own structural classes — present on every node regardless of any
/// author `classDef`/`style`, so they can never be the signal
/// [`class_already_has_tspan_fill_rule`] is looking for. Checking them too
/// would risk a diagram-wide false positive: if merman ever ships a generic
/// `.node tspan{fill:...}` rule for an unrelated reason, every custom-fill
/// node in the diagram would silently stop getting this module's fix.
const STRUCTURAL_NODE_CLASSES: [&str; 2] = ["node", "default"];

/// True when merman's own `<style>` block already defines a working
/// `.TOKEN tspan{fill:...}` rule for one of this node's *author-declared*
/// classes — the signature of an explicit `color:` that merman wired
/// correctly on its own, which this module must not clobber.
fn class_already_has_tspan_fill_rule(svg: &str, class_list: &str) -> bool {
    class_list
        .split_whitespace()
        .filter(|token| !STRUCTURAL_NODE_CLASSES.contains(token))
        .any(|token| svg.contains(&format!(".{token} tspan{{fill:")))
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
            Some(b' ' | b'>' | b'/' | b'\t' | b'\n' | b'\r') => {}
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
        .find(|(i, _)| {
            matches!(
                s.as_bytes().get(i + 2),
                Some(b' ' | b'>' | b'/' | b'\t' | b'\n' | b'\r')
            )
        })
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

/// The first `fill:` *declaration* (not a longer property merely ending in
/// `fill`, e.g. a hypothetical `background-fill:`) inside any `style="..."`
/// attribute in `body`.
fn extract_style_fill(body: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(r#"style=""#) {
        let value_start = from + rel + r#"style=""#.len();
        let len = body[value_start..].find('"')?;
        let value = &body[value_start..value_start + len];
        for decl in value.split(';') {
            let decl = decl.trim();
            if let Some(color) = decl.strip_prefix("fill:") {
                return Some(color.trim().to_owned());
            }
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

/// Append `css` immediately before the document's closing `</style>`. Every
/// merman render carries a `<style>` block already (the host theme profile
/// guarantees it), but a document with none is given its own — right after
/// the root `<svg ...>` tag — rather than silently dropping the fix.
fn inject_rules(svg: &str, css: &str) -> String {
    let mut out = String::with_capacity(svg.len() + css.len() + "<style></style>".len());
    if let Some(pos) = svg.rfind("</style>") {
        out.push_str(&svg[..pos]);
        out.push_str(css);
        out.push_str(&svg[pos..]);
    } else if let Some(svg_tag_end) = svg.find("<svg").and_then(|s| tag_end(svg, s)) {
        out.push_str(&svg[..svg_tag_end]);
        let _ = write!(out, "<style>{css}</style>");
        out.push_str(&svg[svg_tag_end..]);
    } else {
        out.push_str(svg);
    }
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
    let (r, g, b) = (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
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
mod tests;
