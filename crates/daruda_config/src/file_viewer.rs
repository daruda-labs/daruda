use serde::{Deserialize, Serialize};

pub const DEFAULT_SYNTAX_THEME: &str = "daruda";

/// File viewer display settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FileViewerConfig {
    /// Selected syntax palette for raw and diff highlighting. Curated
    /// choices: "daruda" (recommended default), "one-dark", "tokyo-night",
    /// "catppuccin-mocha". Unknown / legacy names fall back to "daruda".
    pub syntax_theme: String,
    /// When true (default), clicking a file in the left dock reuses the
    /// existing file-viewer tab instead of opening a new one. The tab
    /// content is replaced in place; Cmd+W still closes it.
    /// Set to false to open a separate tab for every file.
    pub preview_tab: bool,
}

impl Default for FileViewerConfig {
    fn default() -> Self {
        Self {
            syntax_theme: DEFAULT_SYNTAX_THEME.to_owned(),
            preview_tab: true,
        }
    }
}
