//! What each simple setting *is*, as data: which widget drives it, how it
//! reads out of a [`Config`], and how it turns back into a [`SettingsPatch`].
//!
//! Every one of these facts used to be spread across six coordinated `match`
//! arms and literal lists in `mod.rs` — `persist_*_setting` (live edit),
//! `validate` (whole-config draft), `load_settings_patch` (external change
//! lands), `settings_ui_patches` (which fields the window owns), the
//! constructor, and the section render. Two of those six held the *same* facts
//! twice: a text setting's bounds and error message were written once for the
//! live edit and again for the draft, and the cursor-style string mapping was
//! written out in both directions in two places. Nothing kept either pair in
//! agreement.
//!
//! The tables below are now that single place. A new setting adds one row here
//! plus its widget and its render call; the four generic consumers pick it up
//! without a new arm. What is deliberately *not* here: settings whose load or
//! collect step is bespoke (agent catalog, session hosts, the Telegram token),
//! which stay hand-written in `mod.rs`.

use std::ops::RangeBounds;

use daruda_config::{BuiltinSection, Config, SettingsPatch};
use gpui::{App, Entity, SharedString};

use super::{BoolSetting, SelectSetting, SettingsView, TextSetting};
use crate::surface::strings as s;
use crate::ui::InputState;
use crate::ui::select::SelectState;

/// Read `input`, trim it, parse it, and require `range` — or fail with the
/// field's own message. The one place a text setting's bounds are enforced.
fn bounded<T: std::str::FromStr + PartialOrd>(
    input: &Entity<InputState>,
    range: impl RangeBounds<T>,
    err: impl FnOnce() -> SharedString,
    cx: &App,
) -> Result<T, SharedString> {
    input
        .read(cx)
        .value()
        .trim()
        .parse::<T>()
        .ok()
        .filter(|v| range.contains(v))
        .ok_or_else(err)
}

/// A numeric setting shown as a text input.
pub(super) struct TextSpec {
    pub(super) setting: TextSetting,
    /// Which settings page the input sits on — also its tab-cycle bucket.
    pub(super) section: BuiltinSection,
    /// Hint text for the empty input. A thunk rather than a `&'static str`
    /// because the wording is localized and the table is a `const`.
    pub(super) placeholder: fn() -> String,
    pub(super) field: fn(&SettingsView) -> &Entity<InputState>,
    /// The current value, formatted the way the input displays it. Used both to
    /// seed the widget at construction and to refresh it when the config
    /// changes underneath the window.
    pub(super) show: fn(&Config) -> String,
    /// What `config` currently holds, as a patch. Declares the field this row
    /// owns (via [`SettingsPatch::field`]) without needing a widget to read.
    pub(super) current: fn(&Config) -> SettingsPatch,
    /// The input's contents as a patch, or this field's error message.
    pub(super) parse: fn(&Entity<InputState>, &App) -> Result<SettingsPatch, SharedString>,
}

