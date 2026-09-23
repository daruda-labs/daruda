//! What each simple setting's row says: its label and its hint. The one
//! source for both the page that renders the row and the search that finds
//! it; where the row sits is [`super::layout`].

use super::{BoolSetting, SelectSetting, TextSetting};
use crate::surface::strings as s;

#[derive(Clone, Copy)]
pub(super) struct RowCopy {
    pub(super) label: fn() -> String,
    /// Empty when the row carries no description.
    pub(super) hint: fn() -> String,
}

pub(super) fn text(setting: TextSetting) -> RowCopy {
    match setting {
        TextSetting::TerminalFontSize => RowCopy {
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::TerminalLineHeight => RowCopy {
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::TerminalCellWidth => RowCopy {
            label: s::settings_label_cell_width,
            hint: s::settings_hint_cell_width,
        },
        TextSetting::EditorFontSize => RowCopy {
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::EditorLineHeight => RowCopy {
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::AgentChatFontSize => RowCopy {
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::AgentChatLineHeight => RowCopy {
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::WindowOpacity => RowCopy {
            label: s::settings_label_window_opacity,
            hint: s::settings_hint_opacity,
        },
        TextSetting::ScrollbackMaxRows => RowCopy {
            label: s::settings_label_scrollback,
            hint: s::settings_hint_scrollback,
        },
        TextSetting::TerminalInsetX => RowCopy {
            label: s::settings_label_inset_x,
            hint: String::new,
        },
        TextSetting::TerminalInsetY => RowCopy {
            label: s::settings_label_inset_y,
            hint: String::new,
        },
        TextSetting::ClipboardStreamingMaxBytes => RowCopy {
            label: s::settings_label_clipboard_streaming,
            hint: s::settings_hint_clipboard_streaming,
        },
        TextSetting::PanelsGridColumns => RowCopy {
            label: s::settings_label_grid_columns,
            hint: String::new,
        },
        TextSetting::LeftDefaultWidth => RowCopy {
            label: s::settings_label_left_default_width,
            hint: s::settings_hint_new_windows_width,
        },
        TextSetting::ShellProgram => RowCopy {
            label: s::settings_label_shell_program,
            hint: s::settings_hint_shell_program,
        },
        TextSetting::NotifyLongRunningThresholdSecs => RowCopy {
            label: s::settings_label_notify_long_running_threshold,
            hint: String::new,
        },
        TextSetting::AgentInputMaxRows => RowCopy {
            label: s::settings_label_input_max_rows,
            hint: s::settings_hint_input_max_rows,
        },
        TextSetting::AgentReadingWidth => RowCopy {
            label: s::settings_label_reading_width,
            hint: String::new,
        },
        TextSetting::FlowTimeoutMinutes => RowCopy {
            label: s::settings_label_flow_timeout,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowMaxNodeRuns => RowCopy {
            label: s::settings_label_flow_max_node_runs,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowMaxCost => RowCopy {
            label: s::settings_label_flow_max_cost,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowCostCurrency => RowCopy {
            label: s::settings_label_flow_currency,
            hint: s::settings_hint_flow_currency,
        },
        TextSetting::ClaudeStatusStaleSecs => RowCopy {
            label: s::settings_label_stale_threshold,
            hint: s::settings_hint_stale_threshold,
        },
        TextSetting::ClaudeStatusFileTtlDays => RowCopy {
            label: s::settings_label_file_ttl,
            hint: s::settings_hint_file_ttl,
        },
        TextSetting::UsageLimitsPollSecs => RowCopy {
            label: s::settings_label_usage_limits_poll,
            hint: s::settings_hint_usage_poll,
        },
        TextSetting::UsageStatusPollSecs => RowCopy {
            label: s::settings_label_usage_status_poll,
            hint: s::settings_hint_usage_poll,
        },
        TextSetting::PortsPollSecs => RowCopy {
            label: s::settings_label_ports_poll,
            hint: String::new,
        },
        TextSetting::LogsRetentionDays => RowCopy {
            label: s::settings_label_logs_retention,
            hint: s::settings_hint_logs_retention,
        },
        TextSetting::LogsMaxFileSizeMb => RowCopy {
            label: s::settings_label_logs_max_size,
            hint: s::settings_hint_logs_max_size,
        },
        TextSetting::PresenceGraceSecs => RowCopy {
            label: s::settings_label_presence_grace,
            hint: s::settings_hint_presence_grace,
        },
        TextSetting::PresenceIdleSecs => RowCopy {
            label: s::settings_label_presence_idle,
            hint: s::settings_hint_presence_idle,
        },
        TextSetting::PresenceIdleForegroundSecs => RowCopy {
            label: s::settings_label_presence_idle_foreground,
            hint: s::settings_hint_presence_idle_foreground,
        },
    }
}

pub(super) fn select(setting: SelectSetting) -> RowCopy {
    match setting {
        SelectSetting::Language => RowCopy {
            label: s::settings_label_language,
            hint: s::settings_hint_user_scope,
        },
        SelectSetting::TerminalPreset => RowCopy {
            label: s::settings_label_terminal_theme,
            hint: s::settings_hint_terminal_theme,
        },
        SelectSetting::UiPreset => RowCopy {
            label: s::settings_label_ui_theme,
            hint: s::settings_hint_ui_theme,
        },
        SelectSetting::TerminalFontFamily => RowCopy {
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::EditorFontFamily => RowCopy {
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::AgentChatFontFamily => RowCopy {
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::CursorStyle => RowCopy {
            label: s::settings_label_cursor_style,
            hint: s::settings_hint_cursor_style,
        },
        SelectSetting::RenderMaxFps => RowCopy {
            label: s::settings_label_max_fps,
            hint: String::new,
        },
        SelectSetting::SyntaxTheme => RowCopy {
            label: s::settings_label_syntax_theme,
            hint: s::settings_hint_syntax_theme,
        },
        SelectSetting::PreferredEditor => RowCopy {
            label: s::settings_label_preferred_editor,
            hint: String::new,
        },
        SelectSetting::FileIconColorMode => RowCopy {
            label: s::settings_label_file_icon_colors,
            hint: String::new,
        },
        SelectSetting::OrchestratorAgent => RowCopy {
            label: s::settings_orchestrator_agent_label,
            hint: String::new,
        },
        SelectSetting::OrchestratorAccount => RowCopy {
            label: s::settings_orchestrator_account_label,
            hint: String::new,
        },
    }
}

pub(super) fn bool(setting: BoolSetting) -> RowCopy {
    match setting {
        BoolSetting::AgentUseModifierToSend => RowCopy {
            label: s::settings_label_agent_use_modifier_to_send,
            hint: s::settings_agent_use_modifier_to_send_description,
        },
        BoolSetting::AgentUseReadingWidth => RowCopy {
            label: s::settings_label_agent_use_reading_width,
            hint: s::settings_hint_reading_width,
        },
        BoolSetting::ShellClosePaneOnExit => RowCopy {
            label: s::settings_label_close_on_exit,
            hint: String::new,
        },
        BoolSetting::WindowBlur => RowCopy {
            label: s::settings_label_window_blur,
            hint: s::settings_hint_blur,
        },
        BoolSetting::FilesShowHidden => RowCopy {
            label: s::settings_label_show_hidden,
            hint: String::new,
        },
        BoolSetting::FilesUseGitignore => RowCopy {
            label: s::settings_label_use_gitignore,
            hint: String::new,
        },
        BoolSetting::ClaudeStatusEnabled => RowCopy {
            label: s::settings_label_claude_status_enable,
            hint: String::new,
        },
        BoolSetting::LeftCollapsedByDefault => RowCopy {
            label: s::settings_label_left_collapsed,
            hint: s::settings_hint_new_windows_state,
        },
        BoolSetting::PreviewTab => RowCopy {
            label: s::settings_label_preview_tab,
            hint: s::settings_hint_preview_tab,
        },
        BoolSetting::ShellNaturalTextEditing => RowCopy {
            label: s::settings_label_natural_text_editing,
            hint: s::settings_hint_natural_text_editing,
        },
        BoolSetting::NotifyOsc9 => RowCopy {
            label: s::settings_label_notify_osc9,
            hint: s::settings_hint_notify_osc9,
        },
        BoolSetting::NotifyOsc777 => RowCopy {
            label: s::settings_label_notify_osc777,
            hint: s::settings_hint_notify_osc777,
        },
        BoolSetting::NotifyAttention => RowCopy {
            label: s::settings_label_notify_attention,
            hint: s::settings_hint_notify_attention,
        },
        BoolSetting::NotifyLongRunning => RowCopy {
            label: s::settings_label_notify_long_running,
            hint: s::settings_hint_notify_long_running,
        },
        BoolSetting::NotifySkipFocusedPane => RowCopy {
            label: s::settings_label_notify_skip_focused,
            hint: s::settings_hint_notify_skip_focused,
        },
        BoolSetting::NotifyHook => RowCopy {
            label: s::settings_label_notify_hook,
            hint: s::settings_hint_notify_hook,
        },
        BoolSetting::NotifyAgentCompletion => RowCopy {
            label: s::settings_label_notify_agent_completion,
            hint: String::new,
        },
        BoolSetting::NotifyAgentWaiting => RowCopy {
            label: s::settings_label_notify_agent_waiting,
            hint: String::new,
        },
        BoolSetting::TelegramOnlyWhenAway => RowCopy {
            label: s::remote_only_when_away,
            hint: s::settings_hint_only_when_away,
        },
        BoolSetting::TelegramEnabled => RowCopy {
            label: s::settings_telegram_enabled_label,
            hint: String::new,
        },
        BoolSetting::OrchestratorEnabled => RowCopy {
            label: s::settings_orchestrator_enabled_label,
            hint: String::new,
        },
        BoolSetting::UpdateAutoCheck => RowCopy {
            label: s::settings_label_update_auto_check,
            hint: s::settings_hint_update_auto_check,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_a_label() {
        let copies = TextSetting::ALL
            .iter()
            .map(|t| text(*t))
            .chain(SelectSetting::ALL.iter().map(|v| select(*v)))
            .chain(BoolSetting::ALL.iter().map(|b| bool(*b)));
        for copy in copies {
            assert!(!(copy.label)().is_empty());
        }
    }
}
