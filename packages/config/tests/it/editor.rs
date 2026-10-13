//! Public API tests for editor.

use daruda_config::editor::*;

#[test]
fn default_preferred_is_empty() {
    assert_eq!(EditorConfig::default().preferred, "");
}

#[test]
fn empty_name_has_no_preset() {
    assert!(preset("").is_none());
}

#[test]
fn unknown_name_has_no_preset() {
    assert!(preset("not-a-real-editor").is_none());
}

#[test]
fn every_preset_name_resolves_to_itself() {
    for p in PRESETS {
        assert_eq!(preset(p.name).map(|found| found.name), Some(p.name));
    }
}

#[test]
fn preset_names_are_unique() {
    let mut names: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
    names.sort_unstable();
    let mut deduped = names.clone();
    deduped.dedup();
    assert_eq!(names.len(), deduped.len(), "preset names must be unique");
}

#[test]
fn every_preset_has_at_least_one_macos_launch_path() {
    for p in PRESETS {
        assert!(
            p.macos_app_name.is_some() || !p.macos_bundle_ids.is_empty(),
            "{} has neither a macOS app name nor bundle ids",
            p.name
        );
    }
}
