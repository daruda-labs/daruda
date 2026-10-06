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
    /// Cursor shape while the program has not chosen one. A DECSCUSR
    /// request from the program wins.
    pub style: CursorStyle,
    /// Not implemented: the terminal renderer never blinks the cursor, so
    /// this is read and ignored. Kept so existing files still parse.
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
