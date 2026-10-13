//! Public API tests for ports.

use daruda_config::PortsConfig;
use std::time::Duration;

#[test]
fn interval_clamps_to_minimum() {
    let cfg = PortsConfig { poll_secs: 1 };
    assert_eq!(
        cfg.interval(),
        Duration::from_secs(PortsConfig::MIN_POLL_SECS)
    );
}

#[test]
fn interval_passes_through_above_minimum() {
    let cfg = PortsConfig { poll_secs: 10 };
    assert_eq!(cfg.interval(), Duration::from_secs(10));
}

#[test]
fn deserializes_empty_toml_to_defaults() {
    let cfg: PortsConfig = toml::from_str("").unwrap();
    assert_eq!(cfg, PortsConfig::default());
}
