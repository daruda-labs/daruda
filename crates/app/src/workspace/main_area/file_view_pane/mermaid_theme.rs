//! Maps daruda's active palette to the mermaid `themeVariables` a rendered
//! diagram should adopt, so a dark-mode diagram matches the app's actual
//! surface/text/border colors instead of selkie's generic dark preset.

use gpui::Hsla;

use crate::ui::theme::DarudaTheme;

/// Plain-data palette snapshot threaded down to the GPUI-free mermaid
/// renderer (`markdown_viewer::mermaid_with_theme`). Resolved once here, at
/// the GPUI boundary, so background-thread code never touches `Hsla`.
#[derive(Clone)]
pub(in crate::workspace) struct MermaidPalette {
    pub dark: bool,
    pub background: String,
    pub primary_color: String,
    pub primary_text_color: String,
    pub primary_border_color: String,
    pub line_color: String,
    pub secondary_color: String,
}

impl MermaidPalette {
    pub fn from_theme(theme: &DarudaTheme) -> Self {
        Self {
            dark: theme.is_dark(),
            background: to_hex(theme.file_viewer_bg),
            primary_color: to_hex(theme.md_code_block_bg),
            primary_text_color: to_hex(theme.text_body),
            primary_border_color: to_hex(theme.border),
            line_color: to_hex(theme.text_muted),
            secondary_color: to_hex(theme.md_code_inline_bg),
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

fn to_hex(color: Hsla) -> String {
    let rgb = color.to_rgb();
    format!(
        "#{:02x}{:02x}{:02x}",
        (rgb.r * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb.g * 255.0).round().clamp(0.0, 255.0) as u8,
        (rgb.b * 255.0).round().clamp(0.0, 255.0) as u8,
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
    fn from_theme_matches_is_dark() {
        let theme = DarudaTheme::default();
        let palette = MermaidPalette::from_theme(&theme);
        assert_eq!(palette.dark, theme.is_dark());
        assert!(palette.background.starts_with('#'));
    }
}
