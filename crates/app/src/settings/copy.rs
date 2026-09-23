//! What each simple setting's row says, and where it sits: the page, the
//! card, the label and the hint. The one source for both the page that
//! renders the row and the search that finds it.

use daruda_config::BuiltinSection as Section;

use super::{BoolSetting, SelectSetting, TextSetting};
use crate::surface::strings as s;

#[derive(Clone, Copy)]
pub(super) struct RowCopy {
    pub(super) section: Section,
    /// Title of the card the row sits in.
    pub(super) card: fn() -> String,
    pub(super) label: fn() -> String,
    /// Empty when the row carries no description.
    pub(super) hint: fn() -> String,
}

pub(super) fn text(setting: TextSetting) -> RowCopy {
    match setting {
        TextSetting::TerminalFontSize => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_terminal,
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::TerminalLineHeight => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_terminal,
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::TerminalCellWidth => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_terminal,
            label: s::settings_label_cell_width,
            hint: s::settings_hint_cell_width,
        },
        TextSetting::EditorFontSize => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_editor,
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::EditorLineHeight => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_editor,
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::AgentChatFontSize => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_agent_chat,
            label: s::settings_label_font_size,
            hint: String::new,
        },
        TextSetting::AgentChatLineHeight => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_agent_chat,
            label: s::settings_label_line_height,
            hint: s::settings_hint_line_height,
        },
        TextSetting::WindowOpacity => RowCopy {
            section: Section::Appearance,
            card: s::settings_group_window,
            label: s::settings_label_window_opacity,
            hint: s::settings_hint_opacity,
        },
        TextSetting::ScrollbackMaxRows => RowCopy {
            section: Section::Terminal,
            card: s::settings_group_rendering,
            label: s::settings_label_scrollback,
            hint: s::settings_hint_scrollback,
        },
        TextSetting::TerminalInsetX => RowCopy {
            section: Section::Terminal,
            card: s::settings_group_insets,
            label: s::settings_label_inset_x,
            hint: String::new,
        },
        TextSetting::TerminalInsetY => RowCopy {
            section: Section::Terminal,
            card: s::settings_group_insets,
            label: s::settings_label_inset_y,
            hint: String::new,
        },
        TextSetting::ClipboardStreamingMaxBytes => RowCopy {
            section: Section::Terminal,
            card: s::settings_card_advanced,
            label: s::settings_label_clipboard_streaming,
            hint: s::settings_hint_clipboard_streaming,
        },
        TextSetting::PanelsGridColumns => RowCopy {
            section: Section::Workspace,
            card: s::settings_section_panels,
            label: s::settings_label_grid_columns,
            hint: String::new,
        },
        TextSetting::LeftDefaultWidth => RowCopy {
            section: Section::Workspace,
            card: s::settings_section_sidebar,
            label: s::settings_label_left_default_width,
            hint: s::settings_hint_new_windows_width,
        },
        TextSetting::ShellProgram => RowCopy {
            section: Section::Terminal,
            card: s::settings_card_shell,
            label: s::settings_label_shell_program,
            hint: s::settings_hint_shell_program,
        },
        TextSetting::NotifyLongRunningThresholdSecs => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_terminal_programs,
            label: s::settings_label_notify_long_running_threshold,
            hint: String::new,
        },
        TextSetting::AgentInputMaxRows => RowCopy {
            section: Section::Agent,
            card: s::settings_group_chat,
            label: s::settings_label_input_max_rows,
            hint: s::settings_hint_input_max_rows,
        },
        TextSetting::AgentReadingWidth => RowCopy {
            section: Section::Agent,
            card: s::settings_group_chat,
            label: s::settings_label_reading_width,
            hint: String::new,
        },
        TextSetting::FlowTimeoutMinutes => RowCopy {
            section: Section::Agent,
            card: s::settings_card_flows,
            label: s::settings_label_flow_timeout,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowMaxNodeRuns => RowCopy {
            section: Section::Agent,
            card: s::settings_card_flows,
            label: s::settings_label_flow_max_node_runs,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowMaxCost => RowCopy {
            section: Section::Agent,
            card: s::settings_card_flows,
            label: s::settings_label_flow_max_cost,
            hint: s::settings_hint_zero_no_limit,
        },
        TextSetting::FlowCostCurrency => RowCopy {
            section: Section::Agent,
            card: s::settings_card_flows,
            label: s::settings_label_flow_currency,
            hint: s::settings_hint_flow_currency,
        },
    }
}

