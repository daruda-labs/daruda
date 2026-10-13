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
