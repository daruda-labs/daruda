use crate::{
    AgentEntry, Config, CursorStyle, SessionHostEntry, SessionHostTombstone, StatusBarItem,
};

/// Stable identifier for one Settings UI persistence boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsFieldId {
    GeneralLanguage,
    TerminalPreset,
    UiPreset,
    TerminalFontFamily,
    TerminalFontSize,
    TerminalLineHeight,
    TerminalCellWidth,
    EditorFontFamily,
    EditorFontSize,
    EditorLineHeight,
    AgentChatFontFamily,
    AgentChatFontSize,
    AgentChatLineHeight,
    CursorStyle,
    CursorBlinking,
    AgentUseModifierToSend,
    AgentCatalog,
    SessionHosts,
    RenderMaxFps,
    ShellClosePaneOnExit,
    WindowOpacity,
    WindowBlur,
    ScrollbackMaxRows,
    TerminalInsetX,
    TerminalInsetY,
    FilesShowHidden,
    FilesUseGitignore,
    SyntaxTheme,
    ClipboardStreamingMaxBytes,
    PreferredEditor,
    PanelsGridColumns,
    StatusBarHiddenItems,
    ClaudeStatusEnabled,
    TelegramEnabled,
    TelegramAuthorizedChatId,
}

impl SettingsFieldId {
    pub const fn path(self) -> &'static str {
        match self {
            Self::GeneralLanguage => "general.language",
            Self::TerminalPreset => "theme.terminal_preset",
            Self::UiPreset => "theme.ui_preset",
            Self::TerminalFontFamily => "font.terminal.family",
            Self::TerminalFontSize => "font.terminal.size",
            Self::TerminalLineHeight => "font.terminal.line_height",
            Self::TerminalCellWidth => "font.terminal.cell_width",
            Self::EditorFontFamily => "font.editor.family",
            Self::EditorFontSize => "font.editor.size",
            Self::EditorLineHeight => "font.editor.line_height",
            Self::AgentChatFontFamily => "font.agent_chat.family",
            Self::AgentChatFontSize => "font.agent_chat.size",
            Self::AgentChatLineHeight => "font.agent_chat.line_height",
            Self::CursorStyle => "cursor.style",
            Self::CursorBlinking => "cursor.blinking",
            Self::AgentUseModifierToSend => "agent.use_modifier_to_send",
            Self::AgentCatalog => "agents",
            Self::SessionHosts => "session_hosts",
            Self::RenderMaxFps => "render.max_fps",
            Self::ShellClosePaneOnExit => "shell.close_pane_on_exit",
            Self::WindowOpacity => "window.opacity",
            Self::WindowBlur => "window.blur",
            Self::ScrollbackMaxRows => "scrollback.max_rows",
            Self::TerminalInsetX => "font.terminal.inset_x",
            Self::TerminalInsetY => "font.terminal.inset_y",
            Self::FilesShowHidden => "left_dock.files_show_hidden",
            Self::FilesUseGitignore => "left_dock.files_use_gitignore",
            Self::SyntaxTheme => "file_viewer.syntax_theme",
            Self::ClipboardStreamingMaxBytes => "clipboard.streaming_max_bytes",
            Self::PreferredEditor => "editor.preferred",
            Self::PanelsGridColumns => "panels.grid_columns",
            Self::StatusBarHiddenItems => "status_bar.hidden_items",
            Self::ClaudeStatusEnabled => "claude_status.enable",
            Self::TelegramEnabled => "telegram.enabled",
            Self::TelegramAuthorizedChatId => "telegram.authorized_chat_id",
        }
    }
}

