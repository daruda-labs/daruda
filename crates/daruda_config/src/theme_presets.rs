//! Built-in terminal color theme presets.
//!
//! Each preset defines a full `ColorConfig` (foreground, background, and the
//! 16-color ANSI palette). The special name `"default"` resolves to
//! `ColorConfig::default()` (xterm-compatible colors). The name `"custom"`
//! is reserved and causes the caller to use the `[colors]` section from
//! `config.toml` directly.
//!
//! `colors_for_preset` returns `None` for unknown preset names so callers can
//! decide whether to fall back to defaults or to report the error.

use crate::colors::{AnsiPalette, ColorConfig, HexColor};

/// Metadata for a single built-in preset — name + display label.
pub struct ThemePreset {
    /// The internal key used in `config.toml` (`theme.preset = "<name>"`).
    pub name: &'static str,
    /// The human-readable name shown in the Settings UI.
    pub display_name: &'static str,
}

/// All available built-in presets, in the order they appear in the Settings
/// dropdown. `"default"` is first; `"custom"` is intentionally absent (it is
/// a sentinel that means "use [colors] from config.toml").
pub const PRESETS: &[ThemePreset] = &[
    ThemePreset {
        name: "default",
        display_name: "Default",
    },
    ThemePreset {
        name: "dracula",
        display_name: "Dracula",
    },
    ThemePreset {
        name: "nord",
        display_name: "Nord",
    },
    ThemePreset {
        name: "one_dark",
        display_name: "One Dark",
    },
    ThemePreset {
        name: "solarized_dark",
        display_name: "Solarized Dark",
    },
    ThemePreset {
        name: "gruvbox_dark",
        display_name: "Gruvbox Dark",
    },
    ThemePreset {
        name: "catppuccin_mocha",
        display_name: "Catppuccin Mocha",
    },
    ThemePreset {
        name: "tokyo_night",
        display_name: "Tokyo Night",
    },
    ThemePreset {
        name: "custom",
        display_name: "Custom",
    },
];

