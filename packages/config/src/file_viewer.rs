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
    /// When true (default), browsing files in the left dock reuses one
    /// scratch tab instead of opening a new one per file: the tab content is
    /// replaced in place, and Cmd+W still closes it. Only that scratch tab is
    /// reused — pressing Enter on a row, or opening a file from a flow, an
    /// agent or a skill, gives it a tab of its own that later browsing leaves
    /// alone. Set to false to open a separate tab for every file.
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
