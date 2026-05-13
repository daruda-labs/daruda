use serde::{Deserialize, Serialize};

/// Cursor shape. Maps to DECSCUSR codes at render time.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Cursor appearance configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct CursorConfig {
    /// Cursor shape. Programs can override this via DECSCUSR.
    pub style: CursorStyle,
    /// Whether the cursor blinks when idle.
    pub blinking: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: CursorStyle::default(),
            blinking: true,
        }
    }
}
