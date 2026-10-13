//! Public API tests for logs.

use daruda_config::logs::*;

#[test]
fn defaults_match_30d_10mb() {
    let cfg = LogsConfig::default();
    assert_eq!(cfg.retention_days, 30);
    assert_eq!(cfg.max_file_size_mb, 10);
    assert_eq!(
        cfg.retention_duration().unwrap().as_secs(),
        30 * 24 * 60 * 60
    );
    assert_eq!(cfg.max_file_size_bytes(), Some(10 * 1024 * 1024));
}

#[test]
fn zero_disables_each_knob_independently() {
    let cfg = LogsConfig {
        retention_days: 0,
        max_file_size_mb: 5,
    };
    assert!(cfg.retention_duration().is_none());
    assert_eq!(cfg.max_file_size_bytes(), Some(5 * 1024 * 1024));

    let cfg = LogsConfig {
        retention_days: 7,
        max_file_size_mb: 0,
    };
    assert_eq!(
        cfg.retention_duration().unwrap().as_secs(),
        7 * 24 * 60 * 60
    );
    assert!(cfg.max_file_size_bytes().is_none());
}

#[test]
fn parses_partial_section() {
    // Only retention_days set — max_file_size_mb falls back to default.
    let toml = "retention_days = 7";
    let parsed: LogsConfig = toml::from_str(toml).expect("parse");
    assert_eq!(parsed.retention_days, 7);
    assert_eq!(parsed.max_file_size_mb, DEFAULT_MAX_FILE_SIZE_MB);
}
