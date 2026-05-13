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

pub(super) fn color_for_key(key: TextRunKey) -> gpui::Hsla {
    let mut color = hsla_from_rgb(key.fg);
    if key.flags & CELL_STYLE_FLAG_FAINT != 0 {
        color = color.alpha(0.65);
    }
    color
}

pub(super) fn text_run_for_key(base_font: &gpui::Font, key: TextRunKey, len: usize) -> TextRun {
    let font = font_for_flags(base_font, key.flags);
    let color = color_for_key(key);

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