/// One atomic Settings UI change. Structural editors use a single variant so
/// readers never observe a partially-updated catalog.
#[derive(Clone, Debug)]
pub enum SettingsPatch {
    GeneralLanguage(String),
    TerminalPreset(String),
    UiPreset(String),
    TerminalFontFamily(String),
    TerminalFontSize(f32),
    TerminalLineHeight(f32),
    TerminalCellWidth(f32),
    EditorFontFamily(String),
    EditorFontSize(f32),
    EditorLineHeight(f32),
    AgentChatFontFamily(String),
    AgentChatFontSize(f32),
    AgentChatLineHeight(f32),
    CursorStyle(CursorStyle),
    CursorBlinking(bool),
    AgentUseModifierToSend(bool),
    AgentCatalog(Vec<AgentEntry>),
    SessionHosts {
        entries: Vec<SessionHostEntry>,
        tombstones: Vec<SessionHostTombstone>,
    },
    RenderMaxFps(u32),
    ShellClosePaneOnExit(bool),
    WindowOpacity(f32),
    WindowBlur(bool),
    ScrollbackMaxRows(usize),
    TerminalInsetX(f32),
    TerminalInsetY(f32),
    FilesShowHidden(bool),
    FilesUseGitignore(bool),
    SyntaxTheme(String),
    ClipboardStreamingMaxBytes(usize),
    PreferredEditor(String),
    PanelsGridColumns(u8),
    ToggleStatusBarItem(StatusBarItem),
    ClaudeStatusEnabled(bool),
    TelegramEnabled(bool),
    TelegramAuthorizedChatId(Option<i64>),
}

impl SettingsPatch {
    pub const fn field(&self) -> SettingsFieldId {
        match self {
            Self::GeneralLanguage(_) => SettingsFieldId::GeneralLanguage,
            Self::TerminalPreset(_) => SettingsFieldId::TerminalPreset,
            Self::UiPreset(_) => SettingsFieldId::UiPreset,
            Self::TerminalFontFamily(_) => SettingsFieldId::TerminalFontFamily,
            Self::TerminalFontSize(_) => SettingsFieldId::TerminalFontSize,
            Self::TerminalLineHeight(_) => SettingsFieldId::TerminalLineHeight,
            Self::TerminalCellWidth(_) => SettingsFieldId::TerminalCellWidth,
            Self::EditorFontFamily(_) => SettingsFieldId::EditorFontFamily,
            Self::EditorFontSize(_) => SettingsFieldId::EditorFontSize,
            Self::EditorLineHeight(_) => SettingsFieldId::EditorLineHeight,
            Self::AgentChatFontFamily(_) => SettingsFieldId::AgentChatFontFamily,
            Self::AgentChatFontSize(_) => SettingsFieldId::AgentChatFontSize,
            Self::AgentChatLineHeight(_) => SettingsFieldId::AgentChatLineHeight,
            Self::CursorStyle(_) => SettingsFieldId::CursorStyle,
            Self::CursorBlinking(_) => SettingsFieldId::CursorBlinking,
            Self::AgentUseModifierToSend(_) => SettingsFieldId::AgentUseModifierToSend,
            Self::AgentCatalog(_) => SettingsFieldId::AgentCatalog,
            Self::SessionHosts { .. } => SettingsFieldId::SessionHosts,
            Self::RenderMaxFps(_) => SettingsFieldId::RenderMaxFps,
            Self::ShellClosePaneOnExit(_) => SettingsFieldId::ShellClosePaneOnExit,
            Self::WindowOpacity(_) => SettingsFieldId::WindowOpacity,
            Self::WindowBlur(_) => SettingsFieldId::WindowBlur,
            Self::ScrollbackMaxRows(_) => SettingsFieldId::ScrollbackMaxRows,
            Self::TerminalInsetX(_) => SettingsFieldId::TerminalInsetX,
            Self::TerminalInsetY(_) => SettingsFieldId::TerminalInsetY,
            Self::FilesShowHidden(_) => SettingsFieldId::FilesShowHidden,
            Self::FilesUseGitignore(_) => SettingsFieldId::FilesUseGitignore,
            Self::SyntaxTheme(_) => SettingsFieldId::SyntaxTheme,
            Self::ClipboardStreamingMaxBytes(_) => SettingsFieldId::ClipboardStreamingMaxBytes,
            Self::PreferredEditor(_) => SettingsFieldId::PreferredEditor,
            Self::PanelsGridColumns(_) => SettingsFieldId::PanelsGridColumns,
            Self::ToggleStatusBarItem(_) => SettingsFieldId::StatusBarHiddenItems,
            Self::ClaudeStatusEnabled(_) => SettingsFieldId::ClaudeStatusEnabled,
            Self::TelegramEnabled(_) => SettingsFieldId::TelegramEnabled,
            Self::TelegramAuthorizedChatId(_) => SettingsFieldId::TelegramAuthorizedChatId,
        }
    }

