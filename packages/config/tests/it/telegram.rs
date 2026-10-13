//! Public API tests for telegram.

use daruda_config::telegram::*;

#[test]
fn defaults_are_inert() {
    let cfg = TelegramConfig::default();
    assert!(!cfg.enabled);
    assert_eq!(cfg.authorized_chat_id, None);
}

#[test]
fn toml_round_trip_preserves_explicit_false() {
    let toml_src = "\
enabled = false
authorized_chat_id = 123456789
";
    let cfg: TelegramConfig = toml::from_str(toml_src).unwrap();
    assert!(!cfg.enabled);
    assert_eq!(cfg.authorized_chat_id, Some(123_456_789));

    let serialized = toml::to_string(&cfg).unwrap();
    let reparsed: TelegramConfig = toml::from_str(&serialized).unwrap();
    assert!(!reparsed.enabled);
    assert_eq!(reparsed.authorized_chat_id, Some(123_456_789));
}

#[test]
fn defaults_restrict_the_phone_to_absence() {
    assert!(TelegramConfig::default().only_when_away);
}

#[test]
fn toml_round_trip_unspecified_fields_use_defaults() {
    let toml_src = "enabled = true\n";
    let cfg: TelegramConfig = toml::from_str(toml_src).unwrap();
    assert!(cfg.enabled);
    // Unspecified fields fall back to their defaults via `#[serde(default)]`.
    assert_eq!(cfg.authorized_chat_id, None);
    assert!(cfg.only_when_away);
}
