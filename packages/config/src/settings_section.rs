//! Identifier for a single page in the Settings window.
//!
//! Lives in `daruda_config` so the same enum can be referenced by:
//! - the in-app GUI router (`app/src/settings/`),
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
//! User-visible labels live in `app/src/surface/strings/` (G4); this
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
    /// UI, terminal and syntax themes plus window opacity and blur.
    Appearance,
    Font,
    /// Shell, output, padding, cursor and clipboard for terminal panes.
    Terminal,
    /// Left-dock (sidebar/files) settings, the bottom-dock macro grid and
    /// the external editor — the workspace chrome around the panes.
    Workspace,
    Keymap,
    Agent,
    /// The resident agent `/daruda` talks to.
    Orchestrator,
    /// Named, reusable SSH/Docker host registry a lane's `session_host` can
    /// reference by id (`daruda_config::SessionHostEntry`) — companion to
    /// [`Self::Agent`], which still hosts the per-agent legacy transport
    /// fields the registry is meant to replace.
    SessionHosts,
    Accounts,
    Notifications,
    /// Telegram, Slack and Discord channels that reach agents remotely.
    RemoteControl,
    Plugin,
    About,
}

impl BuiltinSection {
    /// Dock order. Add a variant here to make it discoverable.
    pub const ALL: &'static [Self] = &[
        Self::General,
        Self::Appearance,
        Self::Font,
        Self::Terminal,
        Self::Workspace,
        Self::Keymap,
        Self::Agent,
        Self::Orchestrator,
        Self::SessionHosts,
        Self::Accounts,
        Self::Notifications,
        Self::RemoteControl,
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
            Self::Appearance => "appearance",
            Self::Font => "font",
            Self::Terminal => "terminal",
            Self::Workspace => "workspace",
            Self::Keymap => "keymap",
            Self::Agent => "agent",
            Self::Orchestrator => "orchestrator",
            Self::SessionHosts => "session_hosts",
            Self::Accounts => "accounts",
            Self::Notifications => "notifications",
            Self::RemoteControl => "remote_control",
            Self::Plugin => "plugin",
            Self::About => "about",
        }
    }

    /// Inverse of `slug` — returns `None` for unknown slugs so callers
    /// (config keybinding parser) can ignore typos rather than panic.
    /// Also accepts retired slugs no longer returned by `slug()` (see
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

    /// Retired slugs, mapped to the section that absorbed their content.
    /// Never returned by `slug()` — new code (nav labels, command-palette
    /// entries) only ever sees the current slugs; this is purely an
    /// input-compatibility shim for `from_slug`.
    fn legacy_slug(s: &str) -> Option<Self> {
        LEGACY_SLUGS
            .iter()
            .find(|(old, _)| *old == s)
            .map(|(_, section)| *section)
    }
}

/// Every slug a past release answered to, and where it lands now.
const LEGACY_SLUGS: &[(&str, BuiltinSection)] = &[
    ("window", BuiltinSection::Appearance),
    ("shell", BuiltinSection::Terminal),
    ("cursor", BuiltinSection::Terminal),
    ("clipboard", BuiltinSection::Terminal),
    ("dock", BuiltinSection::Workspace),
    ("sidebar", BuiltinSection::Workspace),
    ("file_viewer", BuiltinSection::Workspace),
    ("left_dock", BuiltinSection::Workspace),
    ("panels", BuiltinSection::Workspace),
    ("external_editor", BuiltinSection::Workspace),
    ("claude_status", BuiltinSection::Agent),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_slugs_are_never_returned_by_slug() {
        // `slug()` must stay canonical — only `from_slug` accepts the
        // retired names, so new code never round-trips through them.
        for &b in BuiltinSection::ALL {
            assert!(
                LEGACY_SLUGS.iter().all(|(old, _)| *old != b.slug()),
                "{} is both current and retired",
                b.slug()
            );
        }
    }
}
