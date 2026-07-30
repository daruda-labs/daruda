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

/// A page in the Settings window. Only constructs `Builtin(...)` today; the
/// outer enum exists so a future `Plugin(PluginSectionId)` variant becomes a
/// compiler-enforced follow-up at every match site rather than a refactor.
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
    /// Left-dock (Sidebar/Files) settings + bottom-dock (macro grid)
    /// settings, combined — both are "dock configuration" from a user's
    /// perspective even though they're separate `Dock` instances
    /// internally (`workspace::layout::Dock` position=Left vs Bottom).
    Dock,
    Clipboard,
    Agent,
    Accounts,
    Notifications,
    Keymap,
    Plugin,
    About,
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
        Self::Dock,
        Self::Clipboard,
        Self::Agent,
        Self::Accounts,
        Self::Notifications,
        Self::Keymap,
        Self::Plugin,
        Self::About,
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
            Self::Dock => "dock",
            Self::Clipboard => "clipboard",
            Self::Agent => "agent",
            Self::Accounts => "accounts",
            Self::Notifications => "notifications",
            Self::Keymap => "keymap",
            Self::Plugin => "plugin",
            Self::About => "about",
        }
    }

    /// Inverse of `slug` — returns `None` for unknown slugs so callers
    /// (config keybinding parser) can ignore typos rather than panic.
    /// Also accepts pre-merge slugs no longer returned by `slug()` (see
    /// `legacy_slug`), so an existing user's `open_settings.<slug>`
    /// keybinding override keeps resolving after a section is merged or
    /// renamed instead of silently going dead.
    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|b| b.slug() == s)
            .or_else(|| Self::legacy_slug(s))
    }

    /// Pre-merge slugs kept recognizable after a `BuiltinSection`
    /// reclassification, mapped to the section that absorbed their
    /// content. Never returned by `slug()` — new code (nav labels,
    /// command-palette entries) only ever sees the current slugs; this
    /// is purely an input-compatibility shim for `from_slug`.
    fn legacy_slug(s: &str) -> Option<Self> {
        match s {
            // `left_dock` and `panels` merged into one `Dock` page.
            "left_dock" | "panels" => Some(Self::Dock),
            // `claude_status` became a subsection of `Agent`.
            "claude_status" => Some(Self::Agent),
            _ => None,
        }
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

    /// Regression: `open_settings.left_dock` / `.panels` /
    /// `.claude_status` keybinding overrides from a config written before
    /// the Dock/Agent reclassification must keep resolving instead of
    /// silently binding nothing.
    #[test]
    fn from_slug_accepts_legacy_left_dock_and_panels_as_dock() {
        assert_eq!(
            BuiltinSection::from_slug("left_dock"),
            Some(BuiltinSection::Dock)
        );
        assert_eq!(
            BuiltinSection::from_slug("panels"),
            Some(BuiltinSection::Dock)
        );
    }

    #[test]
    fn from_slug_accepts_legacy_claude_status_as_agent() {
        assert_eq!(
            BuiltinSection::from_slug("claude_status"),
            Some(BuiltinSection::Agent)
        );
    }

    #[test]
    fn legacy_slugs_are_never_returned_by_slug() {
        // `slug()` must stay canonical — only `from_slug` should accept
        // the retired names, so new code never round-trips through them.
        for &b in BuiltinSection::ALL {
            assert_ne!(b.slug(), "left_dock");
            assert_ne!(b.slug(), "panels");
            assert_ne!(b.slug(), "claude_status");
        }
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
        assert_eq!(BuiltinSection::ALL.len(), 14);
    }
}
