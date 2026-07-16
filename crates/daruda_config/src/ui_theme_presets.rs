//! Built-in UI theme presets — workspace chrome, docks, modal,
//! status bar, dock, agent panels, etc.
//!
//! Separate from `theme_presets` (terminal color palette) because the
//! two are independent axes — a user may run a Nord terminal palette
//! inside a daruda_dark chrome, or vice versa.

/// Metadata for a single built-in UI theme preset — name + display
/// label. The internal `name` is what `config.toml` stores under
/// `theme.ui_preset = "<name>"`.
pub struct UiThemePreset {
    /// The internal key used in `config.toml` (`theme.ui_preset = "<name>"`).
    pub name: &'static str,
    /// The human-readable name shown in the Settings UI.
    pub display_name: &'static str,
}

/// All built-in UI theme presets, in Settings-dropdown order: `daruda_dark`
/// (compile-time fallback) and `daruda_light` (overrides bundled in
/// `assets/themes/daruda_light.json`).
pub const PRESETS: &[UiThemePreset] = &[
    UiThemePreset {
        name: "daruda_dark",
        display_name: "Daruda Dark",
    },
    UiThemePreset {
        name: "daruda_light",
        display_name: "Daruda Light",
    },
];

/// Default UI preset name when the config is empty / fresh.
pub const DEFAULT: &str = "daruda_dark";

/// Whether `name` matches one of the built-in UI presets.
pub fn is_known(name: &str) -> bool {
    PRESETS.iter().any(|p| p.name == name)
}
