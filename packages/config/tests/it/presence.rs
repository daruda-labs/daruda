//! Public API tests for presence.

use daruda_config::presence::*;

#[test]
fn defaults_put_the_foreground_bar_above_the_blurred_one() {
    let cfg = PresenceConfig::default();
    assert_eq!(cfg.away_grace_secs, 10);
    assert_eq!(cfg.away_idle_secs, 30);
    assert_eq!(cfg.away_idle_foreground_secs, 180);
    assert!(cfg.away_idle_foreground_secs > cfg.away_idle_secs);
}

#[test]
fn clamp_refuses_a_foreground_bar_below_the_blurred_one() {
    let mut cfg = PresenceConfig {
        away_grace_secs: 15,
        away_idle_secs: 120,
        away_idle_foreground_secs: 30,
    };
    cfg.clamp();
    assert_eq!(cfg.away_idle_foreground_secs, 120);

    // A foreground bar already above the blurred one is left alone.
    let mut ordered = PresenceConfig::default();
    ordered.clamp();
    assert_eq!(ordered.away_idle_foreground_secs, 180);
}

#[test]
fn toml_round_trip_preserves_explicit_zero() {
    let cfg: PresenceConfig = toml::from_str("away_idle_secs = 0\n").unwrap();
    assert_eq!(cfg.away_idle_secs, 0);
    // Unspecified fields fall back to their defaults via `#[serde(default)]`.
    assert_eq!(cfg.away_grace_secs, 10);
    assert_eq!(cfg.away_idle_foreground_secs, 180);

    let reparsed: PresenceConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
    assert_eq!(reparsed.away_idle_secs, 0);
    assert_eq!(reparsed.away_grace_secs, 10);
    assert_eq!(reparsed.away_idle_foreground_secs, 180);
}
