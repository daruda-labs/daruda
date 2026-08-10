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
    /// Flow runs this window started — what each is doing, and a Stop.
    Flow,
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
        Self::Flow,
    ];
}

/// Segments this file's owner has turned *off*.
///
/// Recorded as an opt-out, not an opt-in, because a list of what to show
/// is also a list of what existed the day it was written: every segment
/// added afterwards is silently absent, and so hidden from the one person
/// who might have wanted it — with no way to find out it is there. The
/// stored list should hold the user's decisions and nothing else.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct StatusBarConfig {
    /// Empty means everything shows, now and after the next segment lands.
    pub hidden_items: Vec<StatusBarItem>,
    /// The old opt-in list. Read once, folded into `hidden_items` by
    /// [`StatusBarConfig::clamp`], and never written back.
    #[serde(skip_serializing)]
    visible_items: Option<Vec<StatusBarItem>>,
}

/// Segments that did not exist while `visible_items` was the format. A
/// file written back then cannot have opted out of one it never saw, so
/// the migration must not read their absence as a choice.
const ADDED_AFTER_OPT_IN: &[StatusBarItem] = &[StatusBarItem::Flow];

impl StatusBarConfig {
    /// Fold a legacy `visible_items` into `hidden_items`, once.
    pub(crate) fn clamp(&mut self) {
        let Some(visible) = self.visible_items.take() else {
            return;
        };
        self.hidden_items = StatusBarItem::ALL
            .iter()
            .copied()
            .filter(|item| !visible.contains(item) && !ADDED_AFTER_OPT_IN.contains(item))
            .collect();
    }

    /// Whether `item` renders. Anything not turned off does.
    pub fn is_visible(&self, item: StatusBarItem) -> bool {
        !self.hidden_items.contains(&item)
    }

    /// Flip `item`'s membership in `visible_items`. Pure mutation so
    /// the toggle logic is unit-testable independent of the I/O-touching
    /// `SettingsStore::patch_user` call chain that persists it
    /// (`Workspace::toggle_status_bar_item`).
    pub fn toggle(&mut self, item: StatusBarItem) {
        if self.is_visible(item) {
            self.hidden_items.push(item);
        } else {
            self.hidden_items.retain(|&i| i != item);
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
        let mut cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        cfg.clamp();
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
        let mut cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        cfg.clamp();
        assert!(!cfg.is_visible(StatusBarItem::ClaudeUsage));
    }

    /// The reason for the inversion. A file written before `Flow` existed
    /// cannot have opted out of it, so the migration must not read its
    /// absence as a decision — otherwise the segment ships hidden to
    /// everyone who already had a config, which is everyone.
    #[test]
    fn a_legacy_file_does_not_hide_a_segment_it_never_saw() {
        let toml = r#"visible_items = ["project_branch", "account_slot", "ports"]"#;
        let mut cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        cfg.clamp();
        assert!(cfg.is_visible(StatusBarItem::Flow));
        // …while a choice it *could* have made is still honoured.
        assert!(!cfg.is_visible(StatusBarItem::ClaudeUsage));
    }

    /// Once migrated the legacy list is gone, so a second pass has nothing
    /// to fold and cannot undo a hide the user has made since.
    #[test]
    fn migrating_twice_does_not_resurrect_the_old_list() {
        let toml = r#"visible_items = ["ports"]"#;
        let mut cfg: StatusBarConfig = toml::from_str(toml).unwrap();
        cfg.clamp();
        cfg.toggle(StatusBarItem::Ports);
        cfg.clamp();
        assert!(!cfg.is_visible(StatusBarItem::Ports));
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
            hidden_items: vec![StatusBarItem::Ports],
            ..Default::default()
        };
        cfg.toggle(StatusBarItem::Ports);
        assert!(cfg.is_visible(StatusBarItem::Ports));
        assert!(cfg.is_visible(StatusBarItem::ProjectBranch));
    }

    #[test]
    fn toggle_is_its_own_inverse() {
        // `hidden_items` is consulted only through `is_visible` (a set
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
            hidden_items: vec![StatusBarItem::Ports],
            ..Default::default()
        };
        let serialized = toml::to_string(&original).unwrap();
        let back: StatusBarConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(back, original);
    }
}
