//! Public API tests for settings section.

use daruda_config::{BuiltinSection, SettingsSection};

#[test]
fn default_section_is_general() {
    assert_eq!(
        SettingsSection::default(),
        SettingsSection::Builtin(BuiltinSection::General)
    );
}

#[test]
fn all_slugs_are_unique() {
    let mut slugs: Vec<_> = BuiltinSection::ALL.iter().map(|b| b.slug()).collect();
    slugs.sort();
    let dedup_len = {
        let mut copy = slugs.clone();
        copy.dedup();
        copy.len()
    };
    assert_eq!(slugs.len(), dedup_len, "slugs must be unique: {slugs:?}");
}

#[test]
fn slug_round_trip() {
    for &b in BuiltinSection::ALL {
        assert_eq!(BuiltinSection::from_slug(b.slug()), Some(b));
    }
}

#[test]
fn from_slug_unknown_returns_none() {
    assert_eq!(BuiltinSection::from_slug("nonexistent"), None);
    assert_eq!(BuiltinSection::from_slug(""), None);
}

/// Regression: an `open_settings.<slug>` keybinding written against any
/// earlier page layout must keep opening the page that now holds it.
#[test]
fn every_retired_slug_opens_the_page_that_absorbed_it() {
    let expected = [
        ("window", BuiltinSection::Appearance),
        ("shell", BuiltinSection::Terminal),
        ("cursor", BuiltinSection::Terminal),
        ("clipboard", BuiltinSection::Terminal),
        ("dock", BuiltinSection::Workspace),
        ("sidebar", BuiltinSection::Workspace),
        ("file_viewer", BuiltinSection::Workspace),
        ("left_dock", BuiltinSection::Workspace),
        ("panels", BuiltinSection::Workspace),
        ("external_editor", BuiltinSection::Workspace),
        ("claude_status", BuiltinSection::Agent),
    ];
    for (slug, section) in expected {
        assert_eq!(BuiltinSection::from_slug(slug), Some(section), "{slug}");
    }
}

#[test]
fn slugs_are_snake_case() {
    for b in BuiltinSection::ALL {
        let s = b.slug();
        assert!(
            s.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
            "slug {s:?} should be snake_case"
        );
    }
}

#[test]
fn all_contains_every_variant() {
    // Spot-check that ALL matches the enum: count must equal the
    // number of variants. If a future variant is added without
    // updating ALL, this test fails.
    //
    // NOTE: bump this number whenever a `BuiltinSection` variant is
    // added or removed. There is no `strum::EnumCount`-style helper
    // in this crate; the count exists precisely to force a manual
    // sync of `ALL` with the enum.
    assert_eq!(BuiltinSection::ALL.len(), 14);
}