pub(super) fn select(setting: SelectSetting) -> RowCopy {
    match setting {
        SelectSetting::Language => RowCopy {
            section: Section::General,
            card: s::settings_group_language,
            label: s::settings_label_language,
            hint: s::settings_hint_user_scope,
        },
        SelectSetting::TerminalPreset => RowCopy {
            section: Section::Appearance,
            card: s::settings_group_themes,
            label: s::settings_label_terminal_theme,
            hint: s::settings_hint_terminal_theme,
        },
        SelectSetting::UiPreset => RowCopy {
            section: Section::Appearance,
            card: s::settings_group_themes,
            label: s::settings_label_ui_theme,
            hint: s::settings_hint_ui_theme,
        },
        SelectSetting::TerminalFontFamily => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_terminal,
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::EditorFontFamily => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_editor,
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::AgentChatFontFamily => RowCopy {
            section: Section::Font,
            card: s::settings_font_domain_agent_chat,
            label: s::settings_label_font_family,
            hint: String::new,
        },
        SelectSetting::CursorStyle => RowCopy {
            section: Section::Terminal,
            card: s::settings_card_cursor,
            label: s::settings_label_cursor_style,
            hint: s::settings_hint_cursor_style,
        },
        SelectSetting::RenderMaxFps => RowCopy {
            section: Section::Terminal,
            card: s::settings_group_rendering,
            label: s::settings_label_max_fps,
            hint: String::new,
        },
        SelectSetting::SyntaxTheme => RowCopy {
            section: Section::Appearance,
            card: s::settings_group_themes,
            label: s::settings_label_syntax_theme,
            hint: s::settings_hint_syntax_theme,
        },
        SelectSetting::PreferredEditor => RowCopy {
            section: Section::Workspace,
            card: s::settings_card_external_editor,
            label: s::settings_label_preferred_editor,
            hint: String::new,
        },
        SelectSetting::FileIconColorMode => RowCopy {
            section: Section::Workspace,
            card: s::settings_card_files,
            label: s::settings_label_file_icon_colors,
            hint: String::new,
        },
        SelectSetting::OrchestratorAgent => RowCopy {
            section: Section::Orchestrator,
            card: s::settings_nav_orchestrator,
            label: s::settings_orchestrator_agent_label,
            hint: String::new,
        },
        SelectSetting::OrchestratorAccount => RowCopy {
            section: Section::Orchestrator,
            card: s::settings_nav_orchestrator,
            label: s::settings_orchestrator_account_label,
            hint: String::new,
        },
    }
}

