//! Forces every mermaid label's text to render with no stroke.
//!
//! merman's `classDef`/`style` CSS for some diagram types (state diagrams'
//! `.CLASS>*{fill:...;stroke:...}` universal *direct-child* selector, in
//! particular) targets every direct child of a highlighted node — which
//! includes the label's own `<g>` wrapper, not just its shape. `stroke` is
//! an inherited SVG property, so it flows from that `<g>` down into the
//! label's `<tspan>`s: the glyphs get outlined in the classDef's stroke
//! color, reading as a thick/bold blob instead of plain text. Flowchart's
//! own `classDef` CSS is scoped precisely to shape tags (`.hl rect{...}`,
//! `.hl polygon{...}`, …) and never hits this; state diagrams' broader
//! selector does.
//!
//! No mermaid diagram type is ever meant to render text with a visible
//! stroke — every readable sample this app ships is plain single-color
//! text. Rather than replicate merman's selector logic per diagram type
//! (fragile across its versions, and this module would need to learn a new
//! quirk each time one surfaces), this forces the actually-intended
//! behavior directly: no `tspan` or `<text>` ever paints a stroke,
//! unconditionally.

use std::fmt::Write as _;

/// A `stroke:none` override for every `tspan` and bare `<text>` (ER's
/// `foreignObject`-fallback labels have no nested `tspan` at all), appended
/// to `svg`'s `<style>` block. A direct match on the element itself always
/// wins over an inherited value regardless of the inherited rule's
/// specificity, so this reliably cancels a stroke leaking down from any
/// ancestor — the mechanism doesn't need to be known, only that it's
/// inherited.
const RULE: &str = "tspan,text{stroke:none !important;}";

/// Rewrite `svg` so no label text renders a stroke, creating a `<style>`
/// block right after the root `<svg>` tag if none exists.
pub(in crate::workspace) fn suppress_label_text_stroke(svg: &str) -> String {
    if let Some(pos) = svg.rfind("</style>") {
        let mut out = String::with_capacity(svg.len() + RULE.len());
        out.push_str(&svg[..pos]);
        out.push_str(RULE);
        out.push_str(&svg[pos..]);
        out
    } else if let Some(svg_tag_end) = svg.find("<svg").and_then(|s| tag_end(svg, s)) {
        let mut out = String::with_capacity(svg.len() + RULE.len() + "<style></style>".len());
        out.push_str(&svg[..svg_tag_end]);
        let _ = write!(out, "<style>{RULE}</style>");
        out.push_str(&svg[svg_tag_end..]);
        out
    } else {
        svg.to_owned()
    }
}

/// End offset (exclusive) of the tag starting at `start`, including its `>`.
fn tag_end(svg: &str, start: usize) -> Option<usize> {
    svg[start..].find('>').map(|i| start + i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> resvg::usvg::Options<'static> {
        super::super::visual::usvg_options()
    }

    #[test]
    fn appends_the_rule_to_an_existing_style_block() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">"#,
            r#"<style>#merman{fill:#000;}</style></svg>"#,
        );
        let out = suppress_label_text_stroke(svg);
        assert!(out.contains("#merman{fill:#000;}tspan,text{stroke:none !important;}</style>"));
    }

    #[test]
    fn creates_a_style_block_when_none_exists() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;
        let out = suppress_label_text_stroke(svg);
        assert!(out.contains("<style>tspan,text{stroke:none !important;}</style>"));
    }

    /// Reproduces the state-diagram bug directly: a `.highlighted>*` rule
    /// strokes the label `<g>` (a direct child of the highlighted node),
    /// and that stroke inherits into the tspan.
    #[test]
    fn cancels_a_stroke_inherited_from_a_direct_child_selector() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">"#,
            r#"<style>.highlighted>*{fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px;}</style>"#,
            r#"<g class="highlighted"><g class="label" transform="translate(20,20)">"#,
            r#"<text><tspan id="probe">Running</tspan></text></g></g></svg>"#,
        );

        let before = resvg::usvg::Tree::from_str(svg, &options()).expect("parses");
        assert!(
            first_span_stroke(&before).is_some(),
            "fixture must reproduce the bug: the span must inherit a stroke before the fix"
        );

        let fixed = suppress_label_text_stroke(svg);
        let after = resvg::usvg::Tree::from_str(&fixed, &options()).expect("parses");
        assert!(
            first_span_stroke(&after).is_none(),
            "no span should have a stroke once the rule is applied"
        );
    }

    /// ER's `foreignObject`-fallback labels are bare `<text>` elements with
    /// no nested `<tspan>` at all — the rule must reach those too, not just
    /// `tspan`, or a future stroke-leak on that path would slip through.
    #[test]
    fn cancels_a_stroke_on_a_bare_text_element_with_no_tspan() {
        let svg = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">"#,
            r#"<style>.highlighted>*{fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px;}</style>"#,
            r#"<g class="highlighted"><text id="probe">CUSTOMER</text></g></svg>"#,
        );

        let before = resvg::usvg::Tree::from_str(svg, &options()).expect("parses");
        assert!(
            first_span_stroke(&before).is_some(),
            "fixture must reproduce the bug for a bare <text> element too"
        );

        let fixed = suppress_label_text_stroke(svg);
        let after = resvg::usvg::Tree::from_str(&fixed, &options()).expect("parses");
        assert!(first_span_stroke(&after).is_none());
    }

    /// Whether the first text span found anywhere in `tree` resolves a
    /// stroke — the direct, unambiguous signal for this bug (unlike text
    /// bounding boxes, which usvg's own docs note are never pixel-tight).
    fn first_span_stroke(tree: &resvg::usvg::Tree) -> Option<resvg::usvg::Stroke> {
        fn find(group: &resvg::usvg::Group) -> Option<resvg::usvg::Stroke> {
            for node in group.children() {
                match node {
                    resvg::usvg::Node::Text(text) => {
                        for chunk in text.chunks() {
                            for span in chunk.spans() {
                                if let Some(stroke) = span.stroke() {
                                    return Some(stroke.clone());
                                }
                            }
                        }
                    }
                    resvg::usvg::Node::Group(inner) => {
                        if let Some(found) = find(inner) {
                            return Some(found);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        find(tree.root())
    }

    /// The real production case: a state diagram whose `classDef` also sets
    /// `stroke:` on a highlighted node renders its label with no stroke
    /// once this pass runs.
    #[test]
    fn real_state_diagram_classdef_stroke_no_longer_bleeds_into_the_label() {
        let source = "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running\n  Running --> [*]\n  class Running highlighted\n  classDef highlighted fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px\n";
        let palette = super::super::mermaid_theme::MermaidPalette::default();
        let svg = super::super::visual::render_mermaid_svg(source, &palette).expect("svg");

        let before = resvg::usvg::Tree::from_str(&svg, &options()).expect("parses");
        assert!(
            first_span_stroke(&before).is_some(),
            "expected the real diagram to reproduce the bug before the fix"
        );

        let fixed = suppress_label_text_stroke(&svg);
        let after = resvg::usvg::Tree::from_str(&fixed, &options()).expect("parses");
        assert!(first_span_stroke(&after).is_none());
    }
}
