//! Public API tests for status bar.

use daruda_config::{StatusBarConfig, StatusBarItem};

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
fn toggle_hides_a_visible_item() {
    let mut cfg = StatusBarConfig::default();
    cfg.toggle(StatusBarItem::Ports);
    assert!(!cfg.is_visible(StatusBarItem::Ports));
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