/// Look up the `ColorConfig` for `name`. Returns `None` for unrecognised
/// names (including `"custom"`, which the caller must handle separately).
pub fn colors_for_preset(name: &str) -> Option<ColorConfig> {
    match name {
        "default" => Some(ColorConfig::default()),
        "dracula" => Some(dracula()),
        "nord" => Some(nord()),
        "one_dark" => Some(one_dark()),
        "solarized_dark" => Some(solarized_dark()),
        "gruvbox_dark" => Some(gruvbox_dark()),
        "catppuccin_mocha" => Some(catppuccin_mocha()),
        "tokyo_night" => Some(tokyo_night()),
        _ => None,
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Individual preset definitions
// ──────────────────────────────────────────────────────────────────────────────

fn h(r: u8, g: u8, b: u8) -> HexColor {
    HexColor::new(r, g, b)
}

fn dracula() -> ColorConfig {
    ColorConfig {
        foreground: h(0xf8, 0xf8, 0xf2),
        background: h(0x28, 0x2a, 0x36),
        normal: AnsiPalette {
            black: h(0x21, 0x22, 0x2c),
            red: h(0xff, 0x55, 0x55),
            green: h(0x50, 0xfa, 0x7b),
            yellow: h(0xf1, 0xfa, 0x8c),
            blue: h(0xbd, 0x93, 0xf9),
            magenta: h(0xff, 0x79, 0xc6),
            cyan: h(0x8b, 0xe9, 0xfd),
            white: h(0xf8, 0xf8, 0xf2),
        },
        bright: AnsiPalette {
            black: h(0x62, 0x72, 0xa4),
            red: h(0xff, 0x6e, 0x6e),
            green: h(0x69, 0xff, 0x94),
            yellow: h(0xff, 0xff, 0xa5),
            blue: h(0xd6, 0xac, 0xff),
            magenta: h(0xff, 0x92, 0xdf),
            cyan: h(0xa4, 0xff, 0xff),
            white: h(0xff, 0xff, 0xff),
        },
    }
}

fn nord() -> ColorConfig {
    ColorConfig {
        foreground: h(0xd8, 0xde, 0xe9),
        background: h(0x2e, 0x34, 0x40),
        normal: AnsiPalette {
            black: h(0x3b, 0x42, 0x52),
            red: h(0xbf, 0x61, 0x6a),
            green: h(0xa3, 0xbe, 0x8c),
            yellow: h(0xeb, 0xcb, 0x8b),
            blue: h(0x81, 0xa1, 0xc1),
            magenta: h(0xb4, 0x8e, 0xad),
            cyan: h(0x88, 0xc0, 0xd0),
            white: h(0xe5, 0xe9, 0xf0),
        },
        bright: AnsiPalette {
            black: h(0x4c, 0x56, 0x6a),
            red: h(0xbf, 0x61, 0x6a),
            green: h(0xa3, 0xbe, 0x8c),
            yellow: h(0xeb, 0xcb, 0x8b),
            blue: h(0x81, 0xa1, 0xc1),
            magenta: h(0xb4, 0x8e, 0xad),
            cyan: h(0x8f, 0xbc, 0xbb),
            white: h(0xec, 0xef, 0xf4),
        },
    }
}

fn one_dark() -> ColorConfig {
    ColorConfig {
        foreground: h(0xab, 0xb2, 0xbf),
        background: h(0x28, 0x2c, 0x34),
        normal: AnsiPalette {
            black: h(0x28, 0x2c, 0x34),
            red: h(0xe0, 0x6c, 0x75),
            green: h(0x98, 0xc3, 0x79),
            yellow: h(0xe5, 0xc0, 0x7b),
            blue: h(0x61, 0xaf, 0xef),
            magenta: h(0xc6, 0x78, 0xdd),
            cyan: h(0x56, 0xb6, 0xc2),
            white: h(0xab, 0xb2, 0xbf),
        },
        bright: AnsiPalette {
            black: h(0x5c, 0x63, 0x70),
            red: h(0xe0, 0x6c, 0x75),
            green: h(0x98, 0xc3, 0x79),
            yellow: h(0xe5, 0xc0, 0x7b),
            blue: h(0x61, 0xaf, 0xef),
            magenta: h(0xc6, 0x78, 0xdd),
            cyan: h(0x56, 0xb6, 0xc2),
            white: h(0xff, 0xff, 0xff),
        },
    }
}

fn solarized_dark() -> ColorConfig {
    ColorConfig {
        foreground: h(0x83, 0x94, 0x96),
        background: h(0x00, 0x2b, 0x36),
        normal: AnsiPalette {
            black: h(0x07, 0x36, 0x42),
            red: h(0xdc, 0x32, 0x2f),
            green: h(0x85, 0x99, 0x00),
            yellow: h(0xb5, 0x89, 0x00),
            blue: h(0x26, 0x8b, 0xd2),
            magenta: h(0xd3, 0x36, 0x82),
            cyan: h(0x2a, 0xa1, 0x98),
            white: h(0xee, 0xe8, 0xd5),
        },
        bright: AnsiPalette {
            black: h(0x00, 0x2b, 0x36),
            red: h(0xcb, 0x4b, 0x16),
            green: h(0x58, 0x6e, 0x75),
            yellow: h(0x65, 0x7b, 0x83),
            blue: h(0x83, 0x94, 0x96),
            magenta: h(0x6c, 0x71, 0xc4),
            cyan: h(0x93, 0xa1, 0xa1),
            white: h(0xfd, 0xf6, 0xe3),
        },
    }
}

fn gruvbox_dark() -> ColorConfig {
    ColorConfig {
        foreground: h(0xeb, 0xdb, 0xb2),
        background: h(0x28, 0x28, 0x28),
        normal: AnsiPalette {
            black: h(0x28, 0x28, 0x28),
            red: h(0xcc, 0x24, 0x1d),
            green: h(0x98, 0x97, 0x1a),
            yellow: h(0xd7, 0x99, 0x21),
            blue: h(0x45, 0x85, 0x88),
            magenta: h(0xb1, 0x62, 0x86),
            cyan: h(0x68, 0x9d, 0x6a),
            white: h(0xa8, 0x99, 0x84),
        },
        bright: AnsiPalette {
            black: h(0x92, 0x83, 0x74),
            red: h(0xfb, 0x49, 0x34),
            green: h(0xb8, 0xbb, 0x26),
            yellow: h(0xfa, 0xbd, 0x2f),
            blue: h(0x83, 0xa5, 0x98),
            magenta: h(0xd3, 0x86, 0x9b),
            cyan: h(0x8e, 0xc0, 0x7c),
            white: h(0xeb, 0xdb, 0xb2),
        },
    }
}

fn catppuccin_mocha() -> ColorConfig {
    ColorConfig {
        foreground: h(0xcd, 0xd6, 0xf4),
        background: h(0x1e, 0x1e, 0x2e),
        normal: AnsiPalette {
            black: h(0x45, 0x47, 0x5a),
            red: h(0xf3, 0x8b, 0xa8),
            green: h(0xa6, 0xe3, 0xa1),
            yellow: h(0xf9, 0xe2, 0xaf),
            blue: h(0x89, 0xb4, 0xfa),
            magenta: h(0xf5, 0xc2, 0xe7),
            cyan: h(0x94, 0xe2, 0xd5),
            white: h(0xba, 0xc2, 0xde),
        },
        bright: AnsiPalette {
            black: h(0x58, 0x5b, 0x70),
            red: h(0xf3, 0x8b, 0xa8),
            green: h(0xa6, 0xe3, 0xa1),
            yellow: h(0xf9, 0xe2, 0xaf),
            blue: h(0x89, 0xb4, 0xfa),
            magenta: h(0xf5, 0xc2, 0xe7),
            cyan: h(0x94, 0xe2, 0xd5),
            white: h(0xa6, 0xad, 0xc8),
        },
    }
}

fn tokyo_night() -> ColorConfig {
    ColorConfig {
        foreground: h(0xc0, 0xca, 0xf5),
        background: h(0x1a, 0x1b, 0x26),
        normal: AnsiPalette {
            black: h(0x15, 0x16, 0x1e),
            red: h(0xf7, 0x76, 0x8e),
            green: h(0x9e, 0xce, 0x6a),
            yellow: h(0xe0, 0xaf, 0x68),
            blue: h(0x7a, 0xa2, 0xf7),
            magenta: h(0xbb, 0x9a, 0xf7),
            cyan: h(0x7d, 0xcf, 0xff),
            white: h(0xa9, 0xb1, 0xd6),
        },
        bright: AnsiPalette {
            black: h(0x41, 0x48, 0x68),
            red: h(0xf7, 0x76, 0x8e),
            green: h(0x9e, 0xce, 0x6a),
            yellow: h(0xe0, 0xaf, 0x68),
            blue: h(0x7a, 0xa2, 0xf7),
            magenta: h(0xbb, 0x9a, 0xf7),
            cyan: h(0x7d, 0xcf, 0xff),
            white: h(0xc0, 0xca, 0xf5),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_presets_return_some() {
        for preset in PRESETS {
            if preset.name == "custom" {
                assert!(colors_for_preset(preset.name).is_none());
            } else {
                assert!(
                    colors_for_preset(preset.name).is_some(),
                    "preset {} returned None",
                    preset.name
                );
            }
        }
    }

    #[test]
    fn unknown_preset_returns_none() {
        assert!(colors_for_preset("__unknown__").is_none());
        assert!(colors_for_preset("").is_none());
    }

    #[test]
    fn default_preset_matches_color_config_default() {
        let preset = colors_for_preset("default").unwrap();
        let default = ColorConfig::default();
        assert_eq!(preset.foreground, default.foreground);
        assert_eq!(preset.background, default.background);
    }

    #[test]
    fn dracula_has_dark_background() {
        let c = colors_for_preset("dracula").unwrap();
        assert_eq!(c.background, HexColor::new(0x28, 0x2a, 0x36));
    }

    #[test]
    fn all_presets_have_non_empty_palette() {
        for preset in PRESETS {
            let Some(c) = colors_for_preset(preset.name) else {
                continue;
            };
            assert_ne!(c.foreground, c.background, "fg == bg for {}", preset.name);
        }
    }
}
