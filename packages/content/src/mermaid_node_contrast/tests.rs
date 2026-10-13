use super::*;

fn options() -> usvg::Options<'static> {
    crate::visual::usvg_options()
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
        r##"<rect fill="#414243"/></g></svg>"##,
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
fn a_node_id_without_the_merman_prefix_still_gets_fixed() {
    // State diagram node ids look like `state-Running-2`, not
    // `merman-<kind>-<name>-<n>` — detection must not require the prefix.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node highlighted statediagram-state" id="state-Running-2" transform="translate(50, 50)">"#,
        r#"<path style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" d="M0 0"/>"#,
        r#"<g class="label"><text><tspan>Running</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert!(
        force_node_label_contrast(svg, &options()).contains("#state-Running-2 tspan{fill:"),
        "a non-merman-prefixed id must still be detected"
    );
}

#[test]
fn an_empty_id_is_never_used_to_build_a_selector() {
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node default" id="" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan>Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert_eq!(force_node_label_contrast(svg, &options()), svg);
}

#[test]
fn a_node_without_a_custom_fill_gets_no_rule() {
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node default" id="merman-flowchart-A-0" transform="translate(50, 50)">"#,
        r##"<rect fill="#414243" x="-40" y="-20" width="80" height="40"/>"##,
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
    r##"<text x="100" y="80" fill="#d5d7db" class="merman-foreignobject-fallback-text root nodes node default nodeLabel">CUSTOMER</text>"##,
    r#"</g></svg>"#,
);

#[test]
fn sibling_fallback_text_fill_is_rewritten_when_it_overlaps_a_custom_fill_node() {
    let out = force_node_label_contrast(SIBLING_FALLBACK_SHAPE, &options());
    assert!(
        !out.contains(r##"fill="#d5d7db""##),
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
        r##"<text x="350" y="10" fill="#d5d7db" class="merman-foreignobject-fallback-text root nodes node default nodeLabel">Unrelated</text>"##,
        r#"</g></svg>"#,
    );
    let out = force_node_label_contrast(svg, &options());
    assert!(out.contains(r##"fill="#d5d7db""##));
}

#[test]
fn a_node_whose_class_already_has_a_working_tspan_rule_is_left_alone() {
    // Reproduces a flowchart `classDef` with an explicit author `color:`
    // — the one case merman wires correctly on its own (a real
    // `.hl tspan{fill:...}` rule reflecting the declared color).
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style>.hl tspan{fill:#0d4f0d;}</style>"#,
        r#"<g class="node default hl" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan class="text-inner-tspan">Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert_eq!(
        force_node_label_contrast(svg, &options()),
        svg,
        "must not add a second, conflicting rule over the author's own explicit color"
    );
}

#[test]
fn a_node_with_a_different_working_class_rule_still_gets_fixed() {
    // A `.hl tspan{...}` rule for a *different* class must not suppress
    // the fix for this node's own (uncolored) class.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style>.hl tspan{fill:#0d4f0d;}</style>"#,
        r#"<g class="node default other" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan class="text-inner-tspan">Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert!(
        force_node_label_contrast(svg, &options()).contains("#merman-flowchart-B-1 tspan{fill:")
    );
}

#[test]
fn a_generic_structural_rule_does_not_suppress_every_custom_node() {
    // A hypothetical future merman emitting a *generic* `.node
    // tspan{fill:...}` (or `.default tspan{...}`) rule — for some
    // unrelated structural reason, not an author `color:` — must not be
    // mistaken for this node's own explicit color and suppress its fix.
    // Every node carries "node"/"default" in its class list, so getting
    // this wrong would silently disable the fix diagram-wide.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style>.node tspan{fill:#d5d7db;}.default tspan{fill:#d5d7db;}</style>"#,
        r#"<g class="node default" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan>Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert!(
        force_node_label_contrast(svg, &options()).contains("#merman-flowchart-B-1 tspan{fill:"),
        "a generic structural-class rule must not suppress this node's fix"
    );
}

#[test]
fn a_near_invisible_fill_is_left_alone() {
    // Alpha well under MIN_SIGNIFICANT_ALPHA: the fill barely tints the
    // surface, so there is nothing worth forcing a contrast decision
    // against.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node default" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:rgba(212,248,212,0.05);stroke:#2d8a2d" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan>Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    assert_eq!(force_node_label_contrast(svg, &options()), svg);
}

#[test]
fn a_property_name_merely_ending_in_fill_is_not_mistaken_for_the_fill_declaration() {
    // A hypothetical `background-fill:` property (never emitted by
    // merman today) must not be parsed as if it were `fill:`.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<style></style>"#,
        r#"<g class="node default" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="background-fill:#000000;fill:#d4f8d4;stroke:#2d8a2d" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan>Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    let out = force_node_label_contrast(svg, &options());
    assert!(
        out.contains("#merman-flowchart-B-1 tspan{fill:#0"),
        "expected a dark contrast color derived from the light fill, got: {out}"
    );
}

#[test]
fn the_smallest_containing_box_wins_when_boxes_overlap() {
    // A cluster/subgraph can itself be a custom-fill node wrapping a
    // nested custom-fill node — the fallback text at their shared
    // center must attribute to the innermost (smaller) box, not
    // whichever node happens to be scanned first.
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="400">"#,
        r#"<style></style>"#,
        r#"<g id="merman-cluster-outer-0" class="node default" transform="translate(100, 100)">"#,
        r#"<rect style="fill:#1a331a;stroke:#2d8a2d" x="-90" y="-90" width="180" height="180"/>"#,
        r#"<g class="label" transform="translate(-40, -20)"></g>"#,
        r#"</g>"#,
        r#"<g id="merman-entity-CUSTOMER-1" class="node default" transform="translate(100, 100)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d" x="-60" y="-40" width="120" height="80"/>"#,
        r#"<g class="label" transform="translate(-40, -20)"></g>"#,
        r#"</g>"#,
        r#"<g data-merman-foreignobject="fallback" class="merman-foreignobject-fallback root nodes node default label nodeLabel">"#,
        r##"<text x="100" y="100" fill="#d5d7db" class="merman-foreignobject-fallback-text root nodes node default nodeLabel">CUSTOMER</text>"##,
        r#"</g></svg>"#,
    );
    let out = force_node_label_contrast(svg, &options());
    // The inner node's fill (#d4f8d4, light) demands a dark contrast
    // color; the outer node's fill (#1a331a, dark) would demand light —
    // picking the outer (larger) box would produce a light/near-white fill.
    let start = out.find("fill=\"#").expect("rewritten fill attribute");
    let value = &out[start + 6..start + 13];
    let r = u8::from_str_radix(&value[1..3], 16).unwrap();
    assert!(
        r < 60,
        "expected the inner (smaller) node's dark contrast, got {value}"
    );
}

