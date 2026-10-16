#[test]
fn values_round_trip_and_reject_unknown_sizes() {
    for size in super::UiSize::ALL {
        assert_eq!(super::UiSize::from_value(size.as_str()), Some(size));
    }
    assert_eq!(super::UiSize::from_value("huge"), None);
}
#[test]
fn patch_and_reset_preserve_other_fields_and_old_configs_use_normal() {
    use crate::{Config, SettingsPatch, UiSize};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# Keep this comment\n[left_dock]\nfiles_show_hidden = false\n",
    )
    .unwrap();
    let old: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(old.left_dock.size, UiSize::Normal);
    for size in UiSize::ALL {
        let saved =
            crate::apply_settings_patch_to(&SettingsPatch::LeftDockSize(size), &path).unwrap();
        assert_eq!(saved.left_dock.size, size);
        assert!(!saved.left_dock.files_show_hidden);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# Keep this comment")
        );
    }
    let expected =
        crate::apply_settings_patch_to(&SettingsPatch::LeftDockSize(UiSize::Large), &path).unwrap();
    let reset = crate::reset_settings_field_to_if_unchanged(
        &SettingsPatch::LeftDockSize(UiSize::Normal),
        &expected,
        &path,
    )
    .unwrap();
    assert_eq!(reset.left_dock.size, UiSize::Normal);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("size ="));
}
