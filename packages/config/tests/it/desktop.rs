//! Public API tests for desktop.

use daruda_config::KeepAwakeMode;
use daruda_config::desktop::DesktopConfig;

fn desktop(text: &str) -> DesktopConfig {
    let config: daruda_config::Config = toml::from_str(text).unwrap();
    config.desktop
}

#[test]
fn existing_configs_keep_previous_desktop_behavior() {
    let config: DesktopConfig = toml::from_str("").unwrap();
    assert!(!config.tray_enabled && !config.close_to_tray);
    assert_eq!(config.keep_awake, KeepAwakeMode::Off);
    assert_eq!(config.keep_awake_approval_wait_minutes, 30);
}

#[test]
fn defaults_match_a_missing_or_empty_table() {
    let default = DesktopConfig::default();
    assert_eq!(default.keep_awake, KeepAwakeMode::Off);
    assert_eq!(default.keep_awake_approval_wait_minutes, 30);
    for text in ["", "[desktop]\n"] {
        let parsed = desktop(text);
        assert_eq!(parsed.keep_awake, KeepAwakeMode::Off, "{text:?}");
        assert_eq!(parsed.keep_awake_approval_wait_minutes, 30, "{text:?}");
    }
}

#[test]
fn keep_awake_modes_parse() {
    assert_eq!(
        desktop("[desktop]\nkeep_awake = \"agent\"\n").keep_awake,
        KeepAwakeMode::Agent
    );
    assert_eq!(
        desktop("[desktop]\nkeep_awake = \"on\"\n").keep_awake,
        KeepAwakeMode::On
    );
    assert_eq!(
        desktop("[desktop]\nkeep_awake = \"off\"\n").keep_awake,
        KeepAwakeMode::Off
    );
}

#[test]
fn the_legacy_flag_folds_into_the_mode() {
    assert_eq!(
        desktop("[desktop]\nkeep_awake_while_working = true\n").keep_awake,
        KeepAwakeMode::Agent
    );
    assert_eq!(
        desktop("[desktop]\nkeep_awake_while_working = false\n").keep_awake,
        KeepAwakeMode::Off
    );
}

#[test]
fn an_explicit_mode_wins_over_the_legacy_flag() {
    let text = "[desktop]\nkeep_awake_while_working = true\nkeep_awake = \"off\"\n";
    assert_eq!(desktop(text).keep_awake, KeepAwakeMode::Off);
}

#[test]
fn the_legacy_flag_is_never_serialized() {
    let config = desktop("[desktop]\nkeep_awake_while_working = true\n");
    let text = toml::to_string(&config).unwrap();
    assert!(!text.contains("keep_awake_while_working"), "{text}");
    assert!(text.contains("keep_awake = \"agent\""), "{text}");
}

#[test]
fn a_zero_approval_wait_is_clamped_to_one_minute() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[desktop]\nkeep_awake_approval_wait_minutes = 0\n").unwrap();
    let config = daruda_config::Config::load_from(&path);
    assert_eq!(config.desktop.keep_awake_approval_wait_minutes, 1);
}

#[test]
fn a_huge_approval_wait_is_clamped_to_one_day() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = format!(
        "[desktop]\nkeep_awake_approval_wait_minutes = {}\n",
        i64::MAX
    );
    std::fs::write(&path, text).unwrap();
    let config = daruda_config::Config::load_from(&path);
    assert_eq!(config.desktop.keep_awake_approval_wait_minutes, 24 * 60);
}

#[test]
fn mode_spelling_matches_the_serialized_name() {
    for mode in [KeepAwakeMode::Off, KeepAwakeMode::Agent, KeepAwakeMode::On] {
        let config = DesktopConfig {
            keep_awake: mode,
            ..DesktopConfig::default()
        };
        let text = toml::to_string(&config).unwrap();
        assert!(
            text.contains(&format!("keep_awake = \"{}\"", mode.as_str())),
            "{text}"
        );
    }
}