#[test]
fn css_is_injected_even_without_an_existing_style_block() {
    let svg = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">"#,
        r#"<g class="node default" id="merman-flowchart-B-1" transform="translate(50, 50)">"#,
        r#"<rect style="fill:#d4f8d4;stroke:#2d8a2d;stroke-width:2px" x="-40" y="-20" width="80" height="40"/>"#,
        r#"<g class="label"><text><tspan>Highlighted</tspan></text></g>"#,
        r#"</g></svg>"#,
    );
    let out = force_node_label_contrast(svg, &options());
    assert!(out.contains("<style>#merman-flowchart-B-1 tspan{fill:"));
    assert!(out.contains("</style>"));
}

/// When a flowchart `classDef` explicitly declares `color:`, merman
/// already wires it correctly — the module must not override it with a
/// second, computed rule.
#[test]
fn real_classdef_with_explicit_author_color_is_not_overridden() {
    let palette = crate::test_palette();
    let source = "flowchart TD\n  A[Start] --> B[Highlighted]:::hl\n  classDef hl fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px,color:#0d4f0d\n";
    let svg = crate::visual::render_mermaid_svg(source, &palette).expect("svg");
    let fixed = force_node_label_contrast(&svg, &options());
    assert_eq!(
        fixed, svg,
        "an explicit author color already wired by merman must be left untouched"
    );
}

/// Every diagram/directive shape this module fixes, rendered through
/// the real production path — proof the fix reaches merman's actual
/// output, not just the synthetic fixtures above.
#[test]
fn real_broken_diagrams_get_a_contrast_fix() {
    let palette = crate::test_palette();
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
        (
            "state diagram classDef",
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running\n  Running --> [*]\n  class Running highlighted\n  classDef highlighted fill:#d4f8d4,stroke:#2d8a2d,stroke-width:2px\n",
            "tspan{fill:",
        ),
    ];
    for (name, source, must_contain) in samples {
        let svg = crate::visual::render_mermaid_svg(source, &palette)
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
