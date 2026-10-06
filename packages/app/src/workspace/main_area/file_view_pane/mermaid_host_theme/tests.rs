use super::*;

fn test_palette() -> MermaidPalette {
    MermaidPalette {
        dark: true,
        background: "#111111".to_owned(),
        primary_color: "#222222".to_owned(),
        primary_text_color: "#eeeeee".to_owned(),
        primary_border_color: "#333333".to_owned(),
        line_color: "#cccccc".to_owned(),
        secondary_color: "#444444".to_owned(),
        surface_muted: "#1a1a1a".to_owned(),
        cluster_background: "#1c1c1c".to_owned(),
        note_background: "#3a2a10".to_owned(),
        note_text: "#f0d9a0".to_owned(),
        activation_background: "#2a2a2a".to_owned(),
        error: "#ff6666".to_owned(),
        warning: "#ffcc66".to_owned(),
        success: "#66ff99".to_owned(),
    }
}

#[test]
fn mermaid_host_theme_profile_matches_appearance_and_palette() {
    let palette = test_palette();
    let dark = mermaid_host_theme_profile(&palette);
    assert_eq!(dark.appearance, merman::render::HostThemeAppearance::Dark);
    assert_eq!(
        dark.roles.canvas.as_deref(),
        Some(palette.background.as_str())
    );
    assert_eq!(
        dark.roles.text.as_deref(),
        Some(palette.primary_text_color.as_str())
    );

    let mut light = palette.clone();
    light.dark = false;
    let light_profile = mermaid_host_theme_profile(&light);
    assert_eq!(
        light_profile.appearance,
        merman::render::HostThemeAppearance::Light
    );
}

/// Regression guard: mindmap/timeline/pie/gitgraph sections don't read
/// `roles` at all — they cycle a categorical `series_palette`
/// (`cScaleN`/`git{N}`/`pie{N}`). An empty palette isn't "use mermaid's
/// default colors", it's "auto-derive from `surface`", which compounds
/// into near-black boxes on top of a dark `surface`. See the mindmap/
/// timeline "too black" report this guards against.
#[test]
fn mermaid_host_theme_profile_always_sets_a_series_palette() {
    assert!(
        !mermaid_host_theme_profile(&test_palette())
            .series_palette
            .is_empty()
    );
    let mut light = test_palette();
    light.dark = false;
    assert!(!mermaid_host_theme_profile(&light).series_palette.is_empty());
}

/// Regression guard: the root background must be patched to
/// `transparent` — `Canvas` would stamp an opaque rectangle that breaks
/// the translucent agent-chat card design, while `None` would skip the
/// rewrite and let per-diagram hardcoded white backgrounds through.
#[test]
fn mermaid_host_theme_profile_patches_root_background_transparent() {
    assert_eq!(
        mermaid_host_theme_profile(&test_palette())
            .output
            .root_background,
        merman::render::HostThemeRootBackground::Color("transparent".to_owned())
    );
}

#[test]
fn mermaid_svg_render_options_reserve_extra_edge_padding() {
    let opts = mermaid_svg_render_options();
    assert_eq!(opts.viewbox_padding, MERMAID_VIEWBOX_PADDING);
    assert!(opts.viewbox_padding > merman::render::SvgRenderOptions::default().viewbox_padding);
}

