//! Public API tests for usage.

use daruda_config::{PollConfig, UsageConfig};
use std::time::Duration;

#[test]
fn poll_defaults_are_five_minutes() {
    let p = PollConfig::default();
    assert_eq!(p.limits_secs, 300);
    assert_eq!(p.status_secs, 300);
}

#[test]
fn limits_interval_returns_none_when_zero() {
    let p = PollConfig {
        limits_secs: 0,
        status_secs: 0,
    };
    assert!(p.limits_interval().is_none());
    assert!(p.status_interval().is_none());
}

#[test]
fn positive_below_min_is_clamped_to_floor() {
    let p = PollConfig {
        limits_secs: 1,
        status_secs: 30,
    };
    assert_eq!(
        p.limits_interval(),
        Some(Duration::from_secs(PollConfig::MIN_POLL_SECS))
    );
    assert_eq!(
        p.status_interval(),
        Some(Duration::from_secs(PollConfig::MIN_POLL_SECS))
    );
}

#[test]
fn positive_at_or_above_min_passes_through() {
    let p = PollConfig {
        limits_secs: 60,
        status_secs: 600,
    };
    assert_eq!(p.limits_interval(), Some(Duration::from_secs(60)));
    assert_eq!(p.status_interval(), Some(Duration::from_secs(600)));
}

#[test]
fn deserializes_empty_toml_to_defaults() {
    let c: UsageConfig = toml::from_str("").unwrap();
    assert_eq!(c, UsageConfig::default());
}

#[test]
fn round_trips_through_toml() {
    let original = UsageConfig {
        poll: PollConfig {
            limits_secs: 600,
            status_secs: 0,
        },
    };
    let serialized = toml::to_string(&original).unwrap();
    let back: UsageConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(back, original);
}
