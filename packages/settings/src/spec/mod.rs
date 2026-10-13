//! Each simple setting as a table row: the widget that drives it, how it reads
//! out of a [`Config`], and how it turns back into a [`SettingsPatch`]. Text-input
//! rows are in `text.rs`; select and toggle rows are here. Settings edited as a
//! whole sub-form (agent catalog, session hosts) are not rows; `catalog/` and
//! `hosts/` own them.

use daruda_config::{Config, SettingsPatch};
use gpui::{Entity, SharedString};

use super::{BoolSetting, SelectSetting, SettingsView, TextSetting};
use daruda_ui::select::SelectState;

mod text;
pub(super) use text::{TEXT_SETTINGS, TextSpec};

/// How a select's widget takes a new value when the config changes underneath
/// the window. Three shapes, because two families cannot just be handed a
/// string: their option *list* is derived from live state too.
pub(super) enum SelectLoad {
    /// The option list is fixed at construction; only the selection moves.
    Value,
    /// Font pickers rebuild their list so the live family stays selectable even
    /// when it is not among the installed fonts.
    Font,
    /// Orchestrator pickers derive both list and selection from live state, so
    /// they own the whole refresh.
    Rebuild(fn(&SettingsView, &mut gpui::Window, &mut gpui::Context<SettingsView>)),
}

/// A setting shown as a dropdown.
pub(super) struct SelectSpec {
    pub(super) setting: SelectSetting,
    pub(super) field: fn(&SettingsView) -> &Entity<SelectState>,
    /// A selected option string as a patch. `None` when the string names
    /// nothing valid — a live edit then does nothing and the draft falls back
    /// to [`Self::current`].
    pub(super) read: fn(&str) -> Option<SettingsPatch>,
    /// What `config` currently holds, as a patch. Both the draft's fallback for
    /// an unselected widget and the window's "I own this field" declaration.
    pub(super) current: fn(&Config) -> SettingsPatch,
    /// The option string that shows `config`'s current value.
    pub(super) show: fn(&Config) -> SharedString,
    pub(super) load: SelectLoad,
}