pub(super) const TEXT_SETTINGS: &[TextSpec] = &[
    TextSpec {
        setting: TextSetting::TerminalFontSize,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("13"),
        field: |w| &w.terminal_font_size_input,
        show: |c| c.font.terminal.size.to_string(),
        current: |c| SettingsPatch::TerminalFontSize(c.font.terminal.size),
        parse: |input, cx| {
            bounded(input, 6.0..=72.0, || s::settings_err_font_size().into(), cx)
                .map(SettingsPatch::TerminalFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalLineHeight,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("1.0"),
        field: |w| &w.terminal_line_height_input,
        show: |c| c.font.terminal.line_height.to_string(),
        current: |c| SettingsPatch::TerminalLineHeight(c.font.terminal.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings_err_spacing().into(), cx)
                .map(SettingsPatch::TerminalLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalCellWidth,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("1.0"),
        field: |w| &w.terminal_cell_width_input,
        show: |c| c.font.terminal.cell_width.to_string(),
        current: |c| SettingsPatch::TerminalCellWidth(c.font.terminal.cell_width),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings_err_spacing().into(), cx)
                .map(SettingsPatch::TerminalCellWidth)
        },
    },
    TextSpec {
        setting: TextSetting::EditorFontSize,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("13"),
        field: |w| &w.editor_font_size_input,
        show: |c| c.font.editor.size.to_string(),
        current: |c| SettingsPatch::EditorFontSize(c.font.editor.size),
        parse: |input, cx| {
            bounded(
                input,
                6.0..=72.0,
                || s::settings_err_editor_font_size().into(),
                cx,
            )
            .map(SettingsPatch::EditorFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::EditorLineHeight,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("1.7"),
        field: |w| &w.editor_line_height_input,
        show: |c| c.font.editor.line_height.to_string(),
        current: |c| SettingsPatch::EditorLineHeight(c.font.editor.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings_err_spacing().into(), cx)
                .map(SettingsPatch::EditorLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::AgentChatFontSize,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("13"),
        field: |w| &w.agent_chat_font_size_input,
        show: |c| c.font.agent_chat.size.to_string(),
        current: |c| SettingsPatch::AgentChatFontSize(c.font.agent_chat.size),
        parse: |input, cx| {
            bounded(
                input,
                6.0..=72.0,
                || s::settings_err_agent_chat_font_size().into(),
                cx,
            )
            .map(SettingsPatch::AgentChatFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::AgentChatLineHeight,
        section: BuiltinSection::Font,
        placeholder: || s::settings_placeholder_example("1.6"),
        field: |w| &w.agent_chat_line_height_input,
        show: |c| c.font.agent_chat.line_height.to_string(),
        current: |c| SettingsPatch::AgentChatLineHeight(c.font.agent_chat.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings_err_spacing().into(), cx)
                .map(SettingsPatch::AgentChatLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::WindowOpacity,
        section: BuiltinSection::Window,
        placeholder: || s::settings_placeholder_range("0.1", "1.0"),
        field: |w| &w.opacity_input,
        show: |c| c.window.opacity.to_string(),
        current: |c| SettingsPatch::WindowOpacity(c.window.opacity),
        parse: |input, cx| {
            bounded(input, 0.1..=1.0, || s::settings_err_opacity().into(), cx)
                .map(SettingsPatch::WindowOpacity)
        },
    },
    TextSpec {
        setting: TextSetting::ScrollbackMaxRows,
        section: BuiltinSection::Terminal,
        placeholder: || s::settings_placeholder_example("10000"),
        field: |w| &w.scrollback_input,
        show: |c| c.scrollback.max_rows.to_string(),
        current: |c| SettingsPatch::ScrollbackMaxRows(c.scrollback.max_rows),
        parse: |input, cx| {
            bounded(
                input,
                1_000..=500_000,
                || s::settings_err_scrollback().into(),
                cx,
            )
            .map(SettingsPatch::ScrollbackMaxRows)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalInsetX,
        section: BuiltinSection::Terminal,
        placeholder: || s::settings_placeholder_example("4"),
        field: |w| &w.inset_x_input,
        show: |c| c.font.terminal.inset_x.to_string(),
        current: |c| SettingsPatch::TerminalInsetX(c.font.terminal.inset_x),
        parse: |input, cx| {
            bounded(input, 0.0..=32.0, || s::settings_err_inset().into(), cx)
                .map(SettingsPatch::TerminalInsetX)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalInsetY,
        section: BuiltinSection::Terminal,
        placeholder: || s::settings_placeholder_example("2"),
        field: |w| &w.inset_y_input,
        show: |c| c.font.terminal.inset_y.to_string(),
        current: |c| SettingsPatch::TerminalInsetY(c.font.terminal.inset_y),
        parse: |input, cx| {
            bounded(input, 0.0..=32.0, || s::settings_err_inset().into(), cx)
                .map(SettingsPatch::TerminalInsetY)
        },
    },
    TextSpec {
        setting: TextSetting::ClipboardStreamingMaxBytes,
        section: BuiltinSection::Clipboard,
        placeholder: || s::settings_placeholder_example("10485760"),
        field: |w| &w.clipboard_streaming_input,
        show: |c| c.clipboard.streaming_max_bytes.to_string(),
        current: |c| SettingsPatch::ClipboardStreamingMaxBytes(c.clipboard.streaming_max_bytes),
        parse: |input, cx| {
            bounded(
                input,
                4_096..=67_108_864,
                || s::settings_err_clipboard().into(),
                cx,
            )
            .map(SettingsPatch::ClipboardStreamingMaxBytes)
        },
    },
    TextSpec {
        setting: TextSetting::PanelsGridColumns,
        section: BuiltinSection::Dock,
        placeholder: || s::settings_placeholder_range("1", "16"),
        field: |w| &w.panels_grid_columns_input,
        show: |c| c.panels.grid_columns.to_string(),
        current: |c| SettingsPatch::PanelsGridColumns(c.panels.grid_columns),
        parse: |input, cx| {
            bounded(input, 1..=16, || s::settings_err_grid_columns().into(), cx)
                .map(SettingsPatch::PanelsGridColumns)
        },
    },
];

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
mod tests {
    use super::*;

    /// Every row's placeholder has to reach a real locale key. `t!` answers a
    /// miss with the key path itself, which renders as `settings.foo` in the
    /// field — visible only while the input is empty, so nothing else catches
    /// it. The number is asserted too: a typo'd `%{value}` drops it silently.
    #[test]
    fn every_text_row_placeholder_resolves() {
        for row in TEXT_SETTINGS {
            let text = (row.placeholder)();
            assert!(
                !text.starts_with("settings."),
                "{:?} placeholder fell through to its key: {text}",
                row.setting
            );
            assert!(
                text.chars().any(|c| c.is_ascii_digit()),
                "{:?} placeholder lost its sample value: {text}",
                row.setting
            );
        }
    }

    /// The `expect` in each lookup is only safe while every variant has exactly
    /// one row. `ALL` is the enum's own list, so adding a variant without a row
    /// fails here rather than at the first click on the new widget.
    #[test]
    fn every_setting_has_exactly_one_row() {
        for setting in TextSetting::ALL {
            let n = TEXT_SETTINGS
                .iter()
                .filter(|s| s.setting == setting)
                .count();
            assert_eq!(n, 1, "{setting:?} has {n} rows in TEXT_SETTINGS");
        }
        assert_eq!(TEXT_SETTINGS.len(), TextSetting::ALL.len());

        for setting in SelectSetting::ALL {
            let n = SELECT_SETTINGS
                .iter()
                .filter(|s| s.setting == setting)
                .count();
            assert_eq!(n, 1, "{setting:?} has {n} rows in SELECT_SETTINGS");
        }
        assert_eq!(SELECT_SETTINGS.len(), SelectSetting::ALL.len());

        for setting in BoolSetting::ALL {
            let n = BOOL_SETTINGS
                .iter()
                .filter(|s| s.setting == setting)
                .count();
            assert_eq!(n, 1, "{setting:?} has {n} rows in BOOL_SETTINGS");
        }
        assert_eq!(BOOL_SETTINGS.len(), BoolSetting::ALL.len());
    }

    /// `show` and `read` are the two directions of one mapping, and a select's
    /// value round-trips through them. A row where they disagree would silently
    /// reset the user's pick the next time the config changed underneath.
    ///
    /// Compared through `Debug` because `SettingsPatch` carries no `PartialEq`;
    /// that is enough to catch a row naming the wrong variant or reading a
    /// different config field than it writes.
    #[test]
    fn a_select_value_round_trips_through_show_and_read() {
        let config = Config::default();
        for spec in SELECT_SETTINGS {
            let shown = (spec.show)(&config);
            let Some(read_back) = (spec.read)(&shown) else {
                panic!("{:?}: show() produced a value read() rejects", spec.setting);
            };
            assert_eq!(
                format!("{read_back:?}"),
                format!("{:?}", (spec.current)(&config)),
                "{:?}: show() -> read() does not land on current()",
                spec.setting
            );
        }
    }

    /// How `load_settings_patch` reaches a given `SettingsPatch` variant.
    enum Coverage {
        /// A `spec` row owns it.
        Row,
        /// `load_settings_patch` handles it by hand (a whole sub-form).
        ByHand,
        /// This window does not show it, so it has nothing to catch up with.
        NotShown,
    }

    /// `load_settings_patch` used to be an exhaustive `match`, so the compiler
    /// refused a new `SettingsPatch` variant with no handler. Dispatching by row
    /// gives that up. This match restores it: it is exhaustive, so a new variant
    /// does not compile until someone classifies it here — and the assertions
    /// below then check the classification is true of the actual tables.
    fn coverage_of(patch: &SettingsPatch) -> Coverage {
        match patch {
            SettingsPatch::GeneralLanguage(_)
            | SettingsPatch::TerminalPreset(_)
            | SettingsPatch::UiPreset(_)
            | SettingsPatch::TerminalFontFamily(_)
            | SettingsPatch::TerminalFontSize(_)
            | SettingsPatch::TerminalLineHeight(_)
            | SettingsPatch::TerminalCellWidth(_)
            | SettingsPatch::EditorFontFamily(_)
            | SettingsPatch::EditorFontSize(_)
            | SettingsPatch::EditorLineHeight(_)
            | SettingsPatch::AgentChatFontFamily(_)
            | SettingsPatch::AgentChatFontSize(_)
            | SettingsPatch::AgentChatLineHeight(_)
            | SettingsPatch::CursorStyle(_)
            | SettingsPatch::AgentUseModifierToSend(_)
            | SettingsPatch::AgentUseReadingWidth(_)
            | SettingsPatch::RenderMaxFps(_)
            | SettingsPatch::ShellClosePaneOnExit(_)
            | SettingsPatch::WindowOpacity(_)
            | SettingsPatch::WindowBlur(_)
            | SettingsPatch::ScrollbackMaxRows(_)
            | SettingsPatch::TerminalInsetX(_)
            | SettingsPatch::TerminalInsetY(_)
            | SettingsPatch::FilesShowHidden(_)
            | SettingsPatch::FilesUseGitignore(_)
            | SettingsPatch::SyntaxTheme(_)
            | SettingsPatch::ClipboardStreamingMaxBytes(_)
            | SettingsPatch::PreferredEditor(_)
            | SettingsPatch::PanelsGridColumns(_)
            | SettingsPatch::ClaudeStatusEnabled(_)
            | SettingsPatch::TelegramEnabled(_)
            | SettingsPatch::OrchestratorEnabled(_)
            | SettingsPatch::OrchestratorAgentId(_)
            | SettingsPatch::OrchestratorAccountId(_) => Coverage::Row,
            SettingsPatch::AgentCatalog(_)
            | SettingsPatch::SessionHosts { .. }
            | SettingsPatch::RemoteChannels(_) => Coverage::ByHand,
            // Neither is a *field* of this window: the status-bar item list is
            // toggled from the bar itself, and the Telegram chat id is owned by
            // pairing. The Notifications section does write the chat id (Unpair)
            // and render it, but from `telegram_authorized_chat_id`, which
            // `adopt_external_settings` mirrors — not from this reload table.
            SettingsPatch::ToggleStatusBarItem(_) | SettingsPatch::TelegramAuthorizedChatId(_) => {
                Coverage::NotShown
            }
        }
    }

    /// Every variant classified `Row` above must actually have one, and no
    /// other classification may. Without this the match is just a second list
    /// that can drift from the tables it describes.
    #[test]
    fn every_reloadable_field_is_classified_the_way_the_tables_are_built() {
        let config = Config::default();
        let by_row: Vec<daruda_config::SettingsFieldId> = TEXT_SETTINGS
            .iter()
            .map(text_field_id)
            .chain(SELECT_SETTINGS.iter().map(select_field_id))
            .chain(BOOL_SETTINGS.iter().map(bool_field_id))
            .collect();

        for patch in SettingsView::settings_ui_patches(&config) {
            let field = patch.field();
            match coverage_of(&patch) {
                Coverage::Row => assert!(
                    by_row.contains(&field),
                    "{field:?} is classified Row but no spec table owns it",
                ),
                Coverage::ByHand => assert!(
                    !by_row.contains(&field),
                    "{field:?} is classified ByHand but a spec row owns it too",
                ),
                Coverage::NotShown => panic!(
                    "{field:?} is declared owned by settings_ui_patches but classified NotShown",
                ),
            }
        }
    }

    /// The same round trip for checkboxes: a mirrored field written back as a
    /// patch has to address the config slot `show` reads.
    #[test]
    fn a_bool_value_round_trips_through_show_and_patch() {
        let mut config = Config::default();
        for spec in BOOL_SETTINGS {
            let flipped = !(spec.show)(&config);
            (spec.patch)(flipped).apply_to(&mut config);
            assert_eq!(
                (spec.show)(&config),
                flipped,
                "{:?}: patch() and show() address different config fields",
                spec.setting
            );
        }
    }
}
