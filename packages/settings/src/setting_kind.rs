//! The setting identifiers the `spec` tables are keyed on, their test-only
//! `ALL` lists, and the file-icon colour select values.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextSetting {
    TerminalFontSize,
    TerminalLineHeight,
    TerminalCellWidth,
    EditorFontSize,
    EditorLineHeight,
    AgentChatFontSize,
    AgentChatLineHeight,
    WindowOpacity,
    ScrollbackMaxRows,
    TerminalInsetX,
    TerminalInsetY,
    ClipboardStreamingMaxBytes,
    PanelsGridColumns,
    ClaudeStatusStaleSecs,
    ClaudeStatusFileTtlDays,
    UsageLimitsPollSecs,
    UsageStatusPollSecs,
    PortsPollSecs,
    LogsRetentionDays,
    LogsMaxFileSizeMb,
    PresenceGraceSecs,
    PresenceIdleSecs,
    PresenceIdleForegroundSecs,
    AgentInputMaxRows,
    AgentReadingWidth,
    FlowTimeoutMinutes,
    FlowMaxNodeRuns,
    FlowMaxCost,
    FlowCostCurrency,
    LeftDefaultWidth,
    ShellProgram,
    NotifyLongRunningThresholdSecs,
    KeepAwakeApprovalWaitMinutes,
}

/// Select values for `left_dock.file_icon_color_mode`, matching its
/// `snake_case` config spelling.
pub(crate) const ICON_COLOR: &str = "color";
pub(crate) const ICON_MONOCHROME: &str = "monochrome";

pub(crate) fn icon_color_value(mode: &daruda_config::IconColorMode) -> &'static str {
    match mode {
        daruda_config::IconColorMode::Color => ICON_COLOR,
        daruda_config::IconColorMode::Monochrome => ICON_MONOCHROME,
    }
}

pub(crate) fn icon_color_from_value(value: &str) -> Option<daruda_config::IconColorMode> {
    match value {
        ICON_COLOR => Some(daruda_config::IconColorMode::Color),
        ICON_MONOCHROME => Some(daruda_config::IconColorMode::Monochrome),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectSetting {
    Language,
    TerminalPreset,
    UiPreset,
    TerminalFontFamily,
    EditorFontFamily,
    AgentChatFontFamily,
    CursorStyle,
    RenderMaxFps,
    SyntaxTheme,
    PreferredEditor,
    FileIconColorMode,
    OrchestratorAgent,
    OrchestratorAccount,
    KeepAwakeMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoolSetting {
    AgentUseModifierToSend,
    AgentUseReadingWidth,
    AgentToolSummaryLabels,
    ShellClosePaneOnExit,
    WindowBlur,
    FilesShowHidden,
    FilesUseGitignore,
    ClaudeStatusEnabled,
    UpdateAutoCheck,
    LeftCollapsedByDefault,
    PreviewTab,
    ShellNaturalTextEditing,
    NotifyOsc9,
    NotifyOsc777,
    NotifyAttention,
    NotifyLongRunning,
    NotifySkipFocusedPane,
    NotifyHook,
    NotifyAgentCompletion,
    NotifyAgentWaiting,
    TelegramOnlyWhenAway,
    ClipboardCopyOnSelect,
    GitConfirmCommit,
    GitConfirmPush,
    GitDefaultCommitMessage,
    TelegramEnabled,
    OrchestratorEnabled,
}

// Rust has no reflection over enum variants, and the `spec` tables are looked
// up by variant, so `ALL` is what lets a test prove every variant has exactly
// one row. `ALL` is itself hand-written, so each is paired with an exhaustive
// `match` that the compiler rejects until a new variant is added to the list
// too — otherwise the missing row would only surface as a panic on the first
// click of the new widget.
impl TextSetting {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 33] = [
        Self::TerminalFontSize,
        Self::TerminalLineHeight,
        Self::TerminalCellWidth,
        Self::EditorFontSize,
        Self::EditorLineHeight,
        Self::AgentChatFontSize,
        Self::AgentChatLineHeight,
        Self::WindowOpacity,
        Self::ScrollbackMaxRows,
        Self::TerminalInsetX,
        Self::TerminalInsetY,
        Self::ClipboardStreamingMaxBytes,
        Self::PanelsGridColumns,
        Self::ClaudeStatusStaleSecs,
        Self::ClaudeStatusFileTtlDays,
        Self::UsageLimitsPollSecs,
        Self::UsageStatusPollSecs,
        Self::PortsPollSecs,
        Self::LogsRetentionDays,
        Self::LogsMaxFileSizeMb,
        Self::PresenceGraceSecs,
        Self::PresenceIdleSecs,
        Self::PresenceIdleForegroundSecs,
        Self::AgentInputMaxRows,
        Self::AgentReadingWidth,
        Self::FlowTimeoutMinutes,
        Self::FlowMaxNodeRuns,
        Self::FlowMaxCost,
        Self::FlowCostCurrency,
        Self::LeftDefaultWidth,
        Self::ShellProgram,
        Self::NotifyLongRunningThresholdSecs,
        Self::KeepAwakeApprovalWaitMinutes,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::TerminalFontSize => (),
            Self::TerminalLineHeight => (),
            Self::TerminalCellWidth => (),
            Self::EditorFontSize => (),
            Self::EditorLineHeight => (),
            Self::AgentChatFontSize => (),
            Self::AgentChatLineHeight => (),
            Self::WindowOpacity => (),
            Self::ScrollbackMaxRows => (),
            Self::TerminalInsetX => (),
            Self::TerminalInsetY => (),
            Self::ClipboardStreamingMaxBytes => (),
            Self::PanelsGridColumns => (),
            Self::ClaudeStatusStaleSecs => (),
            Self::ClaudeStatusFileTtlDays => (),
            Self::UsageLimitsPollSecs => (),
            Self::UsageStatusPollSecs => (),
            Self::PortsPollSecs => (),
            Self::LogsRetentionDays => (),
            Self::LogsMaxFileSizeMb => (),
            Self::PresenceGraceSecs => (),
            Self::PresenceIdleSecs => (),
            Self::PresenceIdleForegroundSecs => (),
            Self::AgentInputMaxRows => (),
            Self::AgentReadingWidth => (),
            Self::FlowTimeoutMinutes => (),
            Self::FlowMaxNodeRuns => (),
            Self::FlowMaxCost => (),
            Self::FlowCostCurrency => (),
            Self::LeftDefaultWidth => (),
            Self::ShellProgram => (),
            Self::NotifyLongRunningThresholdSecs => (),
            Self::KeepAwakeApprovalWaitMinutes => (),
        }
    }
}

impl SelectSetting {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 14] = [
        Self::Language,
        Self::TerminalPreset,
        Self::UiPreset,
        Self::TerminalFontFamily,
        Self::EditorFontFamily,
        Self::AgentChatFontFamily,
        Self::CursorStyle,
        Self::RenderMaxFps,
        Self::SyntaxTheme,
        Self::PreferredEditor,
        Self::FileIconColorMode,
        Self::OrchestratorAgent,
        Self::OrchestratorAccount,
        Self::KeepAwakeMode,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::Language => (),
            Self::TerminalPreset => (),
            Self::UiPreset => (),
            Self::TerminalFontFamily => (),
            Self::EditorFontFamily => (),
            Self::AgentChatFontFamily => (),
            Self::CursorStyle => (),
            Self::RenderMaxFps => (),
            Self::SyntaxTheme => (),
            Self::PreferredEditor => (),
            Self::FileIconColorMode => (),
            Self::OrchestratorAgent => (),
            Self::OrchestratorAccount => (),
            Self::KeepAwakeMode => (),
        }
    }
}