pub(super) const SELECT_SETTINGS: &[SelectSpec] = &[
    SelectSpec {
        setting: SelectSetting::Language,
        field: |w| &w.language_select,
        read: |v| Some(SettingsPatch::GeneralLanguage(v.to_owned())),
        current: |c| SettingsPatch::GeneralLanguage(c.general.language.clone()),
        show: |c| c.general.language.clone().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::TerminalPreset,
        field: |w| &w.terminal_preset_select,
        read: |v| Some(SettingsPatch::TerminalPreset(v.to_owned())),
        current: |c| SettingsPatch::TerminalPreset(c.theme.terminal_preset.clone()),
        show: |c| c.theme.terminal_preset.clone().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::UiPreset,
        field: |w| &w.ui_preset_select,
        read: |v| Some(SettingsPatch::UiPreset(v.to_owned())),
        current: |c| SettingsPatch::UiPreset(c.theme.ui_preset.clone()),
        show: |c| c.theme.ui_preset.clone().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::TerminalFontFamily,
        field: |w| &w.terminal_font_family_select,
        read: |v| Some(SettingsPatch::TerminalFontFamily(v.to_owned())),
        current: |c| SettingsPatch::TerminalFontFamily(c.font.terminal.family.clone()),
        show: |c| c.font.terminal.family.clone().into(),
        load: SelectLoad::Font,
    },
    SelectSpec {
        setting: SelectSetting::EditorFontFamily,
        field: |w| &w.editor_font_family_select,
        read: |v| Some(SettingsPatch::EditorFontFamily(v.to_owned())),
        current: |c| SettingsPatch::EditorFontFamily(c.font.editor.family.clone()),
        show: |c| c.font.editor.family.clone().into(),
        load: SelectLoad::Font,
    },
    SelectSpec {
        setting: SelectSetting::AgentChatFontFamily,
        field: |w| &w.agent_chat_font_family_select,
        read: |v| Some(SettingsPatch::AgentChatFontFamily(v.to_owned())),
        current: |c| SettingsPatch::AgentChatFontFamily(c.font.agent_chat.family.clone()),
        show: |c| c.font.agent_chat.family.clone().into(),
        load: SelectLoad::Font,
    },
    SelectSpec {
        setting: SelectSetting::CursorStyle,
        field: |w| &w.cursor_style_select,
        read: |v| Some(SettingsPatch::CursorStyle(cursor_style_from_option(v))),
        current: |c| SettingsPatch::CursorStyle(c.cursor.style),
        show: |c| cursor_style_option(c.cursor.style).into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::RenderMaxFps,
        field: |w| &w.max_fps_select,
        read: |v| v.parse::<u32>().ok().map(SettingsPatch::RenderMaxFps),
        current: |c| SettingsPatch::RenderMaxFps(c.render.max_fps),
        show: |c| c.render.max_fps.to_string().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::SyntaxTheme,
        field: |w| &w.syntax_theme_select,
        read: |v| Some(SettingsPatch::SyntaxTheme(v.to_owned())),
        current: |c| SettingsPatch::SyntaxTheme(c.file_viewer.syntax_theme.clone()),
        show: |c| c.file_viewer.syntax_theme.clone().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::PreferredEditor,
        field: |w| &w.editor_select,
        read: |v| Some(SettingsPatch::PreferredEditor(v.to_owned())),
        current: |c| SettingsPatch::PreferredEditor(c.editor.preferred.clone()),
        show: |c| c.editor.preferred.clone().into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::FileIconColorMode,
        field: |w| &w.file_icon_color_select,
        read: |v| super::icon_color_from_value(v).map(SettingsPatch::FileIconColorMode),
        current: |c| SettingsPatch::FileIconColorMode(c.left_dock.file_icon_color_mode.clone()),
        show: |c| super::icon_color_value(&c.left_dock.file_icon_color_mode).into(),
        load: SelectLoad::Value,
    },
    SelectSpec {
        setting: SelectSetting::OrchestratorAgent,
        field: |w| &w.orchestrator_agent_select,
        read: |v| {
            Some(SettingsPatch::OrchestratorAgentId(
                super::sections::orchestrator::agent_id_from_select(v.to_owned()),
            ))
        },
        current: |c| SettingsPatch::OrchestratorAgentId(c.orchestrator.agent_id.clone()),
        show: |c| super::sections::orchestrator::agent_select_value(c),
        load: SelectLoad::Rebuild(SettingsView::refresh_orchestrator_agent_select),
    },
    SelectSpec {
        setting: SelectSetting::OrchestratorAccount,
        field: |w| &w.orchestrator_account_select,
        read: |v| {
            Some(SettingsPatch::OrchestratorAccountId(
                super::sections::orchestrator::account_id_from_select(v),
            ))
        },
        current: |c| SettingsPatch::OrchestratorAccountId(c.orchestrator.account_id),
        show: |c| super::sections::orchestrator::account_select_value(c),
        load: SelectLoad::Rebuild(SettingsView::refresh_orchestrator_account_select),
    },
];

/// The option string the cursor-style picker uses for `style`. Paired with
/// [`cursor_style_from_option`]; the two were previously written out
/// separately in `persist_select_setting` and `load_settings_patch`.
fn cursor_style_option(style: daruda_config::CursorStyle) -> &'static str {
    match style {
        daruda_config::CursorStyle::Block => "block",
        daruda_config::CursorStyle::Underline => "underline",
        daruda_config::CursorStyle::Bar => "bar",
    }
}

fn cursor_style_from_option(value: &str) -> daruda_config::CursorStyle {
    match value {
        "underline" => daruda_config::CursorStyle::Underline,
        "bar" => daruda_config::CursorStyle::Bar,
        _ => daruda_config::CursorStyle::Block,
    }
}

/// A setting shown as a checkbox. The window mirrors each one as a plain
/// `bool` field, since a checkbox has no state entity of its own.
pub(super) struct BoolSpec {
    pub(super) setting: BoolSetting,
    /// Read the window's mirror of this checkbox.
    pub(super) get: fn(&SettingsView) -> bool,
    /// Write it. Split from `get` rather than handing out `&mut bool` so a
    /// read-only caller (the whole-config draft) needs no mutable window.
    pub(super) set: fn(&mut SettingsView, bool),
    pub(super) patch: fn(bool) -> SettingsPatch,
    pub(super) show: fn(&Config) -> bool,
}

