//! Host theming for merman-rendered mermaid diagrams. Left alone a diagram
//! paints Mermaid.js's own palette, which has no relation to the pane it lands
//! on; colours come from [`MermaidPalette`] and this decides where each goes.
//!
//! [`mermaid_render_profile`] separates two things one merman profile carries
//! together: the **colours**, which an author opts out of by declaring their
//! own theme, and the **rasterizer settings**, which nobody opts out of.
//!
//! [`mermaid_svg_render_options`] is the viewBox geometry every diagram is
//! laid out under.

use crate::MermaidPalette;

/// Host colors and the SVG output contract are independent so authored themes
/// can opt out of colors while retaining labels compatible with resvg.
pub(super) struct MermaidRenderProfile {
    pub(super) theme: Option<merman::svg::HostTheme>,
    pub(super) output: merman::svg::SvgOutputPolicy,
    pub(super) site_config: serde_json::Value,
}

impl MermaidRenderProfile {
    pub(super) fn render(
        &self,
        source: &str,
    ) -> Result<Option<String>, merman::render::RenderError> {
        use merman::svg::{
            MeasurementProfileId, Presentation, SvgEnvironment, TextMeasurementPolicy,
            TextMeasurementProfile, TextMeasurementProfileIdentity,
        };
        use merman::{
            Engine, MermaidConfig, OperationControl, RenderOutput, RenderRequest, Renderer,
            SvgRequest,
        };
        let mut presentation = Presentation::new();
        if let Some(theme) = &self.theme {
            presentation = presentation.with_theme(theme.clone());
        }
        let presentation = presentation.resolve();
        let engine = presentation.materialize_engine(
            Engine::new().with_site_config(MermaidConfig::from_value(self.site_config.clone())),
        );
        let identity = TextMeasurementProfileIdentity::new(
            MeasurementProfileId::new("daruda-hangul").expect("static measurement profile id"),
            "1",
        )
        .expect("static measurement profile version");
        let measurement = TextMeasurementPolicy::uniform(TextMeasurementProfile::new(
            identity,
            super::mermaid_text_measurer::host_text_measurer(),
        ));
        let request = SvgRequest {
            environment: SvgEnvironment::deterministic().with_text_measurement_policy(measurement),
            options: mermaid_svg_render_options(),
            pipeline: Some(self.output.pipeline()),
            presentation: presentation.render_policy(),
            ..SvgRequest::default()
        };
        match Renderer::new()
            .with_engine(engine)
            .render(RenderRequest::svg(
                source,
                OperationControl::default(),
                request,
            ))? {
            RenderOutput::Svg(output) => Ok(output.map(|output| output.svg().to_owned())),
            _ => Ok(None),
        }
    }
}

pub(super) fn mermaid_render_profile(
    source: &str,
    palette: &MermaidPalette,
) -> MermaidRenderProfile {
    if source_declares_own_theme(source) {
        mermaid_author_theme_profile()
    } else {
        mermaid_host_theme_profile(palette)
    }
}

fn mermaid_author_theme_profile() -> MermaidRenderProfile {
    MermaidRenderProfile {
        theme: None,
        output: merman::svg::SvgOutputPolicy {
            preset: merman::svg::SvgPipelinePreset::ResvgSafe,
            ..Default::default()
        },
        site_config: serde_json::json!({"htmlLabels": false}),
    }
}

pub(super) fn mermaid_host_theme_profile(palette: &MermaidPalette) -> MermaidRenderProfile {
    use merman::svg::{HostTheme, HostThemeAppearance, ThemeRole};
    let mut theme = HostTheme::new().with_appearance(if palette.dark {
        HostThemeAppearance::Dark
    } else {
        HostThemeAppearance::Light
    });
    for (role, color) in [
        (ThemeRole::Canvas, &palette.background),
        (ThemeRole::Surface, &palette.primary_color),
        (ThemeRole::SurfaceAlt, &palette.secondary_color),
        (ThemeRole::SurfaceMuted, &palette.surface_muted),
        (ThemeRole::Text, &palette.primary_text_color),
        (ThemeRole::SubtleText, &palette.line_color),
        (ThemeRole::Border, &palette.primary_border_color),
        (ThemeRole::Line, &palette.line_color),
        (ThemeRole::EdgeLabelBackground, &palette.background),
        (ThemeRole::ClusterBackground, &palette.cluster_background),
        (ThemeRole::ClusterBorder, &palette.primary_border_color),
        (ThemeRole::NoteBackground, &palette.note_background),
        (ThemeRole::NoteBorder, &palette.warning),
        (ThemeRole::NoteText, &palette.note_text),
        (ThemeRole::ActorBackground, &palette.primary_color),
        (ThemeRole::ActorBorder, &palette.primary_border_color),
        (ThemeRole::ActorText, &palette.primary_text_color),
        (
            ThemeRole::ActivationBackground,
            &palette.activation_background,
        ),
        (ThemeRole::ActivationBorder, &palette.primary_border_color),
        (ThemeRole::Error, &palette.error),
        (ThemeRole::Warning, &palette.warning),
        (ThemeRole::Success, &palette.success),
    ] {
        theme = theme
            .try_with_role(role, color.clone())
            .expect("host palette contains CSS colors");
    }
    theme = theme
        .try_with_series_palette(if palette.dark {
            MERMAID_SERIES_PALETTE_DARK
        } else {
            MERMAID_SERIES_PALETTE_LIGHT
        })
        .expect("static series palette contains CSS colors");
    MermaidRenderProfile {
        theme: Some(theme),
        output: merman::svg::SvgOutputPolicy {
            preset: merman::svg::SvgPipelinePreset::ResvgSafe,
            root_background_color: Some(MERMAID_ROOT_BACKGROUND.to_owned()),
            css_override_policy: merman::svg::CssOverridePolicy::StripExistingImportant,
            scoped_css: Some(mermaid_host_scoped_css(palette)),
            ..Default::default()
        },
        site_config: serde_json::json!({"htmlLabels": false}),
    }
}

