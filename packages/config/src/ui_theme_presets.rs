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

/// `theme.ui_preset` value that follows the OS appearance. Not a bundled
/// theme itself: [`resolve`] maps it to [`SYSTEM_DARK`] or [`SYSTEM_LIGHT`].
pub const SYSTEM: &str = "system";

/// The bundled preset [`SYSTEM`] paints under a dark OS appearance.
pub const SYSTEM_DARK: &str = "daruda_dark";

/// The bundled preset [`SYSTEM`] paints under a light OS appearance.
pub const SYSTEM_LIGHT: &str = "daruda_light";

/// The bundled preset a configured `theme.ui_preset` paints when the OS
/// appearance is `dark`. Any name other than [`SYSTEM`] is its own answer.
pub fn resolve(name: &str, dark: bool) -> &str {
    match name {
        SYSTEM if dark => SYSTEM_DARK,
        SYSTEM => SYSTEM_LIGHT,
        other => other,
    }
}

/// Whether `name` matches one of the built-in UI presets.
pub fn is_known(name: &str) -> bool {
    PRESETS.iter().any(|p| p.name == name)
}