pub(super) const BOOL_SETTINGS: &[BoolSpec] = &[
    BoolSpec {
        setting: BoolSetting::AgentUseModifierToSend,
        get: |w| w.agent_use_modifier_to_send,
        set: |w, v| w.agent_use_modifier_to_send = v,
        patch: SettingsPatch::AgentUseModifierToSend,
        show: |c| c.agent.use_modifier_to_send,
    },
    BoolSpec {
        setting: BoolSetting::AgentUseReadingWidth,
        get: |w| w.agent_use_reading_width,
        set: |w, v| w.agent_use_reading_width = v,
        patch: SettingsPatch::AgentUseReadingWidth,
        show: |c| c.agent.use_reading_width,
    },
    BoolSpec {
        setting: BoolSetting::AgentToolSummaryLabels,
        get: |w| w.agent_tool_summary_labels,
        set: |w, v| w.agent_tool_summary_labels = v,
        patch: SettingsPatch::AgentToolSummaryLabels,
        show: |c| c.agent.tool_summary_labels,
    },
    BoolSpec {
        setting: BoolSetting::ShellClosePaneOnExit,
        get: |w| w.close_pane_on_exit,
        set: |w, v| w.close_pane_on_exit = v,
        patch: SettingsPatch::ShellClosePaneOnExit,
        show: |c| c.shell.close_pane_on_exit,
    },
    BoolSpec {
        setting: BoolSetting::WindowBlur,
        get: |w| w.window_blur,
        set: |w, v| w.window_blur = v,
        patch: SettingsPatch::WindowBlur,
        show: |c| c.window.blur,
    },
    BoolSpec {
        setting: BoolSetting::FilesShowHidden,
        get: |w| w.files_show_hidden,
        set: |w, v| w.files_show_hidden = v,
        patch: SettingsPatch::FilesShowHidden,
        show: |c| c.left_dock.files_show_hidden,
    },
    BoolSpec {
        setting: BoolSetting::FilesUseGitignore,
        get: |w| w.files_use_gitignore,
        set: |w, v| w.files_use_gitignore = v,
        patch: SettingsPatch::FilesUseGitignore,
        show: |c| c.left_dock.files_use_gitignore,
    },
    BoolSpec {
        setting: BoolSetting::UpdateAutoCheck,
        get: |w| w.update_auto_check,
        set: |w, v| w.update_auto_check = v,
        patch: SettingsPatch::UpdateAutoCheck,
        show: |c| c.update.auto_check,
    },
    BoolSpec {
        setting: BoolSetting::LeftCollapsedByDefault,
        get: |w| w.left_collapsed_by_default,
        set: |w, v| w.left_collapsed_by_default = v,
        patch: SettingsPatch::LeftCollapsedByDefault,
        show: |c| c.left_dock.left_collapsed_by_default,
    },
    BoolSpec {
        setting: BoolSetting::PreviewTab,
        get: |w| w.preview_tab,
        set: |w, v| w.preview_tab = v,
        patch: SettingsPatch::PreviewTab,
        show: |c| c.file_viewer.preview_tab,
    },
    BoolSpec {
        setting: BoolSetting::ShellNaturalTextEditing,
        get: |w| w.shell_natural_text_editing,
        set: |w, v| w.shell_natural_text_editing = v,
        patch: SettingsPatch::ShellNaturalTextEditing,
        show: |c| c.shell.natural_text_editing,
    },
    BoolSpec {
        setting: BoolSetting::NotifyOsc9,
        get: |w| w.notify_osc9,
        set: |w, v| w.notify_osc9 = v,
        patch: SettingsPatch::NotifyOsc9,
        show: |c| c.notifications.osc9_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifyOsc777,
        get: |w| w.notify_osc777,
        set: |w, v| w.notify_osc777 = v,
        patch: SettingsPatch::NotifyOsc777,
        show: |c| c.notifications.osc777_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifyAttention,
        get: |w| w.notify_attention,
        set: |w, v| w.notify_attention = v,
        patch: SettingsPatch::NotifyAttention,
        show: |c| c.notifications.attention_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifyLongRunning,
        get: |w| w.notify_long_running,
        set: |w, v| w.notify_long_running = v,
        patch: SettingsPatch::NotifyLongRunning,
        show: |c| c.notifications.long_running_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifySkipFocusedPane,
        get: |w| w.notify_skip_focused_pane,
        set: |w, v| w.notify_skip_focused_pane = v,
        patch: SettingsPatch::NotifySkipFocusedPane,
        show: |c| c.notifications.skip_focused_pane,
    },
    BoolSpec {
        setting: BoolSetting::ClipboardCopyOnSelect,
        get: |w| w.clipboard_copy_on_select,
        set: |w, v| w.clipboard_copy_on_select = v,
        patch: SettingsPatch::ClipboardCopyOnSelect,
        show: |c| c.clipboard.copy_on_select,
    },
    BoolSpec {
        setting: BoolSetting::GitConfirmCommit,
        get: |w| w.git_confirm_commit,
        set: |w, v| w.git_confirm_commit = v,
        patch: SettingsPatch::GitConfirmCommit,
        show: |c| c.git.confirm_commit,
    },
    BoolSpec {
        setting: BoolSetting::GitConfirmPush,
        get: |w| w.git_confirm_push,
        set: |w, v| w.git_confirm_push = v,
        patch: SettingsPatch::GitConfirmPush,
        show: |c| c.git.confirm_push,
    },
    BoolSpec {
        setting: BoolSetting::GitDefaultCommitMessage,
        get: |w| w.git_default_commit_message,
        set: |w, v| w.git_default_commit_message = v,
        patch: SettingsPatch::GitDefaultCommitMessage,
        show: |c| c.git.default_commit_message,
    },
    BoolSpec {
        setting: BoolSetting::NotifyHook,
        get: |w| w.notify_hook,
        set: |w, v| w.notify_hook = v,
        patch: SettingsPatch::NotifyHook,
        show: |c| c.notifications.hook_notification_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifyAgentCompletion,
        get: |w| w.notify_agent_completion,
        set: |w, v| w.notify_agent_completion = v,
        patch: SettingsPatch::NotifyAgentCompletion,
        show: |c| c.notifications.agent_completion_enabled,
    },
    BoolSpec {
        setting: BoolSetting::NotifyAgentWaiting,
        get: |w| w.notify_agent_waiting,
        set: |w, v| w.notify_agent_waiting = v,
        patch: SettingsPatch::NotifyAgentWaiting,
        show: |c| c.notifications.agent_waiting_enabled,
    },
    BoolSpec {
        setting: BoolSetting::TelegramOnlyWhenAway,
        get: |w| w.telegram_only_when_away,
        set: |w, v| w.telegram_only_when_away = v,
        patch: SettingsPatch::TelegramOnlyWhenAway,
        show: |c| c.telegram.only_when_away,
    },
    BoolSpec {
        setting: BoolSetting::ClaudeStatusEnabled,
        get: |w| w.claude_status_enable,
        set: |w, v| w.claude_status_enable = v,
        patch: SettingsPatch::ClaudeStatusEnabled,
        show: |c| c.claude_status.enable,
    },
    BoolSpec {
        setting: BoolSetting::TelegramEnabled,
        get: |w| w.telegram_enabled,
        set: |w, v| w.telegram_enabled = v,
        patch: SettingsPatch::TelegramEnabled,
        show: |c| c.telegram.enabled,
    },
    BoolSpec {
        setting: BoolSetting::OrchestratorEnabled,
        get: |w| w.orchestrator_enabled,
        set: |w, v| w.orchestrator_enabled = v,
        patch: SettingsPatch::OrchestratorEnabled,
        show: |c| c.orchestrator.enabled,
    },
];

