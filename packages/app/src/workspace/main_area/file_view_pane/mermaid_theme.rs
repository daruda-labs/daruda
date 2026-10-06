//! Maps daruda's active palette to the mermaid `themeVariables` a rendered
//! diagram should adopt, so a dark-mode diagram matches the app's actual
//! surface/text/border colors instead of the renderer's generic dark preset.

use gpui::Hsla;

use crate::ui::theme::{self, DarudaTheme};

/// Plain-data palette snapshot threaded down to the GPUI-free mermaid
/// renderer (`mermaid_host_theme::mermaid_host_theme_profile`). Resolved once
/// here, at the GPUI boundary, so background-thread code never touches
/// `Hsla`. Field set mirrors merman's `HostThemeRoles` — a diagram-type-
/// agnostic role palette, not per-diagram-type `themeVariables` — so every
/// diagram kind (flowchart, sequence, pie, ...) picks up daruda's actual
/// surface/text/border colors instead of leaving diagram-specific elements
/// (sequence notes/actors, pie background, ...) on mermaid's own light
/// defaults.
#[derive(Clone)]
pub(in crate::workspace) struct MermaidPalette {
    pub dark: bool,
    pub background: String,
    pub primary_color: String,
    pub primary_text_color: String,
    pub primary_border_color: String,
    pub line_color: String,
    pub secondary_color: String,
    pub surface_muted: String,
    pub cluster_background: String,
    pub note_background: String,
    pub note_text: String,
    pub activation_background: String,
    pub error: String,
    pub warning: String,
    pub success: String,
}

impl MermaidPalette {
    pub fn from_theme(theme: &DarudaTheme) -> Self {
        let canvas = theme.file_viewer_bg;
        Self {
            dark: theme.is_dark(),
            background: to_hex(canvas),
            // `md_code_block_bg`/`md_code_inline_bg`/`dock_bg` sit on daruda's
            // panel-elevation ladder, which is deliberately subtle
            // (a few % lightness apart) for panel-on-panel chrome — too close to
            // `canvas` to read as a filled node/section on open diagram canvas
            // (mindmap topics, timeline sections have no border to compensate, so
            // they rendered as near-invisible black-on-black boxes). `overlay_*`
            // already encodes the right *direction* per theme (white-alpha in
            // dark, black-alpha in light; see `daruda_light.json`), but even its
            // strongest step (`overlay_prominent`, 10% alpha) is tuned for barely-
            // perceptible chrome (hover/selected rows), not a standalone content
            // box — so `diagram_surface` reuses that hue at a level actually
            // meant to read as a filled card.
            primary_color: to_hex_over(diagram_surface(theme, DIAGRAM_SURFACE_ALPHA), canvas),
            primary_text_color: to_hex(theme.text_body),
            primary_border_color: to_hex(theme.border),
            line_color: to_hex(theme.text_muted),
            secondary_color: to_hex_over(diagram_surface(theme, DIAGRAM_SURFACE_ALT_ALPHA), canvas),
            surface_muted: to_hex_over(diagram_surface(theme, DIAGRAM_SURFACE_ALT_ALPHA), canvas),
            cluster_background: to_hex_over(
                diagram_surface(theme, DIAGRAM_SURFACE_ALT_ALPHA),
                canvas,
            ),
            // `banner_warning_bg` is a translucent tint (`with_alpha(WARNING, 0.10)`)
            // meant to composite over a panel, not stand alone — flattening it to
            // opaque RGB without compositing would emit a full-intensity warning
            // color instead of the intended subtle tint.
            note_background: to_hex_over(theme.banner_warning_bg, canvas),
            note_text: to_hex(theme.banner_warning_text),
            activation_background: to_hex_over(
                diagram_surface(theme, DIAGRAM_SURFACE_ALT_ALPHA),
                canvas,
            ),
            error: to_hex(theme.banner_error_text),
            warning: to_hex(theme.banner_warning_text),
            success: to_hex(theme.banner_success_text),
        }
    }

