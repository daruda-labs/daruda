use daruda_config::{Config, SettingsPatch, StatusIconTheme};

#[test]
fn status_icon_theme_round_trips_without_changing_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "# Preserve me\n[left_dock]\nsize = 'large'\n").unwrap();
    assert_eq!(
        Config::load_from(&path).left_dock.status_icon_theme,
        StatusIconTheme::Dots
    );
    for theme in StatusIconTheme::ALL.into_iter().chain([
        StatusIconTheme::Custom("user/example".into()),
        StatusIconTheme::Custom("plugin/example@market/ice".into()),
    ]) {
        assert_eq!(
            StatusIconTheme::from_value(theme.as_str()),
            Some(theme.clone())
        );
        let saved = daruda_config::apply_settings_patch_to(
            &SettingsPatch::StatusIconTheme(theme.clone()),
            &path,
        )
        .unwrap();
        assert_eq!(saved.left_dock.status_icon_theme, theme);
        assert_eq!(Config::load_from(&path).left_dock.status_icon_theme, theme);
        assert_eq!(saved.left_dock.size, daruda_config::UiSize::Large);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# Preserve me")
        );
    }
    assert_eq!(StatusIconTheme::from_value(""), None);
    let expected = Config::load_from(&path);
    let reset = daruda_config::reset_settings_field_to_if_unchanged(
        &SettingsPatch::StatusIconTheme(StatusIconTheme::Dots),
        &expected,
        &path,
    )
    .unwrap();
    assert_eq!(reset.left_dock.status_icon_theme, StatusIconTheme::Dots);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("status_icon_theme")
    );
}

#[test]
fn full_config_write_preserves_the_selected_theme() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut config = Config::default();
    config.left_dock.status_icon_theme = StatusIconTheme::Icons;
    daruda_config::patch_config_file_to(&config, &path).unwrap();
    assert_eq!(
        Config::load_from(&path).left_dock.status_icon_theme,
        StatusIconTheme::Icons
    );
}
