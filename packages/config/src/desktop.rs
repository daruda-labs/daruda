//! App-wide desktop lifecycle preferences; never overridden per project.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct DesktopConfig {
    /// Show a Windows tray icon. Takes effect at the next launch.
    pub tray_enabled: bool,
    /// Hide a closing window while retaining its sessions, when a tray exists.
    /// Explicit Quit still uses the normal running-work and unsaved-edits gate.
    pub close_to_tray: bool,
    /// Inhibit system sleep only while a pane reports active work.
    /// Display sleep and the user's lid-close power policy remain unchanged.
    pub keep_awake_while_working: bool,
}

#[cfg(test)]
mod tests {
    #[test]
    fn existing_configs_keep_previous_desktop_behavior() {
        let config: super::DesktopConfig = toml::from_str("").unwrap();
        assert!(!config.tray_enabled && !config.close_to_tray && !config.keep_awake_while_working);
    }
}