    pub fn from_file_viewer(cx: &gpui::App) -> Self {
        let ui_theme = cx.try_global::<DarudaTheme>().cloned().unwrap_or_default();
        let surface = theme::PaneSurfaceTokens::file_viewer(cx);
        let canvas = surface.background;
        let line = to_hex_over(surface.foreground_muted, canvas);
        let primary_surface = diagram_surface_for_base(canvas, DIAGRAM_SURFACE_ALPHA);
        let secondary_surface = diagram_surface_for_base(canvas, DIAGRAM_SURFACE_ALT_ALPHA);
        let primary_surface_hex = to_hex_over(primary_surface, canvas);
        let secondary_surface_hex = to_hex_over(secondary_surface, canvas);

        Self {
            dark: !surface.syntax_is_light,
            background: to_hex(canvas),
            primary_color: primary_surface_hex.clone(),
            primary_text_color: to_hex(surface.foreground),
            primary_border_color: to_hex_over(surface.border_tint, canvas),
            line_color: line,
            secondary_color: secondary_surface_hex.clone(),
            surface_muted: secondary_surface_hex.clone(),
            cluster_background: secondary_surface_hex.clone(),
            note_background: to_hex_over(ui_theme.banner_warning_bg, canvas),
            note_text: to_hex(ui_theme.banner_warning_text),
            activation_background: secondary_surface_hex,
            error: to_hex(ui_theme.banner_error_text),
            warning: to_hex(ui_theme.banner_warning_text),
            success: to_hex(ui_theme.banner_success_text),
        }
    }

    /// Agent-chat diagrams sit on the terminal-mirrored chat surface, which can
    /// disagree with the UI theme's light/dark bit. Build this palette from
    /// that actual surface/foreground pair so light UI chrome cannot leak
    /// black Mermaid text onto a dark chat transcript.
    pub fn from_agent_chat(cx: &gpui::App) -> Self {
        let ui_theme = cx.try_global::<DarudaTheme>().cloned().unwrap_or_default();
        let surface = theme::PaneSurfaceTokens::agent_chat(cx);
        let canvas = surface.background;
        let line = to_hex_over(surface.foreground_muted, canvas);
        let primary_surface = diagram_surface_for_base(canvas, DIAGRAM_SURFACE_ALPHA);
        let secondary_surface = diagram_surface_for_base(canvas, DIAGRAM_SURFACE_ALT_ALPHA);
        let primary_surface_hex = to_hex_over(primary_surface, canvas);
        let secondary_surface_hex = to_hex_over(secondary_surface, canvas);

        Self {
            dark: !surface.syntax_is_light,
            background: to_hex(canvas),
            primary_color: primary_surface_hex.clone(),
            primary_text_color: to_hex(surface.foreground),
            primary_border_color: to_hex_over(surface.border_tint, canvas),
            line_color: line,
            secondary_color: secondary_surface_hex.clone(),
            surface_muted: secondary_surface_hex.clone(),
            cluster_background: secondary_surface_hex.clone(),
            note_background: to_hex_over(ui_theme.banner_warning_bg, canvas),
            note_text: to_hex(ui_theme.banner_warning_text),
            activation_background: secondary_surface_hex,
            error: to_hex(ui_theme.banner_error_text),
            warning: to_hex(ui_theme.banner_warning_text),
            success: to_hex(ui_theme.banner_success_text),
        }
    }
}

impl Default for MermaidPalette {
    /// Falls back to the compile-time (dark) palette when no `DarudaTheme`
    /// global is installed yet.
    fn default() -> Self {
        Self::from_theme(&DarudaTheme::default())
    }
}

/// Alpha for a primary diagram surface (node/topic/section fill) — strong
/// enough to read as a filled card against `canvas` without a border to
/// help, unlike `overlay_prominent`'s 10% ceiling tuned for barely-visible
/// UI chrome (hover/selected rows).
const DIAGRAM_SURFACE_ALPHA: f32 = 0.22;
/// Alpha for a secondary diagram surface (edge-label tag, cluster fill,
/// activation bar) — visibly lighter than `canvas` but subordinate to
/// `DIAGRAM_SURFACE_ALPHA`.
const DIAGRAM_SURFACE_ALT_ALPHA: f32 = 0.14;

