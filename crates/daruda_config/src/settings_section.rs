//! Identifier for a single page in the Settings window.
//!
//! Lives in `daruda_config` so the same enum can be referenced by:
//! - the in-app GUI router (`app/src/settings_window/`),
//! - keybinding-override deserialization in
//!   `app/src/surface/action_map.rs`,
//! - any future CLI / plugin entry point that needs to address a
//!   specific settings page.
//!
//! The two-layer shape (`SettingsSection { Builtin(...) }`) leaves a
//! seam for a future `Plugin(PluginSectionId)` variant when daruda
//! grows a plugin system: every match site already routes through the
//! outer enum, so a new `Plugin` arm becomes a compiler-enforced
//! follow-up rather than a sweeping refactor.
//!
//! User-visible labels live in `app/src/surface/strings.rs` (G4); this
//! crate ships only the closed enum + slug helpers used for routing.

/// A page in the Settings window.
///
/// Phase 1 only ever constructs `Builtin(...)` variants. The
/// `non_exhaustive` `Plugin(...)` placeholder is **not** added yet —
/// adding it now would force every match site to handle a variant
/// that has no constructor. The forward path is: when plugins land,
/// add `Plugin(PluginSectionId)` here and let the compiler walk every
/// `match SettingsSection { ... }` site.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    Builtin(BuiltinSection),
}

impl Default for SettingsSection {
    fn default() -> Self {
        Self::Builtin(BuiltinSection::default())
    }
}

impl From<BuiltinSection> for SettingsSection {
    fn from(b: BuiltinSection) -> Self {
        Self::Builtin(b)
    }
}

/// Built-in settings page kinds. Order in `ALL` is the order they
/// appear in the left dock.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum BuiltinSection {
    #[default]
    General,
    Font,
    Cursor,
    Shell,
    Window,
    Terminal,
    LeftDock,
    Clipboard,
    Panels,
    ClaudeStatus,
    Notifications,
    Keymap,
    Plugin,
}

impl BuiltinSection {
    /// Dock order. Add a variant here to make it discoverable.
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::Font,
        Self::Cursor,
        Self::Shell,
        Self::Window,
        Self::Terminal,
        Self::LeftDock,
        Self::Clipboard,
        Self::Panels,
        Self::ClaudeStatus,
        Self::Notifications,
        Self::Keymap,
        Self::Plugin,
    ];

    /// Stable slug used by config keybinding overrides
    /// (`open_settings.font`) and command-palette action ids
    /// (`open_settings_font`). Stable across releases; never
    /// translated.
    pub const fn slug(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Font => "font",
            Self::Cursor => "cursor",
            Self::Shell => "shell",
            Self::Window => "window",
            Self::Terminal => "terminal",
            Self::LeftDock => "left_dock",
            Self::Clipboard => "clipboard",
            Self::Panels => "panels",
            Self::ClaudeStatus => "claude_status",
            Self::Notifications => "notifications",
            Self::Keymap => "keymap",
            Self::Plugin => "plugin",
        }
    }

    /// Inverse of `slug` — returns `None` for unknown slugs so callers
    /// (config keybinding parser) can ignore typos rather than panic.
    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|b| b.slug() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_section_is_general() {
        assert_eq!(
            SettingsSection::default(),
            SettingsSection::Builtin(BuiltinSection::General)
        );
    }

    #[test]
    fn all_slugs_are_unique() {
        let mut slugs: Vec<_> = BuiltinSection::ALL.iter().map(|b| b.slug()).collect();
        slugs.sort();
        let dedup_len = {
            let mut copy = slugs.clone();
            copy.dedup();
            copy.len()
        };
        assert_eq!(slugs.len(), dedup_len, "slugs must be unique: {slugs:?}");
    }

    #[test]
    fn slug_round_trip() {
        for &b in BuiltinSection::ALL {
            assert_eq!(BuiltinSection::from_slug(b.slug()), Some(b));
        }
    }

    #[test]
    fn from_slug_unknown_returns_none() {
        assert_eq!(BuiltinSection::from_slug("nonexistent"), None);
        assert_eq!(BuiltinSection::from_slug(""), None);
    }

    #[test]
    fn slugs_are_snake_case() {
        for b in BuiltinSection::ALL {
            let s = b.slug();
            assert!(
                s.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "slug {s:?} should be snake_case"
            );
        }
    }

    #[test]
    fn all_contains_every_variant() {
        // Spot-check that ALL matches the enum: count must equal the
        // number of variants. If a future variant is added without
        // updating ALL, this test fails.
        //
        // NOTE: bump this number whenever a `BuiltinSection` variant is
        // added or removed. There is no `strum::EnumCount`-style helper
        // in this crate; the count exists precisely to force a manual
        // sync of `ALL` with the enum.
        assert_eq!(BuiltinSection::ALL.len(), 13);
    }
}
