use super::*;
use crate::theme::contrast_ratio;

/// The editor's per-capture colours (looked up the exact way
/// gpui_component's highlighter does, via `SyntaxColors::style`) must
/// equal the diff view's `syntax_color` for the same capture. This is
/// the single-source invariant: drift between the two mappings fails
/// here.
#[test]
fn editor_and_diff_share_one_colour_source() {
    let editor = editor_syntax_colors();
    // Every capture whose `SyntaxColors` field is set explicitly in
    // `editor_syntax_colors_of`. Exercising all of them guards the
    // static field->bucket map against drifting from
    // `bucket_for_capture` (which the diff path uses). The field->bucket
    // mapping is palette-independent, so checking Daruda catches any
    // misassignment for all palettes.
    for capture in [
        "keyword",
        "function",
        "title",
        "type",
        "enum",
        "constructor",
        "label",
        "preproc",
        "embedded",
        "constant",
        "boolean",
        "number",
        "attribute",
        "variant",
        "link_uri",
        "string",
        "text.literal",
        "string.escape",
        "string.regex",
        "string.special",
        "string.special.symbol",
        "tag",
        "variable.special",
        "link_text",
        "tag.doctype",
        "comment",
        "comment.doc",
        "hint",
        "predictive",
        "variable",
        "property",
        "operator",
        "punctuation",
        "emphasis",
        // Dotted sub-captures with no exact field must fall back to the
        // first segment in both editor and diff views.
        "function.method",
        "function.call",
        "function.macro",
        "keyword.control",
        "keyword.function",
        "type.builtin",
        "constant.builtin",
        "variable.parameter",
        "variable.member",
        "punctuation.bracket",
    ] {
        let from_editor = editor.style(capture).and_then(|s| s.color);
        assert_eq!(
            from_editor,
            Some(syntax_color(capture)),
            "capture {capture:?}: editor highlight must match the diff palette"
        );
    }
}

/// Distinct semantic groups must stay visually distinct after the
/// refactor — a guard against accidentally collapsing the palette.
#[test]
fn semantic_groups_are_distinct() {
    let t = syntax_theme();
    let groups = [
        t.keyword,
        t.function,
        t.type_,
        t.constant,
        t.string,
        t.string_special,
        t.tag,
        t.tag_doctype,
        t.comment,
        t.default,
    ];
    for (i, a) in groups.iter().enumerate() {
        for b in &groups[i + 1..] {
            assert_ne!(a, b, "syntax groups must be distinct colours");
        }
    }
}

#[test]
fn from_config_name_maps_curated_and_falls_back() {
    assert_eq!(
        SyntaxPalette::from_config_name("daruda"),
        SyntaxPalette::Daruda
    );
    assert_eq!(
        SyntaxPalette::from_config_name("one-dark"),
        SyntaxPalette::OneDark
    );
    assert_eq!(
        SyntaxPalette::from_config_name("tokyo-night"),
        SyntaxPalette::TokyoNight
    );
    assert_eq!(
        SyntaxPalette::from_config_name("catppuccin-mocha"),
        SyntaxPalette::CatppuccinMocha
    );
    // Unknown + legacy base16 names fall back to the recommended default.
    assert_eq!(SyntaxPalette::from_config_name(""), SyntaxPalette::Daruda);
    assert_eq!(
        SyntaxPalette::from_config_name("base16-ocean.dark"),
        SyntaxPalette::Daruda
    );
    assert_eq!(
        SyntaxPalette::from_config_name("nonsense"),
        SyntaxPalette::Daruda
    );
}

/// Every curated palette, used to assert invariants across all of them.
const ALL_PALETTES: [SyntaxPalette; 14] = [
    SyntaxPalette::Daruda,
    SyntaxPalette::OneDark,
    SyntaxPalette::TokyoNight,
    SyntaxPalette::CatppuccinMocha,
    SyntaxPalette::Dracula,
    SyntaxPalette::GitHubDark,
    SyntaxPalette::MaterialPalenight,
    SyntaxPalette::Monokai,
    SyntaxPalette::Nord,
    SyntaxPalette::GruvboxDark,
    SyntaxPalette::SolarizedDark,
    SyntaxPalette::AyuMirage,
    SyntaxPalette::NightOwl,
    SyntaxPalette::Darcula,
];

#[test]
fn every_palette_separates_keyword_from_default() {
    for p in ALL_PALETTES {
        for is_light in [false, true] {
            let t = syntax_theme_of(p, is_light);
            assert_ne!(
                t.color_for("keyword"),
                t.color_for(""),
                "{p:?} (light={is_light}): keyword must differ from default"
            );
        }
    }
}

#[test]
fn light_variant_clears_contrast_on_light_bg() {
    // Daruda Light is the WCAG-AA light default; every bucket must clear
    // 4.5:1 on the light editor background (`#fafafa`).
    let bg = base16(0xfa_fa_fa);
    let t = syntax_theme_of(SyntaxPalette::Daruda, true);
    for capture in [
        "keyword", "function", "type", "constant", "string", "comment", "",
    ] {
        assert!(
            contrast_ratio(t.color_for(capture), bg) >= 4.5,
            "Daruda Light {capture:?} must clear 4.5:1 on #fafafa"
        );
    }
}

#[test]
fn every_palette_round_trips_through_config_name() {
    // Each curated palette must have a config name that resolves back
    // to it (the settings dropdown relies on this).
    let names = [
        "daruda",
        "one-dark",
        "tokyo-night",
        "catppuccin-mocha",
        "dracula",
        "github-dark",
        "material-palenight",
        "monokai",
        "nord",
        "gruvbox-dark",
        "solarized-dark",
        "ayu-mirage",
        "night-owl",
        "darcula",
    ];
    assert_eq!(names.len(), ALL_PALETTES.len());
    for (name, expected) in names.iter().zip(ALL_PALETTES) {
        assert_eq!(SyntaxPalette::from_config_name(name), expected, "{name}");
    }
}

#[test]
fn capture_grouping_is_shared_by_color_and_style() {
    // color_for / style_for / editor colours must all read the same
    // bucket for a capture — guard against the grouping drifting.
    let t = syntax_theme_of(SyntaxPalette::Daruda, false);
    for capture in ["keyword", "string.escape", "comment", "function", ""] {
        let bucket = bucket_for_capture(capture);
        assert_eq!(t.color_for(capture), t.color(bucket));
        assert_eq!(t.style_for(capture), t.style(bucket));
    }
}

#[test]
fn daruda_carries_non_color_channels() {
    let t = syntax_theme_of(SyntaxPalette::Daruda, false);
    assert!(t.style_for("keyword").bold, "Daruda keyword is bold");
    assert!(
        t.style_for("string.escape").bold,
        "Daruda string_special is bold"
    );
    assert!(t.style_for("comment").italic, "Daruda comment is italic");
    // A color-only palette carries no non-color channel.
    let one = syntax_theme_of(SyntaxPalette::OneDark, false);
    assert_eq!(one.style_for("keyword"), TokenStyle::default());
}
