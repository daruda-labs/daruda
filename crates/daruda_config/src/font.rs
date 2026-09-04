use serde::{Deserialize, Deserializer, Serialize};

pub const SYSTEM_UI_FONT_FAMILY: &str = ".SystemUIFont";

/// Font settings grouped by the surface that consumes them.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FontConfig {
    pub terminal: TerminalFontConfig,
    pub editor: EditorFontConfig,
    pub agent_chat: AgentChatFontConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TerminalFontConfig {
    /// Primary font family (e.g. "Monaco", "JetBrains Mono").
    pub family: String,
    /// Font size in points. Clamped to 6.0-72.0 at load time.
    pub size: f32,
    /// Line-height multiplier. 1.0 uses the font's natural metrics.
    pub line_height: f32,
    /// Cell-width multiplier. 1.0 uses the font's natural advance width.
    pub cell_width: f32,
    /// Horizontal inset inside the terminal pane, in pixels.
    pub inset_x: f32,
    /// Vertical inset inside the terminal pane, in pixels.
    pub inset_y: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct EditorFontConfig {
    /// Font used by raw files, diffs, and Markdown previews.
    pub family: String,
    /// Font size in points. Clamped to 6.0-72.0 at load time.
    pub size: f32,
    /// Line-height multiplier.
    pub line_height: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AgentChatFontConfig {
    /// Font used by conversation prose. Code keeps a monospace face.
    pub family: String,
    /// Font size in points. Clamped to 6.0-72.0 at load time.
    pub size: f32,
    /// Line-height multiplier.
    pub line_height: f32,
}

impl Default for TerminalFontConfig {
    fn default() -> Self {
        Self {
            family: default_monospace_font_family().to_string(),
            size: 13.0,
            line_height: 1.0,
            cell_width: 1.0,
            inset_x: 4.0,
            inset_y: 2.0,
        }
    }
}

impl Default for EditorFontConfig {
    fn default() -> Self {
        Self {
            family: default_monospace_font_family().to_string(),
            size: 13.0,
            line_height: 1.7,
        }
    }
}

impl Default for AgentChatFontConfig {
    fn default() -> Self {
        Self {
            family: SYSTEM_UI_FONT_FAMILY.to_string(),
            size: 13.0,
            line_height: 1.6,
        }
    }
}

impl FontConfig {
    /// Clamp all numeric fields to their valid ranges.
    pub fn clamp(&mut self) {
        self.terminal.size = self.terminal.size.clamp(6.0, 72.0);
        self.terminal.line_height = self.terminal.line_height.clamp(0.5, 2.0);
        self.terminal.cell_width = self.terminal.cell_width.clamp(0.5, 2.0);
        self.terminal.inset_x = self.terminal.inset_x.clamp(0.0, 32.0);
        self.terminal.inset_y = self.terminal.inset_y.clamp(0.0, 32.0);
        self.editor.size = self.editor.size.clamp(6.0, 72.0);
        self.editor.line_height = self.editor.line_height.clamp(0.5, 2.0);
        self.agent_chat.size = self.agent_chat.size.clamp(6.0, 72.0);
        self.agent_chat.line_height = self.agent_chat.line_height.clamp(0.5, 2.0);
    }
}

fn default_monospace_font_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "Monaco"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "DejaVu Sans Mono"
    }
}

// Option-backed input types retain whether each nested key was present. This
// lets partially migrated files combine new tables with legacy flat keys.
#[derive(Default, Deserialize)]
struct FontConfigInput {
    terminal: Option<TerminalFontConfigInput>,
    editor: Option<TextFontConfigInput>,
    agent_chat: Option<TextFontConfigInput>,
    family: Option<String>,
    size: Option<f32>,
    editor_size: Option<f32>,
    agent_chat_size: Option<f32>,
    vertical_spacing: Option<f32>,
    horizontal_spacing: Option<f32>,
    inset_x: Option<f32>,
    inset_y: Option<f32>,
}

#[derive(Default, Deserialize)]
struct TerminalFontConfigInput {
    family: Option<String>,
    size: Option<f32>,
    line_height: Option<f32>,
    cell_width: Option<f32>,
    inset_x: Option<f32>,
    inset_y: Option<f32>,
}

#[derive(Default, Deserialize)]
struct TextFontConfigInput {
    family: Option<String>,
    size: Option<f32>,
    line_height: Option<f32>,
}

impl<'de> Deserialize<'de> for FontConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let input = FontConfigInput::deserialize(deserializer)?;
        let defaults = Self::default();
        let terminal = input.terminal.unwrap_or_default();
        let editor = input.editor.unwrap_or_default();
        let agent_chat = input.agent_chat.unwrap_or_default();

        Ok(Self {
            terminal: TerminalFontConfig {
                family: terminal
                    .family
                    .or_else(|| input.family.clone())
                    .unwrap_or(defaults.terminal.family),
                size: terminal
                    .size
                    .or(input.size)
                    .unwrap_or(defaults.terminal.size),
                line_height: terminal
                    .line_height
                    .or(input.vertical_spacing)
                    .unwrap_or(defaults.terminal.line_height),
                cell_width: terminal
                    .cell_width
                    .or(input.horizontal_spacing)
                    .unwrap_or(defaults.terminal.cell_width),
                inset_x: terminal
                    .inset_x
                    .or(input.inset_x)
                    .unwrap_or(defaults.terminal.inset_x),
                inset_y: terminal
                    .inset_y
                    .or(input.inset_y)
                    .unwrap_or(defaults.terminal.inset_y),
            },
            editor: EditorFontConfig {
                family: editor
                    .family
                    .or(input.family)
                    .unwrap_or(defaults.editor.family),
                size: editor
                    .size
                    .or(input.editor_size)
                    .unwrap_or(defaults.editor.size),
                line_height: editor.line_height.unwrap_or(defaults.editor.line_height),
            },
            agent_chat: AgentChatFontConfig {
                family: agent_chat.family.unwrap_or(defaults.agent_chat.family),
                size: agent_chat
                    .size
                    .or(input.agent_chat_size)
                    .unwrap_or(defaults.agent_chat.size),
                line_height: agent_chat
                    .line_height
                    .unwrap_or(defaults.agent_chat.line_height),
            },
        })
    }
}
