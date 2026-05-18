use serde::{Deserialize, Serialize};

pub const DEFAULT_SYNTAX_THEME: &str = "base16-ocean.dark";

/// File viewer display settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FileViewerConfig {
    /// Syntect theme name for syntax highlighting in raw and diff views.
    /// Built-in choices: "base16-ocean.dark", "base16-ocean.light",
    /// "base16-eighties.dark", "base16-mocha.dark", "InspiredGitHub",
    /// "Solarized (dark)", "Solarized (light)".
    /// Unknown names fall back to "base16-ocean.dark".
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