impl BoolSetting {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 27] = [
        Self::AgentUseModifierToSend,
        Self::AgentUseReadingWidth,
        Self::AgentToolSummaryLabels,
        Self::ShellClosePaneOnExit,
        Self::WindowBlur,
        Self::FilesShowHidden,
        Self::FilesUseGitignore,
        Self::UpdateAutoCheck,
        Self::LeftCollapsedByDefault,
        Self::PreviewTab,
        Self::ShellNaturalTextEditing,
        Self::NotifyOsc9,
        Self::NotifyOsc777,
        Self::NotifyAttention,
        Self::NotifyLongRunning,
        Self::NotifySkipFocusedPane,
        Self::NotifyHook,
        Self::NotifyAgentCompletion,
        Self::NotifyAgentWaiting,
        Self::TelegramOnlyWhenAway,
        Self::ClipboardCopyOnSelect,
        Self::GitConfirmCommit,
        Self::GitConfirmPush,
        Self::GitDefaultCommitMessage,
        Self::ClaudeStatusEnabled,
        Self::TelegramEnabled,
        Self::OrchestratorEnabled,
    ];

    /// Compile-time guard for `ALL`: this match is exhaustive, so a new
    /// variant fails to build until it is listed above as well.
    #[cfg(test)]
    fn _all_is_exhaustive(self) {
        match self {
            Self::AgentUseModifierToSend => (),
            Self::AgentUseReadingWidth => (),
            Self::AgentToolSummaryLabels => (),
            Self::ShellClosePaneOnExit => (),
            Self::WindowBlur => (),
            Self::FilesShowHidden => (),
            Self::FilesUseGitignore => (),
            Self::UpdateAutoCheck => (),
            Self::LeftCollapsedByDefault => (),
            Self::PreviewTab => (),
            Self::ShellNaturalTextEditing => (),
            Self::NotifyOsc9 => (),
            Self::NotifyOsc777 => (),
            Self::NotifyAttention => (),
            Self::NotifyLongRunning => (),
            Self::NotifySkipFocusedPane => (),
            Self::NotifyHook => (),
            Self::NotifyAgentCompletion => (),
            Self::NotifyAgentWaiting => (),
            Self::TelegramOnlyWhenAway => (),
            Self::ClipboardCopyOnSelect => (),
            Self::GitConfirmCommit => (),
            Self::GitConfirmPush => (),
            Self::GitDefaultCommitMessage => (),
            Self::ClaudeStatusEnabled => (),
            Self::TelegramEnabled => (),
            Self::OrchestratorEnabled => (),
        }
    }
}
