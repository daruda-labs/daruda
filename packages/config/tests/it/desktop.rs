//! Public API tests for desktop.

#[test]
fn existing_configs_keep_previous_desktop_behavior() {
    let config: daruda_config::desktop::DesktopConfig = toml::from_str("").unwrap();
    assert!(!config.tray_enabled && !config.close_to_tray && !config.keep_awake_while_working);
}
