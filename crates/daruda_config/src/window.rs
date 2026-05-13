use serde::{Deserialize, Serialize};

/// Window appearance configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Background opacity. 1.0 = fully opaque. Clamped to 0.1–1.0.
    pub opacity: f32,
    /// Enable background blur behind transparent regions (macOS).
    pub blur: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            blur: false,
        }
    }
}

impl WindowConfig {
    /// Clamp opacity to the valid range.
    pub fn clamp(&mut self) {
        self.opacity = self.opacity.clamp(0.1, 1.0);
    }
}