    /// Apply this one field to an in-memory snapshot. Persistence callers use
    /// this to advance only the addressed conflict baseline.
    pub fn apply_to(&self, config: &mut Config) {
        match self {
            Self::GeneralLanguage(value) => config.general.language = value.clone(),
            Self::TerminalPreset(value) => config.theme.terminal_preset = value.clone(),
            Self::UiPreset(value) => config.theme.ui_preset = value.clone(),
            Self::TerminalFontFamily(value) => config.font.terminal.family = value.clone(),
            Self::TerminalFontSize(value) => config.font.terminal.size = *value,
            Self::TerminalLineHeight(value) => config.font.terminal.line_height = *value,
            Self::TerminalCellWidth(value) => config.font.terminal.cell_width = *value,
            Self::EditorFontFamily(value) => config.font.editor.family = value.clone(),
            Self::EditorFontSize(value) => config.font.editor.size = *value,
            Self::EditorLineHeight(value) => config.font.editor.line_height = *value,
            Self::AgentChatFontFamily(value) => config.font.agent_chat.family = value.clone(),
            Self::AgentChatFontSize(value) => config.font.agent_chat.size = *value,
            Self::AgentChatLineHeight(value) => config.font.agent_chat.line_height = *value,
            Self::CursorStyle(value) => config.cursor.style = *value,
            Self::CursorBlinking(value) => config.cursor.blinking = *value,
            Self::AgentUseModifierToSend(value) => config.agent.use_modifier_to_send = *value,
            Self::AgentCatalog(value) => config.agents = value.clone(),
            Self::SessionHosts {
                entries,
                tombstones,
            } => {
                config.session_hosts = entries.clone();
                config.session_host_tombstones = tombstones.clone();
            }
            Self::RenderMaxFps(value) => config.render.max_fps = *value,
            Self::ShellClosePaneOnExit(value) => config.shell.close_pane_on_exit = *value,
            Self::WindowOpacity(value) => config.window.opacity = *value,
            Self::WindowBlur(value) => config.window.blur = *value,
            Self::ScrollbackMaxRows(value) => config.scrollback.max_rows = *value,
            Self::TerminalInsetX(value) => config.font.terminal.inset_x = *value,
            Self::TerminalInsetY(value) => config.font.terminal.inset_y = *value,
            Self::FilesShowHidden(value) => config.left_dock.files_show_hidden = *value,
            Self::FilesUseGitignore(value) => config.left_dock.files_use_gitignore = *value,
            Self::SyntaxTheme(value) => config.file_viewer.syntax_theme = value.clone(),
            Self::ClipboardStreamingMaxBytes(value) => {
                config.clipboard.streaming_max_bytes = *value;
            }
            Self::PreferredEditor(value) => config.editor.preferred = value.clone(),
            Self::PanelsGridColumns(value) => config.panels.grid_columns = *value,
            Self::ToggleStatusBarItem(value) => config.status_bar.toggle(*value),
            Self::ClaudeStatusEnabled(value) => config.claude_status.enable = *value,
            Self::TelegramEnabled(value) => config.telegram.enabled = *value,
            Self::TelegramAuthorizedChatId(value) => config.telegram.authorized_chat_id = *value,
        }
    }

