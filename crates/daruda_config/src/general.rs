//! General application settings — language selection.

use serde::{Deserialize, Serialize};

/// Supported UI language codes. "auto" follows the system locale.
pub const SUPPORTED_LOCALES: &[&str] = &["auto", "en", "ko"];

/// General application settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// UI display language. `"auto"` follows the system locale; other values
    /// pin to a specific language (e.g. `"en"`, `"ko"`). Unknown values fall
    /// back to `"auto"` at apply time.
    pub language: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            language: "auto".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let general: GeneralConfig =
            cfg["general"].clone().try_into().unwrap();
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
}