/// `overlay_prominent` with its hue/lightness kept (already the correct
/// *direction* per theme — white in dark, black in light) but its alpha
/// replaced, so a diagram surface can be stronger than any step on daruda's
/// actual UI-chrome overlay ladder while staying theme-coherent.
fn diagram_surface(theme: &DarudaTheme, alpha: f32) -> Hsla {
    Hsla {
        a: alpha,
        ..theme.overlay_prominent
    }
}

fn diagram_surface_for_base(base: Hsla, alpha: f32) -> Hsla {
    let overlay = if base.l < 0.5 {
        theme::OVERLAY_WHITE
    } else {
        theme::OVERLAY_BLACK
    };
    Hsla {
        a: alpha,
        ..overlay
    }
}

fn to_hex(color: Hsla) -> String {
    let rgb = color.to_rgb();
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgb.r * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb.g * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb.b * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// Alpha-composites `fg` over the opaque `base`, then hex-encodes the result.
/// mermaid's `themeVariables`/host-theme roles carry no alpha channel, so a
/// translucent daruda token (a banner tint, an overlay) needs to be flattened
/// against the surface it actually sits on before it can be handed over —
/// otherwise the alpha is silently dropped and the raw, full-intensity hue
/// leaks through.
fn to_hex_over(fg: Hsla, base: Hsla) -> String {
    let fg_rgb = fg.to_rgb();
    let base_rgb = base.to_rgb();
    let a = fg_rgb.a;
    let blend = |f: f32, b: f32| f * a + b * (1.0 - a);
    format!(
        "#{:02x}{:02x}{:02x}",
        (blend(fg_rgb.r, base_rgb.r) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (blend(fg_rgb.g, base_rgb.g) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
        (blend(fg_rgb.b, base_rgb.b) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_hex_converts_pure_colors() {
        assert_eq!(
            to_hex(Hsla {
                h: 0.0,
                s: 0.0,
                l: 0.0,
                a: 1.0
            }),
            "#000000"
        );
        assert_eq!(
            to_hex(Hsla {
                h: 0.0,
                s: 0.0,
                l: 1.0,
                a: 1.0
            }),
            "#ffffff"
        );
    }

    #[test]
    fn to_hex_over_flattens_translucent_color_against_base() {
        let translucent_red = Hsla {
            h: 0.0,
            s: 1.0,
            l: 0.5,
            a: 0.1,
        };
        let black = Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 1.0,
        };
        // 10% red over black should land close to black, not full-intensity red.
        let composited = to_hex_over(translucent_red, black);
        assert_ne!(
            composited,
            to_hex(Hsla {
                a: 1.0,
                ..translucent_red
            })
        );
        assert_eq!(composited, "#1a0000");
    }

    #[test]
    fn from_theme_matches_is_dark() {
        let theme = DarudaTheme::default();
        let palette = MermaidPalette::from_theme(&theme);
        assert_eq!(palette.dark, theme.is_dark());
        assert!(palette.background.starts_with('#'));
    }

    #[gpui::test]
    fn from_agent_chat_uses_chat_surface_not_light_ui_text(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let light_ui = DarudaTheme {
                title_bar_bg: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.95,
                    a: 1.0,
                },
                text_body: Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 0.0,
                    a: 1.0,
                },
                ..Default::default()
            };
            cx.set_global(light_ui);
            theme::set_agent_chat_bg(cx, 0, 0, 0);
            theme::set_agent_chat_fg(cx, 255, 255, 255);

            let palette = MermaidPalette::from_agent_chat(cx);

            assert!(palette.dark);
            assert_eq!(palette.background, "#000000");
            assert_eq!(palette.primary_text_color, "#ffffff");
            assert_ne!(palette.primary_text_color, "#000000");
            assert_ne!(palette.line_color, "#000000");
        });
    }
}
