//! Public API tests for claude status.

use daruda_config::ClaudeStatusConfig;

#[test]
fn defaults_match_spec() {
    let c = ClaudeStatusConfig::default();
    assert!(c.enable);
    assert_eq!(c.stale_threshold_secs, 300);
    assert_eq!(c.file_ttl_days, 7);
}

#[test]
fn deserializes_partial_toml() {
    // Empty section → defaults, missing fields → defaults.
    let c: ClaudeStatusConfig = toml::from_str("").unwrap();
    assert_eq!(c, ClaudeStatusConfig::default());

    let c: ClaudeStatusConfig = toml::from_str("enable = false").unwrap();
    assert!(!c.enable);
    assert_eq!(c.stale_threshold_secs, 300);
}
