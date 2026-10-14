//! App-wide desktop lifecycle preferences; never overridden per project.

use serde::{Deserialize, Serialize};

/// When daruda holds the computer awake. Display sleep and the user's
/// lid-close power policy are never overridden.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeepAwakeMode {
    #[default]
    Off,
    /// While an agent is working or waiting on a permission approval.
    Agent,
    /// Whenever daruda is running.
    On,
}

impl KeepAwakeMode {
    /// The spelling `config.toml` uses, matching the serde names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Agent => "agent",
            Self::On => "on",
        }
    }
}

const DEFAULT_APPROVAL_WAIT_MINUTES: u64 = 30;
/// One day; an approval nobody answers by then is abandoned.
pub const MAX_APPROVAL_WAIT_MINUTES: u64 = 24 * 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(from = "DesktopFile")]
pub struct DesktopConfig {
    /// Show a Windows tray icon. Takes effect at the next launch.
    pub tray_enabled: bool,
    /// Hide a closing window while retaining its sessions, when a tray exists.
    /// Explicit Quit still uses the normal running-work and unsaved-edits gate.
    pub close_to_tray: bool,
    pub keep_awake: KeepAwakeMode,
    /// How long a pending permission approval keeps the computer awake.
    pub keep_awake_approval_wait_minutes: u64,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            tray_enabled: false,
            close_to_tray: false,
            keep_awake: KeepAwakeMode::Off,
            keep_awake_approval_wait_minutes: DEFAULT_APPROVAL_WAIT_MINUTES,
        }
    }
}

impl DesktopConfig {
    /// Zero minutes would be indistinguishable from not waiting at all; the
    /// upper bound keeps the value a sane span that fits a TOML integer.
    pub(crate) fn clamp(&mut self) {
        self.keep_awake_approval_wait_minutes = self
            .keep_awake_approval_wait_minutes
            .clamp(1, MAX_APPROVAL_WAIT_MINUTES);
    }
}

/// The `[desktop]` table as written on disk. `keep_awake` stays optional
/// here so an explicit value can win over the legacy flag.
#[derive(Deserialize)]
#[serde(default)]
struct DesktopFile {
    tray_enabled: bool,
    close_to_tray: bool,
    keep_awake: Option<KeepAwakeMode>,
    keep_awake_approval_wait_minutes: u64,
    /// The old Windows-only switch. Read once, folded into `keep_awake`,
    /// and never written back.
    keep_awake_while_working: Option<bool>,
}

impl Default for DesktopFile {
    fn default() -> Self {
        let config = DesktopConfig::default();
        Self {
            tray_enabled: config.tray_enabled,
            close_to_tray: config.close_to_tray,
            keep_awake: None,
            keep_awake_approval_wait_minutes: config.keep_awake_approval_wait_minutes,
            keep_awake_while_working: None,
        }
    }
}

impl From<DesktopFile> for DesktopConfig {
    fn from(file: DesktopFile) -> Self {
        let legacy = match file.keep_awake_while_working {
            Some(true) => KeepAwakeMode::Agent,
            Some(false) | None => KeepAwakeMode::Off,
        };
        Self {
            tray_enabled: file.tray_enabled,
            close_to_tray: file.close_to_tray,
            keep_awake: file.keep_awake.unwrap_or(legacy),
            keep_awake_approval_wait_minutes: file.keep_awake_approval_wait_minutes,
        }
    }
}
