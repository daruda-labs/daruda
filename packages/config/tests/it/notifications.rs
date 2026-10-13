//! Public API tests for notifications.

use daruda_config::notifications::*;

#[test]
fn agent_channels_default_true() {
    let cfg = NotificationsConfig::default();
    assert!(cfg.agent_completion_enabled);
    assert!(cfg.agent_waiting_enabled);
}

#[test]
fn toml_round_trip_preserves_explicit_false() {
    let toml_src = "\
agent_completion_enabled = false
agent_waiting_enabled = false
";
    let cfg: NotificationsConfig = toml::from_str(toml_src).unwrap();
    assert!(!cfg.agent_completion_enabled);
    assert!(!cfg.agent_waiting_enabled);
    // Unspecified fields fall back to their defaults via `#[serde(default)]`.
    assert!(cfg.hook_notification_enabled);

    let serialized = toml::to_string(&cfg).unwrap();
    let reparsed: NotificationsConfig = toml::from_str(&serialized).unwrap();
    assert!(!reparsed.agent_completion_enabled);
    assert!(!reparsed.agent_waiting_enabled);
}
