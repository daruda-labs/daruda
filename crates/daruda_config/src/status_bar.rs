//! Status bar configuration: which segments the bottom status bar
//! shows. Toggled at runtime via the status bar's right-click menu
//! and persisted through the same config-save path as other
//! Settings-UI-controlled keys.

use serde::{Deserialize, Serialize};

/// A toggleable segment in the status bar. `ALL`'s order is both the
/// render order (left-to-right) and the order the right-click toggle
/// menu lists them in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusBarItem {
    ProjectBranch,
    AccountSlot,
    Ports,
    /// Plan-rate usage for the focused pane's Claude account. Claude
    /// only — Codex has no rate-limit backend to read from.
    ClaudeUsage,
}

impl StatusBarItem {
    /// Every known segment, in render/menu order. Add a variant here
    /// to make it toggleable and displayed. A config written before a
    /// variant existed lists only the older ones, so the new segment
    /// starts hidden for existing users until they tick it — the
    /// stored list is an explicit choice, not a partial snapshot.
    pub const ALL: &'static [Self] = &[
        Self::ProjectBranch,
        Self::AccountSlot,
        Self::Ports,
        Self::ClaudeUsage,
    ];
}

/// Which status bar segments are visible.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct StatusBarConfig {
    /// Segments to render. Defaults to [`StatusBarItem::ALL`] so a
    /// fresh config shows every segment the status bar can display.
    pub visible_items: Vec<StatusBarItem>,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            visible_items: StatusBarItem::ALL.to_vec(),
        }
    }
}

impl StatusBarConfig {
    /// Whether `item` is in `visible_items`.
    pub fn is_visible(&self, item: StatusBarItem) -> bool {
        self.visible_items.contains(&item)
    }

    /// Flip `item`'s membership in `visible_items`. Pure mutation so
    /// the toggle logic is unit-testable independent of the I/O-touching
    /// `SettingsStore::patch_user` call chain that persists it
    /// (`Workspace::toggle_status_bar_item`).
    pub fn toggle(&mut self, item: StatusBarItem) {
        if self.is_visible(item) {
            self.visible_items.retain(|&i| i != item);
        } else {
            self.visible_items.push(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shows_every_item() {
        let cfg = StatusBarConfig::default();
        for item in StatusBarItem::ALL {
            assert!(cfg.is_visible(*item));
        }
    }

    #[test]
    fn deserializes_empty_toml_to_defaults() {
        let cfg: StatusBarConfig = toml::from_str("").unwrap();
        assert_eq!(cfg, StatusBarConfig::default());
    }

    #[test]
    fn deserializes_explicit_subset() {
        let toml = r#"visible_items = ["project_branch", "ports"]"#;
        let cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        assert!(cfg.is_visible(StatusBarItem::ProjectBranch));
        assert!(cfg.is_visible(StatusBarItem::Ports));
        assert!(!cfg.is_visible(StatusBarItem::AccountSlot));
    }

    #[test]
    fn a_config_predating_an_item_leaves_it_hidden() {
        // The stored list is an explicit choice: a config written before
        // `claude_usage` existed keeps the segment off rather than having
        // it reappear on upgrade.
        let toml = r#"visible_items = ["project_branch", "account_slot", "ports"]"#;
        let cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.is_visible(StatusBarItem::ClaudeUsage));
    }

    #[test]
    fn toggle_hides_a_visible_item() {
        let mut cfg = StatusBarConfig::default();
        cfg.toggle(StatusBarItem::Ports);
        assert!(!cfg.is_visible(StatusBarItem::Ports));
    }

    #[test]
    fn toggle_shows_a_hidden_item() {
        let mut cfg = StatusBarConfig {
            visible_items: vec![StatusBarItem::ProjectBranch],
        };
        cfg.toggle(StatusBarItem::Ports);
        assert!(cfg.is_visible(StatusBarItem::Ports));
        assert!(cfg.is_visible(StatusBarItem::ProjectBranch));
    }

    #[test]
    fn toggle_is_its_own_inverse() {
        // `visible_items` is consulted only through `is_visible` (a set
        // membership test) — vector order is incidental, so toggling
        // twice is checked by membership, not exact struct equality.
        let mut cfg = StatusBarConfig::default();
        cfg.toggle(StatusBarItem::AccountSlot);
        cfg.toggle(StatusBarItem::AccountSlot);
        for item in StatusBarItem::ALL {
            assert_eq!(
                cfg.is_visible(*item),
                StatusBarConfig::default().is_visible(*item)
            );
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let original = StatusBarConfig {
            visible_items: vec![StatusBarItem::Ports],
        };
        let serialized = toml::to_string(&original).unwrap();
        let back: StatusBarConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(back, original);
    }
}
