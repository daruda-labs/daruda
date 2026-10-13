//! Public API tests for theme presets.

use daruda_config::theme_presets::*;
use daruda_config::{ColorConfig, HexColor};

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
