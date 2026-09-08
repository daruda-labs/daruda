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

use super::mermaid_theme::MermaidPalette;

/// The merman profile a diagram renders under: daruda's colours unless the
/// source declares its own theme, plus the rasterizer settings either way.
///
/// Applied unconditionally — skipping the profile to honour an author's theme
/// also drops [`resvg_compatible_profile`], which blanks every label.
pub(in crate::workspace) fn mermaid_render_profile(
    source: &str,
    palette: &MermaidPalette,
) -> merman::render::HostThemeProfile {
    if source_declares_own_theme(source) {
        mermaid_author_theme_profile()
    } else {
        mermaid_host_theme_profile(palette)
    }
}

/// The settings resvg compatibility demands of every diagram, whatever its
/// colours.
///
/// `htmlLabels` off because resvg has no `<foreignObject>` support — HTML
/// labels rasterize as boxes and arrows with no text, and SVG text is also what
/// lets a `classDef color:` reach the glyphs. It must be the root flag;
/// flowchart labels ignore the deprecated `flowchart.htmlLabels`. `ResvgSafe`
/// is the fallback if a source turns `htmlLabels` back on itself.
fn resvg_compatible_profile() -> merman::render::HostThemeProfileBuilder {
    merman::render::HostThemeProfile::builder()
        .site_config("htmlLabels", false)
        .output(resvg_compatible_output())
}

/// The output half of [`resvg_compatible_profile`], separate so the host-themed
/// profile extends it rather than restating the pipeline from its own preset.
fn resvg_compatible_output() -> merman::render::HostThemeOutput {
    merman::render::HostThemeOutput {
        pipeline: merman::render::HostThemePipelinePreset::ResvgSafe,
        ..merman::render::HostThemeOutput::default()
    }
}

/// Profile for a source that picked its own colours: rasterizer settings only.
///
/// Every field merman's `compile` reads for its `has_profile_theme_input` gate
/// stays default, so the compiled site config carries no `theme` / `darkMode`
/// to override the author's with. `css_override_policy` stays `Preserve` —
/// stripping `!important` exists to let host CSS win, and there is none here.
fn mermaid_author_theme_profile() -> merman::render::HostThemeProfile {
    resvg_compatible_profile().build()
}

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
    resvg_compatible_profile()
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
        // `Color(transparent)` rewrites per-diagram hardcoded backgrounds by
        // clearing them (usvg parses the non-standard root `background-color`;
        // `transparent` yields an alpha-0 fill). `None` would skip the
        // postprocessor and let hardcoded whites through. Stripping
        // `!important` is what lets the scoped host CSS win.
        .output(merman::render::HostThemeOutput {
            root_background: merman::render::HostThemeRootBackground::Color(
                MERMAID_ROOT_BACKGROUND.to_owned(),
            ),
            css_override_policy: merman::render::CssOverridePolicy::StripExistingImportant,
            scoped_css: Some(mermaid_host_scoped_css(palette)),
            ..resvg_compatible_output()
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
