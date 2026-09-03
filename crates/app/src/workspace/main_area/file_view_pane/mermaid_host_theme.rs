//! Host theming for merman-rendered mermaid diagrams.
//!
//! A diagram is rendered by merman into SVG and rasterized; left alone it
//! paints Mermaid.js's own palette, which has no relation to the pane it lands
//! on. [`mermaid_host_theme_profile`] and the scoped CSS under it put it on
//! daruda's surface instead. The colours themselves come from
//! [`MermaidPalette`]; this decides where each one goes.
//!
//! Two neighbours travel with it: [`mermaid_svg_render_options`], which is the
//! viewBox geometry every diagram is laid out under, and
//! [`source_has_own_theme_directive`], the opt-out — an author who wrote their
//! own `%%{init}%%` block gets no host theme at all.

use super::mermaid_theme::MermaidPalette;

/// Build the merman host-theme profile matching daruda's active appearance
/// (`palette`), so every diagram type — not just flowchart nodes — picks up
/// daruda's actual surface/text/border/note/actor colors instead of leaving
/// diagram-specific elements (sequence notes/actors, pie background, ...) on
/// mermaid's own hardcoded light defaults. The root background is force-
/// patched to `transparent` regardless of which diagram renderer produced
/// the SVG — the rewrite is what stops the hardcoded-white leak many diagram
/// types don't route through `themeVariables`, and transparency (rather than
/// an opaque `canvas` fill) lets the host surface show through, matching the
/// translucent-tint design language of agent-chat cards. Node/label fills
/// stay opaque (`MermaidPalette` flattens them against `canvas`) so text
/// keeps a solid backing.
pub(in crate::workspace) fn mermaid_host_theme_profile(
    palette: &MermaidPalette,
) -> merman::render::HostThemeProfile {
    merman::render::HostThemeProfile::builder()
        .appearance(if palette.dark {
            merman::render::HostThemeAppearance::Dark
        } else {
            merman::render::HostThemeAppearance::Light
        })
        .roles(merman::render::HostThemeRoles {
            canvas: Some(palette.background.clone()),
            surface: Some(palette.primary_color.clone()),
            surface_alt: Some(palette.secondary_color.clone()),
            surface_muted: Some(palette.surface_muted.clone()),
            text: Some(palette.primary_text_color.clone()),
            subtle_text: Some(palette.line_color.clone()),
            border: Some(palette.primary_border_color.clone()),
            line: Some(palette.line_color.clone()),
            edge_label_background: Some(palette.background.clone()),
            cluster_background: Some(palette.cluster_background.clone()),
            cluster_border: Some(palette.primary_border_color.clone()),
            note_background: Some(palette.note_background.clone()),
            note_border: Some(palette.warning.clone()),
            note_text: Some(palette.note_text.clone()),
            actor_background: Some(palette.primary_color.clone()),
            actor_border: Some(palette.primary_border_color.clone()),
            actor_text: Some(palette.primary_text_color.clone()),
            activation_background: Some(palette.activation_background.clone()),
            activation_border: Some(palette.primary_border_color.clone()),
            error: Some(palette.error.clone()),
            warning: Some(palette.warning.clone()),
            success: Some(palette.success.clone()),
        })
        // Mindmap/timeline sections, pie slices, and git-graph branches don't
        // read from `roles` at all — they cycle a categorical `series_palette`
        // (`cScaleN`/`git{N}`/`pie{N}`). Left empty, merman's "base" theme
        // auto-derives those from `surface`, compounding into more
        // near-black boxes on top of the ones `roles` already covers. daruda
        // has no categorical palette of its own, so borrow merman's — tuned
        // by its own authors for the same "editor preview on a dark/light
        // host" case this is.
        .series_palette(if palette.dark {
            MERMAID_SERIES_PALETTE_DARK
        } else {
            MERMAID_SERIES_PALETTE_LIGHT
        })
        // Flowchart node labels only honor the root `htmlLabels` flag, not the
        // deprecated `flowchart.htmlLabels` fallback. Keep host-themed output on
        // SVG text labels so classDef `color:` applies to the actual rendered
        // `["..."]` label glyphs after `resvg_safe_editor()` processing.
        .site_config("htmlLabels", false)
        // `resvg_safe_editor()` defaults the root background to the opaque
        // `canvas` role; `Color(transparent)` keeps its rewrite of per-
        // diagram hardcoded backgrounds while clearing them instead of
        // repainting (usvg parses the non-standard root `background-color`
        // and `transparent` yields an alpha-0 fill). `None` would skip the
        // postprocessor entirely and let hardcoded whites through.
        .output(merman::render::HostThemeOutput {
            root_background: merman::render::HostThemeRootBackground::Color(
                MERMAID_ROOT_BACKGROUND.to_owned(),
            ),
            scoped_css: Some(mermaid_host_scoped_css(palette)),
            ..merman::render::HostThemeOutput::resvg_safe_editor()
        })
        .build()
}