pub(super) fn bool(setting: BoolSetting) -> RowCopy {
    match setting {
        BoolSetting::AgentUseModifierToSend => RowCopy {
            section: Section::Agent,
            card: s::settings_group_chat,
            label: s::settings_label_agent_use_modifier_to_send,
            hint: s::settings_agent_use_modifier_to_send_description,
        },
        BoolSetting::AgentUseReadingWidth => RowCopy {
            section: Section::Agent,
            card: s::settings_group_chat,
            label: s::settings_label_agent_use_reading_width,
            hint: s::settings_hint_reading_width,
        },
        BoolSetting::ShellClosePaneOnExit => RowCopy {
            section: Section::Terminal,
            card: s::settings_card_shell,
            label: s::settings_label_close_on_exit,
            hint: String::new,
        },
        BoolSetting::WindowBlur => RowCopy {
            section: Section::Appearance,
            card: s::settings_group_window,
            label: s::settings_label_window_blur,
            hint: s::settings_hint_blur,
        },
        BoolSetting::FilesShowHidden => RowCopy {
            section: Section::Workspace,
            card: s::settings_card_files,
            label: s::settings_label_show_hidden,
            hint: String::new,
        },
        BoolSetting::FilesUseGitignore => RowCopy {
            section: Section::Workspace,
            card: s::settings_card_files,
            label: s::settings_label_use_gitignore,
            hint: String::new,
        },
        BoolSetting::ClaudeStatusEnabled => RowCopy {
            section: Section::Agent,
            card: s::settings_section_claude_status,
            label: s::settings_label_claude_status_enable,
            hint: String::new,
        },
        BoolSetting::LeftCollapsedByDefault => RowCopy {
            section: Section::Workspace,
            card: s::settings_section_sidebar,
            label: s::settings_label_left_collapsed,
            hint: s::settings_hint_new_windows_state,
        },
        BoolSetting::PreviewTab => RowCopy {
            section: Section::Workspace,
            card: s::settings_card_files,
            label: s::settings_label_preview_tab,
            hint: s::settings_hint_preview_tab,
        },
        BoolSetting::ShellNaturalTextEditing => RowCopy {
            section: Section::Terminal,
            card: s::settings_card_shell,
            label: s::settings_label_natural_text_editing,
            hint: s::settings_hint_natural_text_editing,
        },
        BoolSetting::NotifyOsc9 => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_terminal_programs,
            label: s::settings_label_notify_osc9,
            hint: s::settings_hint_notify_osc9,
        },
        BoolSetting::NotifyOsc777 => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_terminal_programs,
            label: s::settings_label_notify_osc777,
            hint: s::settings_hint_notify_osc777,
        },
        BoolSetting::NotifyAttention => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_terminal_programs,
            label: s::settings_label_notify_attention,
            hint: s::settings_hint_notify_attention,
        },
        BoolSetting::NotifyLongRunning => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_terminal_programs,
            label: s::settings_label_notify_long_running,
            hint: s::settings_hint_notify_long_running,
        },
        BoolSetting::NotifySkipFocusedPane => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_notify_behavior,
            label: s::settings_label_notify_skip_focused,
            hint: s::settings_hint_notify_skip_focused,
        },
        BoolSetting::NotifyHook => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_claude_code_terminals,
            label: s::settings_label_notify_hook,
            hint: s::settings_hint_notify_hook,
        },
        BoolSetting::NotifyAgentCompletion => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_agent_chat_notifications,
            label: s::settings_label_notify_agent_completion,
            hint: String::new,
        },
        BoolSetting::NotifyAgentWaiting => RowCopy {
            section: Section::Notifications,
            card: s::settings_card_agent_chat_notifications,
            label: s::settings_label_notify_agent_waiting,
            hint: String::new,
        },
        BoolSetting::TelegramOnlyWhenAway => RowCopy {
            section: Section::RemoteControl,
            card: s::settings_telegram_heading,
            label: s::remote_only_when_away,
            hint: s::settings_hint_only_when_away,
        },
        BoolSetting::TelegramEnabled => RowCopy {
            section: Section::RemoteControl,
            card: s::settings_telegram_heading,
            label: s::settings_telegram_enabled_label,
            hint: String::new,
        },
        BoolSetting::OrchestratorEnabled => RowCopy {
            section: Section::Orchestrator,
            card: s::settings_nav_orchestrator,
            label: s::settings_orchestrator_enabled_label,
            hint: String::new,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_a_label_and_a_card() {
        let copies = TextSetting::ALL
            .iter()
            .map(|t| text(*t))
            .chain(SelectSetting::ALL.iter().map(|v| select(*v)))
            .chain(BoolSetting::ALL.iter().map(|b| bool(*b)));
        for copy in copies {
            assert!(!(copy.label)().is_empty());
            assert!(!(copy.card)().is_empty());
        }
    }

    /// A text row's tab bucket and its search section are the same page.
    #[test]
    fn text_rows_agree_with_their_spec_section() {
        for spec in super::super::spec::TEXT_SETTINGS {
            assert_eq!(
                text(spec.setting).section,
                spec.section,
                "{:?}",
                spec.setting
            );
        }
    }
}