    /// Whether the addressed value differs between two config snapshots.
    pub fn field_changed_between(&self, left: &Config, right: &Config) -> bool {
        match self {
            Self::GeneralLanguage(_) => left.general.language != right.general.language,
            Self::TerminalPreset(_) => left.theme.terminal_preset != right.theme.terminal_preset,
            Self::UiPreset(_) => left.theme.ui_preset != right.theme.ui_preset,
            Self::TerminalFontFamily(_) => left.font.terminal.family != right.font.terminal.family,
            Self::TerminalFontSize(_) => left.font.terminal.size != right.font.terminal.size,
            Self::TerminalLineHeight(_) => {
                left.font.terminal.line_height != right.font.terminal.line_height
            }
            Self::TerminalCellWidth(_) => {
                left.font.terminal.cell_width != right.font.terminal.cell_width
            }
            Self::EditorFontFamily(_) => left.font.editor.family != right.font.editor.family,
            Self::EditorFontSize(_) => left.font.editor.size != right.font.editor.size,
            Self::EditorLineHeight(_) => {
                left.font.editor.line_height != right.font.editor.line_height
            }
            Self::AgentChatFontFamily(_) => {
                left.font.agent_chat.family != right.font.agent_chat.family
            }
            Self::AgentChatFontSize(_) => left.font.agent_chat.size != right.font.agent_chat.size,
            Self::AgentChatLineHeight(_) => {
                left.font.agent_chat.line_height != right.font.agent_chat.line_height
            }
            Self::CursorStyle(_) => left.cursor.style != right.cursor.style,
            Self::CursorBlinking(_) => left.cursor.blinking != right.cursor.blinking,
            Self::AgentUseModifierToSend(_) => {
                left.agent.use_modifier_to_send != right.agent.use_modifier_to_send
            }
            Self::AgentCatalog(_) => left.agents != right.agents,
            Self::SessionHosts { .. } => {
                left.session_hosts != right.session_hosts
                    || left.session_host_tombstones != right.session_host_tombstones
            }
            Self::RenderMaxFps(_) => left.render.max_fps != right.render.max_fps,
            Self::ShellClosePaneOnExit(_) => {
                left.shell.close_pane_on_exit != right.shell.close_pane_on_exit
            }
            Self::WindowOpacity(_) => left.window.opacity != right.window.opacity,
            Self::WindowBlur(_) => left.window.blur != right.window.blur,
            Self::ScrollbackMaxRows(_) => left.scrollback.max_rows != right.scrollback.max_rows,
            Self::TerminalInsetX(_) => left.font.terminal.inset_x != right.font.terminal.inset_x,
            Self::TerminalInsetY(_) => left.font.terminal.inset_y != right.font.terminal.inset_y,
            Self::FilesShowHidden(_) => {
                left.left_dock.files_show_hidden != right.left_dock.files_show_hidden
            }
            Self::FilesUseGitignore(_) => {
                left.left_dock.files_use_gitignore != right.left_dock.files_use_gitignore
            }
            Self::SyntaxTheme(_) => left.file_viewer.syntax_theme != right.file_viewer.syntax_theme,
            Self::ClipboardStreamingMaxBytes(_) => {
                left.clipboard.streaming_max_bytes != right.clipboard.streaming_max_bytes
            }
            Self::PreferredEditor(_) => left.editor.preferred != right.editor.preferred,
            Self::PanelsGridColumns(_) => left.panels.grid_columns != right.panels.grid_columns,
            Self::ToggleStatusBarItem(_) => {
                left.status_bar.hidden_items != right.status_bar.hidden_items
            }
            Self::ClaudeStatusEnabled(_) => left.claude_status.enable != right.claude_status.enable,
            Self::TelegramEnabled(_) => left.telegram.enabled != right.telegram.enabled,
            Self::TelegramAuthorizedChatId(_) => {
                left.telegram.authorized_chat_id != right.telegram.authorized_chat_id
            }
        }
    }
}
