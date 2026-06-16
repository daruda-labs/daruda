//! Terminal cell style → GPUI render primitives.
//!
//! Maps ghostty cell-style flags (bold, italic, underline, …) and
//! foreground/background `Rgb` values to GPUI `Font`, `Hsla`, and
//! `TextRun` instances. Used by the element prepaint to build runs
//! for `ShapedLine` and by the cursor overlay for contrast.

use ghostty_vt::Rgb;
use gpui::{TextRun, UnderlineStyle, px};

use crate::ux::theme;

pub(super) const CELL_STYLE_FLAG_BOLD: u8 = 0x02;
pub(super) const CELL_STYLE_FLAG_ITALIC: u8 = 0x04;
pub(super) const CELL_STYLE_FLAG_UNDERLINE: u8 = 0x08;
pub(super) const CELL_STYLE_FLAG_FAINT: u8 = 0x10;
pub(super) const CELL_STYLE_FLAG_STRIKETHROUGH: u8 = 0x40;

/// Cache key for runs sharing identical visual style — bold/italic
/// switch the GPUI `Font`, fg + faint switch the color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TextRunKey {
    pub(super) fg: Rgb,
    pub(super) flags: u8,
}

/// Gray level the dim blend targets. iTerm2 uses the midpoint (0.5),
/// which washes a dark theme *lighter*; daruda targets a darker neutral
/// so an inactive pane recedes (reads as darkened) rather than washed.
const DIM_GRAY_LEVEL: f32 = 0.3;

/// Blend an `Hsla` toward [`DIM_GRAY_LEVEL`] by `amount` in sRGB space,
/// preserving alpha (so a transparent background stays equally
/// transparent — just dimmer). Same shape as iTerm2's
/// `colorDimmedBy:towardsGrayLevel:` with a darker target.
/// `amount <= 0.0` returns the color unchanged.
pub(super) fn dim_toward_gray(color: gpui::Hsla, amount: f32) -> gpui::Hsla {
    if amount <= 0.0 {
        return color;
    }
    let rgba = gpui::Rgba::from(color);
    let g = DIM_GRAY_LEVEL;
    gpui::Rgba {
        r: rgba.r * (1.0 - amount) + g * amount,
        g: rgba.g * (1.0 - amount) + g * amount,
        b: rgba.b * (1.0 - amount) + g * amount,
        a: rgba.a, // alpha preserved
    }
    .into()
}

pub(super) fn hsla_from_rgb(rgb: Rgb) -> gpui::Hsla {
    hsla_from_rgb_alpha(rgb, 1.0)
}

pub(super) fn hsla_from_rgb_alpha(rgb: Rgb, alpha: f32) -> gpui::Hsla {
    let rgba = gpui::Rgba {
        r: rgb.r as f32 / 255.0,
        g: rgb.g as f32 / 255.0,
        b: rgb.b as f32 / 255.0,
        a: alpha,
    };
    rgba.into()
}

/// Pick a high-contrast cursor color for the given background — black
/// on light, white on dark, with reduced alpha so glyphs underneath
/// remain readable.
pub(crate) fn cursor_color_for_background(background: Rgb) -> gpui::Hsla {
    let bg = hsla_from_rgb(background);
    let mut cursor = if bg.l > 0.6 {
        theme::CURSOR_DARK
    } else {
        theme::CURSOR_LIGHT
    };
    cursor.a = 0.72;
    cursor
}

pub(super) fn font_for_flags(base: &gpui::Font, flags: u8) -> gpui::Font {
    let mut font = base.clone();
    if flags & CELL_STYLE_FLAG_BOLD != 0 {
        font = font.bold();
    }
    if flags & CELL_STYLE_FLAG_ITALIC != 0 {
        font = font.italic();
    }
    font
}

pub(super) fn color_for_key(key: TextRunKey, dim_amount: f32) -> gpui::Hsla {
    let mut color = hsla_from_rgb(key.fg);
    if key.flags & CELL_STYLE_FLAG_FAINT != 0 {
        color = color.alpha(0.65);
    }
    dim_toward_gray(color, dim_amount)
}

pub(super) fn text_run_for_key(
    base_font: &gpui::Font,
    key: TextRunKey,
    len: usize,
    dim_amount: f32,
) -> TextRun {
    let font = font_for_flags(base_font, key.flags);
    let color = color_for_key(key, dim_amount);

    let underline = (key.flags & CELL_STYLE_FLAG_UNDERLINE != 0).then_some(UnderlineStyle {
        color: Some(color),
        thickness: px(theme::TERMINAL_UNDERLINE_THICKNESS),
        wavy: false,
    });

    let strikethrough =
        (key.flags & CELL_STYLE_FLAG_STRIKETHROUGH != 0).then_some(gpui::StrikethroughStyle {
            color: Some(color),
            thickness: px(theme::TERMINAL_UNDERLINE_THICKNESS),
        });

    TextRun {
        len,
        font,
        color,
        background_color: None,
        underline,
        strikethrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_of(color: gpui::Hsla) -> gpui::Rgba {
        gpui::Rgba::from(color)
    }

    #[test]
    fn dim_zero_is_identity() {
        let c = hsla_from_rgb(Rgb {
            r: 200,
            g: 100,
            b: 50,
        });
        assert_eq!(dim_toward_gray(c, 0.0), c);
        assert_eq!(dim_toward_gray(c, -1.0), c);
    }

    #[test]
    fn dim_blends_toward_gray_level() {
        let black = gpui::Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
        .into();
        let white = gpui::Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
        .into();

        // Blend toward DIM_GRAY_LEVEL by 0.4:
        //   black → g*0.4,   white → 0.6 + g*0.4.
        let g = DIM_GRAY_LEVEL;
        let expected_black = g * 0.4;
        let expected_white = 0.6 + g * 0.4;
        let dimmed_black = rgba_of(dim_toward_gray(black, 0.4));
        let dimmed_white = rgba_of(dim_toward_gray(white, 0.4));

        assert!((dimmed_black.r - expected_black).abs() < 1e-3);
        assert!((dimmed_black.b - expected_black).abs() < 1e-3);
        assert!((dimmed_white.r - expected_white).abs() < 1e-3);
        assert!((dimmed_white.g - expected_white).abs() < 1e-3);
    }

    #[test]
    fn dim_preserves_alpha() {
        let translucent = gpui::Rgba {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.37,
        }
        .into();
        let dimmed = rgba_of(dim_toward_gray(translucent, 0.4));
        assert!((dimmed.a - 0.37).abs() < 1e-3);
    }
}