pub(super) fn mermaid_svg_render_options() -> merman::svg::SvgRenderOptions {
    merman::svg::SvgRenderOptions {
        viewbox_padding: MERMAID_VIEWBOX_PADDING,
        ..merman::svg::SvgRenderOptions::default()
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

/// Config keys through which a source picks its own colours. A source setting
/// none of them says nothing about colour, so it is not an opt-out.
const THEME_DECLARING_KEYS: [&str; 3] = ["theme", "themeVariables", "themeCSS"];

/// Whether `source` picks its own colours, in either place mermaid accepts a
/// theme: an `%%{init: ...}%%` directive or a leading `---` frontmatter block.
///
/// Host colours travel through site config, which wins over the document's own
/// config wholesale rather than merging per-field, so it is all or nothing.
/// Colour is all it covers — the rasterizer settings in
/// [`resvg_compatible_profile`] apply to an opted-out diagram either way.
fn source_declares_own_theme(source: &str) -> bool {
    init_directive_bodies(source).any(|body| {
        THEME_DECLARING_KEYS
            .iter()
            .any(|key| declares_key(body, key))
    }) || frontmatter_config_declares_theme(source)
}

/// Body of every terminated `%%{init ...}%%` / `%%{initialize ...}%%` block in
/// `source`, directive name included. An unterminated block is skipped —
/// mermaid wouldn't apply it either.
fn init_directive_bodies(source: &str) -> impl Iterator<Item = &str> {
    source
        .split("%%{")
        .skip(1)
        .filter_map(|rest| rest.split_once("}%%").map(|(body, _)| body))
        .filter(|body| body.trim_start().starts_with("init"))
}

/// Whether `source`'s leading `---` frontmatter picks colours under its
/// `config:` mapping. That mapping is the only frontmatter home for a theme —
/// merman maps no top-level `theme:` into the diagram config.
fn frontmatter_config_declares_theme(source: &str) -> bool {
    let Some(frontmatter) = frontmatter_block(source) else {
        return false;
    };
    let mut config_indent = None;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        // A flow mapping (`config: {theme: x}`) carries the keys on this line.
        if let Some(inline) = trimmed.strip_prefix("config:") {
            config_indent = Some(indent);
            if THEME_DECLARING_KEYS
                .iter()
                .any(|key| declares_key(inline, key))
            {
                return true;
            }
        } else if config_indent.is_some_and(|base| indent > base) {
            if THEME_DECLARING_KEYS
                .iter()
                .any(|key| yaml_line_declares_key(trimmed, key))
            {
                return true;
            }
        } else {
            config_indent = None;
        }
    }
    false
}

/// Contents of `source`'s leading `---` … `---` frontmatter, terminator
/// required — an unterminated block is not frontmatter to mermaid either.
fn frontmatter_block(source: &str) -> Option<&str> {
    let rest = source.strip_prefix("---")?;
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))?;
    let mut end = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(&rest[..end]);
        }
        end += line.len();
    }
    None
}

/// Whether the YAML line `trimmed` opens a mapping entry named `key`.
fn yaml_line_declares_key(trimmed: &str, key: &str) -> bool {
    let unquoted = trimmed.strip_prefix(['\'', '"']).unwrap_or(trimmed);
    unquoted.strip_prefix(key).is_some_and(|rest| {
        rest.strip_prefix(['\'', '"'])
            .unwrap_or(rest)
            .trim_start()
            .starts_with(':')
    })
}

/// Whether `body` uses `key` in key position — `key:` or `'key':` opening an
/// object entry, rather than a value or a longer key that merely starts with
/// it (`themeVariables` must not read as `theme`).
///
/// The scan is depth-blind and does not exclude directive-shaped text inside a
/// node label: a false positive costs only host chrome, never a rendered label.
fn declares_key(body: &str, key: &str) -> bool {
    let mut offset = 0;
    while let Some(at) = body[offset..].find(key) {
        let start = offset + at;
        let end = start + key.len();
        offset = end;

        let before = body[..start].trim_end();
        let before = before.strip_suffix(['\'', '"']).unwrap_or(before);
        let opens_an_entry = matches!(before.trim_end().chars().next_back(), Some('{' | ','));

        let after = body[end..].trim_start();
        let after = after.strip_prefix(['\'', '"']).unwrap_or(after);

        if opens_an_entry && after.trim_start().starts_with(':') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
