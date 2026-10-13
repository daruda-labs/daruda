//! Public API tests for general.

use daruda_config::general::*;

#[test]
fn default_language_is_auto() {
    assert_eq!(GeneralConfig::default().language, "auto");
}

#[test]
fn supported_locales_contains_auto() {
    assert!(SUPPORTED_LOCALES.contains(&"auto"));
}

#[test]
fn deserialize_explicit_language() {
    let toml = "[general]\nlanguage = \"ko\"\n";
    let cfg: toml::Value = toml::from_str(toml).unwrap();
    let general: GeneralConfig = cfg["general"].clone().try_into().unwrap();
    assert_eq!(general.language, "ko");
}

#[test]
fn deserialize_missing_section_uses_default() {
    let toml = "[font]\nsize = 14.0\n";
    let cfg: toml::Value = toml::from_str(toml).unwrap();
    let general: GeneralConfig = cfg
        .get("general")
        .cloned()
        .map(|v| v.try_into().unwrap())
        .unwrap_or_default();
    assert_eq!(general.language, "auto");
}

#[test]
fn serialize_round_trips() {
    let original = GeneralConfig {
        language: "en".to_owned(),
    };
    let serialized = toml::to_string(&original).unwrap();
    let deserialized: GeneralConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(deserialized.language, original.language);
}