/// The `SettingsFieldId` each row owns, derived from the row rather than
/// declared beside it — a fifth fact per row is a fifth thing that can drift.
/// The config only supplies a value to build the patch around; the variant,
/// which is all `field()` reads, does not depend on it. Shared rather than
/// built per call, since `load_settings_patch` scans every row.
static PROBE_CONFIG: std::sync::LazyLock<Config> = std::sync::LazyLock::new(Config::default);

pub(super) fn text_field_id(spec: &TextSpec) -> daruda_config::SettingsFieldId {
    (spec.current)(&PROBE_CONFIG).field()
}

pub(super) fn select_field_id(spec: &SelectSpec) -> daruda_config::SettingsFieldId {
    (spec.current)(&PROBE_CONFIG).field()
}

pub(super) fn bool_field_id(spec: &BoolSpec) -> daruda_config::SettingsFieldId {
    (spec.patch)(false).field()
}

pub(super) fn text_spec(setting: TextSetting) -> &'static TextSpec {
    TEXT_SETTINGS
        .iter()
        .find(|spec| spec.setting == setting)
        .expect("every TextSetting has a row (see spec::tests)")
}

pub(super) fn select_spec(setting: SelectSetting) -> &'static SelectSpec {
    SELECT_SETTINGS
        .iter()
        .find(|spec| spec.setting == setting)
        .expect("every SelectSetting has a row (see spec::tests)")
}

pub(super) fn bool_spec(setting: BoolSetting) -> &'static BoolSpec {
    BOOL_SETTINGS
        .iter()
        .find(|spec| spec.setting == setting)
        .expect("every BoolSetting has a row (see spec::tests)")
}

#[cfg(test)]
mod tests;