pub(in crate::workspace) fn mermaid_svg_render_options() -> merman::render::SvgRenderOptions {
    merman::render::SvgRenderOptions {
        viewbox_padding: MERMAID_VIEWBOX_PADDING,
        ..merman::render::SvgRenderOptions::default()
    }
}

fn mermaid_host_scoped_css(palette: &MermaidPalette) -> String {
    // Timeline connector lines read from `cScaleInv`, which is a label-contrast
    // color for each bright section fill and often resolves to black. Keep label
    // contrast intact, but draw timeline lines with the host structural line
    // color so dashed connectors stay visible on dark editor surfaces.
    let text = &palette.primary_text_color;
    format!(
        concat!(
            ".lineWrapper line {{ stroke: {line} !important; }}",
            " text[fill=\"#000\"],",
            " text[fill=\"#000000\"],",
            " text[fill=\"black\"],",
            " text[style*=\"fill:#000\"],",
            " text[style*=\"fill: #000\"],",
            " text[style*=\"fill:black\"],",
            " text[style*=\"fill: black\"] {{ fill: {text} !important; stroke: none !important; }}",
            " .messageText,",
            " text.actor > tspan,",
            " .labelText,",
            " .labelText > tspan,",
            " .loopText,",
            " .loopText > tspan,",
            " .sectionTitle,",
            " .sectionTitle > tspan,",
            " .titleText,",
            " .flowchartTitleText,",
            " .erDiagramTitleText,",
            " .statediagramTitleText,",
            " .requirementDiagramTitleText,",
            " .gitTitleText,",
            " .pieTitleText,",
            " .treemapTitle,",
            " .packetTitle,",
            " .radarTitle,",
            " .classTitleText,",
            " .classDiagramTitleText,",
            " g.classGroup text,",
            " .cluster-label text,",
            " .classLabel .label,",
            " .taskText,",
            " .taskText0,",
            " .taskText1,",
            " .taskText2,",
            " .taskText3,",
            " .taskTextOutsideLeft,",
            " .taskTextOutsideRight,",
            " .taskTextOutside0,",
            " .taskTextOutside1,",
            " .taskTextOutside2,",
            " .taskTextOutside3,",
            " .activeText0,",
            " .activeText1,",
            " .activeText2,",
            " .activeText3,",
            " .doneText0,",
            " .doneText1,",
            " .doneText2,",
            " .doneText3,",
            " .critText0,",
            " .critText1,",
            " .critText2,",
            " .critText3,",
            " .activeCritText0,",
            " .activeCritText1,",
            " .activeCritText2,",
            " .activeCritText3,",
            " .doneCritText0,",
            " .doneCritText1,",
            " .doneCritText2,",
            " .doneCritText3,",
            " .milestoneText,",
            " .grid .tick text {{ fill: {text} !important; stroke: none !important; }}",
            " .radarTitle,",
            " span[style*=\"color:#000\"],",
            " span[style*=\"color: #000\"],",
            " span[style*=\"color:black\"],",
            " span[style*=\"color: black\"] {{ color: {text} !important; }}",
        ),
        line = palette.line_color,
        text = text
    )
}

/// CSS color for the patched SVG root background: transparent, so the
/// diagram composites over whatever surface hosts it (agent-chat card
/// tint, file-viewer background) instead of stamping an opaque rectangle.
const MERMAID_ROOT_BACKGROUND: &str = "transparent";
const MERMAID_VIEWBOX_PADDING: f64 = 24.0;

const MERMAID_SERIES_PALETTE_DARK: [&str; 8] = [
    "#60a5fa", "#34d399", "#f59e0b", "#c084fc", "#22d3ee", "#fb7185", "#facc15", "#a3e635",
];
const MERMAID_SERIES_PALETTE_LIGHT: [&str; 8] = [
    "#2563eb", "#059669", "#d97706", "#7c3aed", "#0891b2", "#be123c", "#a16207", "#65a30d",
];

/// Whether `source` already carries its own `%%{init: ...}%%` directive —
/// if so, daruda's host theme is skipped entirely so the diagram author's
/// customization (theme name, individual `themeVariables`, `themeCSS`, ...)
/// isn't silently overridden. merman applies a renderer-level host theme via
/// site config, which wins over a document-level directive wholesale rather
/// than merging per-field, so partial-respect isn't possible here — an
/// author who wrote any `%%{init}%%` block opts out of daruda's chrome for
/// that diagram.
pub(in crate::workspace) fn source_has_own_theme_directive(source: &str) -> bool {
    source.contains("%%{init")
}

#[cfg(test)]
mod tests {
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
    fn source_has_own_theme_directive_detects_any_init_block() {
        assert!(!source_has_own_theme_directive("graph TD\nA-->B"));
        assert!(source_has_own_theme_directive(
            "%%{init: {\"theme\":\"forest\"}}%%\ngraph TD\nA-->B"
        ));
        // Even a themeVariables-only (no theme name) directive opts out —
        // daruda's host theme can't merge on top of it per-field.
        assert!(source_has_own_theme_directive(
            "%%{init: {\"themeVariables\": {\"primaryColor\": \"#ff0000\"}}}%%\ngraph TD\nA-->B"
        ));
    }
}
