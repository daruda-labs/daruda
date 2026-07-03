use serde::{Deserialize, Serialize};

/// Font configuration.
///
/// `family` is the primary font family name. Platform-specific
/// fallbacks (CJK, emoji) are appended by the renderer — this field
/// only controls the first choice.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct FontConfig {
    /// Primary font family (e.g. "Monaco", "JetBrains Mono").
    pub family: String,
    /// Terminal font size in points. Clamped to 6.0–72.0 at load time.
    pub size: f32,
    /// File-viewer / editor font size in points (raw code, diff, markdown
    /// raw rows, line-number gutter). Independent of the terminal `size`.
    /// Clamped to 6.0–72.0 at load time.
    pub editor_size: f32,
    /// Agent-chat font size in points — the whole conversation pane (message
    /// bodies, headers, tool titles, and chrome share one size). Independent
    /// of the terminal `size` and the `editor_size`. Clamped to 6.0–72.0 at
    /// load time.
    pub agent_chat_size: f32,
    /// Line height multiplier. 1.0 = natural line height.
    pub vertical_spacing: f32,
    /// Cell width multiplier. 1.0 = natural advance width.
    pub horizontal_spacing: f32,
    /// Horizontal inset (left/right padding) inside the terminal pane,
    /// in pixels. iTerm2 `TerminalMargin`. Clamped to 0.0–32.0.
    pub inset_x: f32,
    /// Vertical inset (top/bottom padding) inside the terminal pane,
    /// in pixels. iTerm2 `TerminalVMargin`. Clamped to 0.0–32.0.
    pub inset_y: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: default_font_family().to_string(),
            size: 13.0,
            editor_size: 13.0,
            agent_chat_size: 13.0,
            vertical_spacing: 1.0,
            horizontal_spacing: 1.0,
            inset_x: 4.0,
            inset_y: 2.0,
        }
    }
}

impl FontConfig {
    /// Clamp all numeric fields to their valid ranges.
    pub fn clamp(&mut self) {
        self.size = self.size.clamp(6.0, 72.0);
        self.editor_size = self.editor_size.clamp(6.0, 72.0);
        self.agent_chat_size = self.agent_chat_size.clamp(6.0, 72.0);
        self.vertical_spacing = self.vertical_spacing.clamp(0.5, 2.0);
        self.horizontal_spacing = self.horizontal_spacing.clamp(0.5, 2.0);
        self.inset_x = self.inset_x.clamp(0.0, 32.0);
        self.inset_y = self.inset_y.clamp(0.0, 32.0);
    }
}

fn default_font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Monaco"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}