#[test]
fn mermaid_host_theme_profile_adds_host_text_and_timeline_overrides() {
    let css = mermaid_host_theme_profile(&test_palette())
        .output
        .scoped_css
        .expect("host profile should inject scoped CSS");
    for expected in [
        ".lineWrapper line { stroke: #cccccc !important; }",
        "text[fill=\"#000\"]",
        "text[style*=\"fill:#000\"]",
        ".messageText",
        ".titleText",
        ".classDiagramTitleText",
        ".taskText0",
        ".activeText0",
        ".grid .tick text",
        "fill: #eeeeee !important; stroke: none !important;",
        "color: #eeeeee !important;",
    ] {
        assert!(
            css.contains(expected),
            "scoped CSS missing {expected:?}: {css}"
        );
    }
    for removed in [
        ".nodeLabel",
        ".label text",
        ".label span",
        ".cluster-label span",
    ] {
        assert!(
            !css.contains(removed),
            "flowchart label override should preserve classDef text colors: {css}"
        );
    }
    assert_eq!(
        mermaid_host_theme_profile(&test_palette())
            .site_config
            .get("htmlLabels")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn rendered_mermaid_svg_uses_host_scoped_css_for_lines_text_and_titles() {
    let palette = test_palette();
    let profile = mermaid_host_theme_profile(&palette);
    for (name, source, expected) in [
        (
            "timeline",
            "timeline\n  section Collect\n    Receive : Validate\n",
            "#merman .lineWrapper line { stroke: #cccccc !important; }",
        ),
        (
            "sequence",
            "sequenceDiagram\n  participant A\n  participant B\n  A->>B: hello\n",
            "#merman .messageText",
        ),
        (
            "gantt",
            "gantt\n  title Host title\n  dateFormat YYYY-MM-DD\n  A :a1, 2026-07-31, 1d\n",
            "#merman .titleText",
        ),
        (
            "class",
            "classDiagram\n  class Agent\n  Agent : +heartbeat()\n",
            "#merman .classDiagramTitleText",
        ),
    ] {
        let svg = merman::render::HeadlessRenderer::new()
            .with_svg_options(mermaid_svg_render_options())
            .with_host_theme(&profile)
            .render_svg_sync(source)
            .expect("merman should render")
            .expect("diagram should be detected");
        assert!(
            svg.contains(expected),
            "{name} scoped override missing {expected:?} from SVG: {svg}"
        );
        assert!(
            svg.contains("fill: #eeeeee !important; stroke: none !important;"),
            "{name} host text fill missing from SVG: {svg}"
        );
    }
}

#[test]
fn rendered_gantt_svg_overrides_hardcoded_black_axis_labels() {
    let palette = test_palette();
    let profile = mermaid_host_theme_profile(&palette);
    let svg = merman::render::HeadlessRenderer::new()
        .with_svg_options(mermaid_svg_render_options())
        .with_host_theme(&profile)
        .render_svg_sync(
            "gantt\n  title 데이터 보관 정책 검증\n  dateFormat YYYY-MM-DD\n  axisFormat %m/%d\n  section Hot\n  Redis TTL 상태 :active, r1, 2026-07-31, 2d\n",
        )
        .expect("merman should render")
        .expect("diagram should be detected");

    assert!(
        svg.contains("fill=\"#000\""),
        "fixture should exercise merman's hardcoded black axis label path: {svg}"
    );
    assert!(
        svg.contains("#merman text[fill=\"#000\"]"),
        "hardcoded black text fill override missing from SVG: {svg}"
    );
    assert!(
        svg.contains("fill: #eeeeee !important; stroke: none !important;"),
        "host text fill override missing from SVG: {svg}"
    );
}

#[test]
fn rendered_flowchart_preserves_classdef_label_colors() {
    let palette = test_palette();
    let profile = mermaid_host_theme_profile(&palette);
    let svg = merman::render::HeadlessRenderer::new()
        .with_svg_options(mermaid_svg_render_options())
        .with_host_theme(&profile)
        .render_svg_sync(
            r##"flowchart TB
  subgraph API["API Gateway"]
A1["Ingress<br/>rate limit"]
A2["Auth<br/>JWT / API Key"]
  end

  subgraph CORE["Core Services"]
C1["Collector"]
C2["Rule Engine"]
C3["Notifier"]
  end

  subgraph STORE["Storage"]
S1[("Redis<br/>TTL cache")]
S2[("MariaDB<br/>metadata")]
S3[("Object Store<br/>parquet")]
  end

  A1 --> A2 --> C1
  C1 --> S1
  C1 --> S3
  C1 --> C2 --> S2
  C2 --> C3

  classDef edge fill:#e8f3ff,stroke:#2b6cb0,color:#102a43
  classDef core fill:#eefbea,stroke:#2f855a,color:#123524
  classDef store fill:#fff8db,stroke:#b7791f,color:#3d2c00

  class A1,A2 edge
  class C1,C2,C3 core
  class S1,S2,S3 store
"##,
        )
        .expect("merman should render")
        .expect("diagram should be detected");

    assert!(
        !svg.contains("merman-foreignobject-fallback"),
        "flowchart labels should use SVG text so classDef color targets them: {svg}"
    );
    for expected in [
        ".edge tspan{fill:#102a43;}",
        ".core tspan{fill:#123524;}",
        ".store tspan{fill:#3d2c00;}",
    ] {
        assert!(
            svg.contains(expected),
            "classDef text color rule missing {expected:?}: {svg}"
        );
    }
}

#[test]
fn rendered_light_mermaid_svg_keeps_host_text_color_for_readability() {
    let mut palette = test_palette();
    palette.dark = false;
    palette.primary_text_color = "#fafafa".to_owned();
    let profile = mermaid_host_theme_profile(&palette);
    let svg = merman::render::HeadlessRenderer::new()
        .with_svg_options(mermaid_svg_render_options())
        .with_host_theme(&profile)
        .render_svg_sync(
            "sequenceDiagram\n  participant Agent\n  participant API\n  Agent->>API: heartbeat\n",
        )
        .expect("merman should render")
        .expect("diagram should be detected");
    assert!(
        svg.contains("fill: #fafafa !important; stroke: none !important;"),
        "light host text color override missing from SVG: {svg}"
    );
}

#[test]
fn a_source_without_any_directive_keeps_host_chrome() {
    assert!(!source_declares_own_theme("graph TD\nA-->B"));
}

/// An `%%{init}%%` block that tunes layout or behaviour says nothing about
/// colour, so it must not cost the diagram daruda's chrome.
#[test]
fn a_non_theme_init_block_is_not_an_opt_out() {
    for source in [
        "%%{init: {'flowchart': {'curve': 'linear'}}}%%\ngraph TD\nA-->B",
        "%%{init: {\"securityLevel\": \"loose\"}}%%\ngraph TD\nA-->B",
        "%%{init: {'sequence': {'showSequenceNumbers': true}}}%%\nsequenceDiagram\nA->>B: hi",
        // A value that merely spells a theme key is not a declaration.
        "%%{init: {'flowchart': {'defaultRenderer': 'theme'}}}%%\ngraph TD\nA-->B",
        // Neither is prose outside any directive.
        "graph TD\nA[\"themeVariables: none\"]-->B",
    ] {
        assert!(
            !source_declares_own_theme(source),
            "should keep host chrome: {source}"
        );
    }
}

#[test]
fn every_theme_declaring_directive_spelling_opts_out() {
    for source in [
        "%%{init: {\"theme\":\"forest\"}}%%\ngraph TD\nA-->B",
        "%%{init: {'theme': 'neutral'}}%%\nflowchart TD\nA-->B",
        // A themeVariables- or themeCSS-only directive counts too: daruda's
        // host colours can't merge on top of it per-field.
        "%%{init: {\"themeVariables\": {\"primaryColor\": \"#ff0000\"}}}%%\ngraph TD\nA-->B",
        "%%{init: {'themeCSS': '.node rect { fill: red; }'}}%%\ngraph TD\nA-->B",
        "%%{initialize: {'theme': 'dark'}}%%\ngraph TD\nA-->B",
        "%%{ init : { theme : 'forest' } }%%\ngraph TD\nA-->B",
        "%%{init: {'flowchart': {'curve': 'linear'}, 'theme': 'neutral'}}%%\ngraph TD\nA-->B",
    ] {
        assert!(
            source_declares_own_theme(source),
            "should defer to the author: {source}"
        );
    }
}

/// The colour opt-out must not take the rasterizer settings with it: resvg
/// cannot paint `<foreignObject>`, so whichever branch of
/// [`mermaid_render_profile`] a diagram takes, dropping `htmlLabels: false`
/// costs it every node, cluster and edge label.
#[test]
fn every_profile_carries_the_rasterizer_settings() {
    for source in [
        "graph TD\nA-->B",
        "%%{init: {'theme': 'neutral'}}%%\ngraph TD\nA-->B",
    ] {
        let profile = mermaid_render_profile(source, &test_palette());
        assert_eq!(
            profile
                .site_config
                .get("htmlLabels")
                .and_then(serde_json::Value::as_bool),
            Some(false),
            "htmlLabels must stay off for: {source}"
        );
        assert_eq!(
            profile.output.pipeline,
            merman::render::HostThemePipelinePreset::ResvgSafe,
            "resvg-safe pipeline must stay on for: {source}"
        );
    }
}

#[test]
fn author_theme_profile_keeps_rasterizer_settings_and_no_colours() {
    let profile = mermaid_render_profile(
        "%%{init: {'theme': 'neutral'}}%%\ngraph TD\nA-->B",
        &test_palette(),
    );
    // Every input merman's `has_profile_theme_input` gate reads has to stay
    // empty, or `compile()` injects `theme: base` over the author's own.
    assert_eq!(
        profile.appearance,
        merman::render::HostThemeAppearance::Light
    );
    assert_eq!(profile.roles, merman::render::HostThemeRoles::default());
    assert!(profile.series_palette.is_empty());
    assert!(profile.theme_variables.is_empty());
    assert!(profile.font_family.is_none());
    assert!(profile.font_size.is_none());
    assert!(profile.output.scoped_css.is_none());
    assert_eq!(
        profile.output.root_background,
        merman::render::HostThemeRootBackground::None
    );
}

/// Frontmatter is the spelling mermaid recommends over `%%{init}%%`, and
/// merman merges it the same way, so it has to opt out the same way.
/// A theme only counts under `config:` — merman maps no top-level `theme:`.
#[test]
fn a_frontmatter_config_theme_opts_out() {
    for source in [
        "---\nconfig:\n  theme: forest\n---\nflowchart TD\nA-->B",
        "---\ntitle: Example\nconfig:\n  theme: neutral\n---\nflowchart TD\nA-->B",
        "---\nconfig:\n  themeVariables:\n    primaryColor: '#ff0000'\n---\ngraph TD\nA-->B",
        "---\nconfig: {theme: forest}\n---\ngraph TD\nA-->B",
    ] {
        assert!(
            source_declares_own_theme(source),
            "should defer to the author: {source:?}"
        );
    }

    for source in [
        // No colour key anywhere.
        "---\ntitle: Example\n---\nflowchart TD\nA-->B",
        "---\nconfig:\n  flowchart:\n    curve: linear\n---\nflowchart TD\nA-->B",
        // merman does not map a top-level `theme:`, so neither do we.
        "---\ntheme: forest\n---\nflowchart TD\nA-->B",
        // Unterminated: mermaid wouldn't read it as frontmatter either.
        "---\nconfig:\n  theme: forest\nflowchart TD\nA-->B",
        // A `---` that isn't the very first line is not frontmatter.
        "flowchart TD\nA-->B\n---\nconfig:\n  theme: forest\n---\n",
    ] {
        assert!(
            !source_declares_own_theme(source),
            "should keep host chrome: {source:?}"
        );
    }
}

/// End-to-end through the render funnel: an author-themed flowchart keeps its
/// own colours and still comes back as SVG text resvg can paint.
#[test]
fn author_themed_diagram_renders_svg_text_labels() {
    let source = concat!(
        "%%{init: {'theme': 'neutral'}}%%\n",
        "flowchart TD\n",
        "  subgraph S[\"Cluster title\"]\n",
        "    A[\"Alpha node\"] --> B[\"Beta node\"]\n",
        "  end\n",
        "  A -.->|Edge caption| B\n",
    );
    let palette = test_palette();
    let svg = super::super::visual::render_mermaid_svg(source, &palette).expect("svg");

    assert!(
        !svg.contains("<foreignObject"),
        "resvg cannot paint foreignObject labels: {svg}"
    );
    // merman splits a label into one `<tspan>` per word, so match words.
    for word in ["Alpha", "Beta", "Cluster", "title", "Edge", "caption"] {
        assert!(
            svg.contains(&format!("{word}</tspan>")),
            "word {word:?} is not tspan text: {svg}"
        );
    }
    assert!(
        !svg.contains(&palette.primary_text_color),
        "host text colour leaked into an author-themed diagram: {svg}"
    );
    // Absence of the host colour alone would also pass if merman fell back
    // to some third theme, so pin neutral's own node fill/stroke.
    assert!(
        svg.contains("fill:#eee;stroke:#999"),
        "the author's `neutral` theme was not the one applied: {svg}"
    );
}
