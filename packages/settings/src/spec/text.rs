//! The numeric settings shown as text inputs, and the bounds each one enforces.

use std::ops::RangeBounds;

use daruda_config::{Config, SettingsPatch};
use gpui::{App, Entity, SharedString};

use crate::strings as s;
use crate::{SettingsView, TextSetting};
use daruda_ui::InputState;

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

/// The largest integer `config.toml` can hold; a `u64` field above it would
/// wrap when written as TOML's signed 64-bit integer.
const MAX_TOML_INT: u64 = i64::MAX as u64;

/// A numeric setting shown as a text input.
pub(crate) struct TextSpec {
    pub(crate) setting: TextSetting,
    /// Hint text for the empty input. A thunk rather than a `&'static str`
    /// because the wording is localized and the table is a `const`.
    pub(crate) placeholder: fn() -> String,
    pub(crate) field: fn(&SettingsView) -> &Entity<InputState>,
    /// The current value, formatted the way the input displays it. Used both to
    /// seed the widget at construction and to refresh it when the config
    /// changes underneath the window.
    pub(crate) show: fn(&Config) -> String,
    /// What `config` currently holds, as a patch. Declares the field this row
    /// owns (via [`SettingsPatch::field`]) without needing a widget to read.
    pub(crate) current: fn(&Config) -> SettingsPatch,
    /// The input's contents as a patch, or this field's error message.
    pub(crate) parse: fn(&Entity<InputState>, &App) -> Result<SettingsPatch, SharedString>,
}

pub(crate) const TEXT_SETTINGS: &[TextSpec] = &[
    TextSpec {
        setting: TextSetting::TerminalFontSize,
        placeholder: || s::settings::placeholder_example("13"),
        field: |w| &w.terminal_font_size_input,
        show: |c| c.font.terminal.size.to_string(),
        current: |c| SettingsPatch::TerminalFontSize(c.font.terminal.size),
        parse: |input, cx| {
            bounded(
                input,
                6.0..=72.0,
                || s::settings::err_font_size().into(),
                cx,
            )
            .map(SettingsPatch::TerminalFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalLineHeight,
        placeholder: || s::settings::placeholder_example("1.0"),
        field: |w| &w.terminal_line_height_input,
        show: |c| c.font.terminal.line_height.to_string(),
        current: |c| SettingsPatch::TerminalLineHeight(c.font.terminal.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings::err_spacing().into(), cx)
                .map(SettingsPatch::TerminalLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalCellWidth,
        placeholder: || s::settings::placeholder_example("1.0"),
        field: |w| &w.terminal_cell_width_input,
        show: |c| c.font.terminal.cell_width.to_string(),
        current: |c| SettingsPatch::TerminalCellWidth(c.font.terminal.cell_width),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings::err_spacing().into(), cx)
                .map(SettingsPatch::TerminalCellWidth)
        },
    },
    TextSpec {
        setting: TextSetting::EditorFontSize,
        placeholder: || s::settings::placeholder_example("13"),
        field: |w| &w.editor_font_size_input,
        show: |c| c.font.editor.size.to_string(),
        current: |c| SettingsPatch::EditorFontSize(c.font.editor.size),
        parse: |input, cx| {
            bounded(
                input,
                6.0..=72.0,
                || s::settings::err_editor_font_size().into(),
                cx,
            )
            .map(SettingsPatch::EditorFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::EditorLineHeight,
        placeholder: || s::settings::placeholder_example("1.7"),
        field: |w| &w.editor_line_height_input,
        show: |c| c.font.editor.line_height.to_string(),
        current: |c| SettingsPatch::EditorLineHeight(c.font.editor.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings::err_spacing().into(), cx)
                .map(SettingsPatch::EditorLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::AgentChatFontSize,
        placeholder: || s::settings::placeholder_example("13"),
        field: |w| &w.agent_chat_font_size_input,
        show: |c| c.font.agent_chat.size.to_string(),
        current: |c| SettingsPatch::AgentChatFontSize(c.font.agent_chat.size),
        parse: |input, cx| {
            bounded(
                input,
                6.0..=72.0,
                || s::settings::err_agent_chat_font_size().into(),
                cx,
            )
            .map(SettingsPatch::AgentChatFontSize)
        },
    },
    TextSpec {
        setting: TextSetting::AgentChatLineHeight,
        placeholder: || s::settings::placeholder_example("1.6"),
        field: |w| &w.agent_chat_line_height_input,
        show: |c| c.font.agent_chat.line_height.to_string(),
        current: |c| SettingsPatch::AgentChatLineHeight(c.font.agent_chat.line_height),
        parse: |input, cx| {
            bounded(input, 0.5..=2.0, || s::settings::err_spacing().into(), cx)
                .map(SettingsPatch::AgentChatLineHeight)
        },
    },
    TextSpec {
        setting: TextSetting::WindowOpacity,
        placeholder: || s::settings::placeholder_range("0.1", "1.0"),
        field: |w| &w.opacity_input,
        show: |c| c.window.opacity.to_string(),
        current: |c| SettingsPatch::WindowOpacity(c.window.opacity),
        parse: |input, cx| {
            bounded(input, 0.1..=1.0, || s::settings::err_opacity().into(), cx)
                .map(SettingsPatch::WindowOpacity)
        },
    },
    TextSpec {
        setting: TextSetting::ScrollbackMaxRows,
        placeholder: || s::settings::placeholder_example("10000"),
        field: |w| &w.scrollback_input,
        show: |c| c.scrollback.max_rows.to_string(),
        current: |c| SettingsPatch::ScrollbackMaxRows(c.scrollback.max_rows),
        parse: |input, cx| {
            bounded(
                input,
                1_000..=500_000,
                || s::settings::err_scrollback().into(),
                cx,
            )
            .map(SettingsPatch::ScrollbackMaxRows)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalInsetX,
        placeholder: || s::settings::placeholder_example("4"),
        field: |w| &w.inset_x_input,
        show: |c| c.font.terminal.inset_x.to_string(),
        current: |c| SettingsPatch::TerminalInsetX(c.font.terminal.inset_x),
        parse: |input, cx| {
            bounded(input, 0.0..=32.0, || s::settings::err_inset().into(), cx)
                .map(SettingsPatch::TerminalInsetX)
        },
    },
    TextSpec {
        setting: TextSetting::TerminalInsetY,
        placeholder: || s::settings::placeholder_example("2"),
        field: |w| &w.inset_y_input,
        show: |c| c.font.terminal.inset_y.to_string(),
        current: |c| SettingsPatch::TerminalInsetY(c.font.terminal.inset_y),
        parse: |input, cx| {
            bounded(input, 0.0..=32.0, || s::settings::err_inset().into(), cx)
                .map(SettingsPatch::TerminalInsetY)
        },
    },
    TextSpec {
        setting: TextSetting::ClipboardStreamingMaxBytes,
        placeholder: || s::settings::placeholder_example("10485760"),
        field: |w| &w.clipboard_streaming_input,
        show: |c| c.clipboard.streaming_max_bytes.to_string(),
        current: |c| SettingsPatch::ClipboardStreamingMaxBytes(c.clipboard.streaming_max_bytes),
        parse: |input, cx| {
            bounded(
                input,
                4_096..=67_108_864,
                || s::settings::err_clipboard().into(),
                cx,
            )
            .map(SettingsPatch::ClipboardStreamingMaxBytes)
        },
    },
    TextSpec {
        setting: TextSetting::PanelsGridColumns,
        placeholder: || s::settings::placeholder_range("1", "16"),
        field: |w| &w.panels_grid_columns_input,
        show: |c| c.panels.grid_columns.to_string(),
        current: |c| SettingsPatch::PanelsGridColumns(c.panels.grid_columns),
        parse: |input, cx| {
            bounded(input, 1..=16, || s::settings::err_grid_columns().into(), cx)
                .map(SettingsPatch::PanelsGridColumns)
        },
    },
    TextSpec {
        setting: TextSetting::ClaudeStatusStaleSecs,
        placeholder: || s::settings::placeholder_example("300"),
        field: |w| &w.claude_status_stale_input,
        show: |c| c.claude_status.stale_threshold_secs.to_string(),
        current: |c| SettingsPatch::ClaudeStatusStaleSecs(c.claude_status.stale_threshold_secs),
        parse: |input, cx| {
            bounded(
                input,
                30..=86_400,
                || s::settings::err_stale_threshold().into(),
                cx,
            )
            .map(SettingsPatch::ClaudeStatusStaleSecs)
        },
    },
    TextSpec {
        setting: TextSetting::ClaudeStatusFileTtlDays,
        placeholder: || s::settings::placeholder_example("7"),
        field: |w| &w.claude_status_ttl_input,
        show: |c| c.claude_status.file_ttl_days.to_string(),
        current: |c| SettingsPatch::ClaudeStatusFileTtlDays(c.claude_status.file_ttl_days),
        parse: |input, cx| {
            bounded(input, 1..=365, || s::settings::err_file_ttl().into(), cx)
                .map(SettingsPatch::ClaudeStatusFileTtlDays)
        },
    },
    TextSpec {
        setting: TextSetting::UsageLimitsPollSecs,
        placeholder: || s::settings::placeholder_example("300"),
        field: |w| &w.usage_limits_poll_input,
        show: |c| c.usage.poll.limits_secs.to_string(),
        current: |c| SettingsPatch::UsageLimitsPollSecs(c.usage.poll.limits_secs),
        parse: |input, cx| {
            bounded(
                input,
                0..=MAX_TOML_INT,
                || s::settings::err_whole_number().into(),
                cx,
            )
            .map(SettingsPatch::UsageLimitsPollSecs)
        },
    },
    TextSpec {
        setting: TextSetting::UsageStatusPollSecs,
        placeholder: || s::settings::placeholder_example("300"),
        field: |w| &w.usage_status_poll_input,
        show: |c| c.usage.poll.status_secs.to_string(),
        current: |c| SettingsPatch::UsageStatusPollSecs(c.usage.poll.status_secs),
        parse: |input, cx| {
            bounded(
                input,
                0..=MAX_TOML_INT,
                || s::settings::err_whole_number().into(),
                cx,
            )
            .map(SettingsPatch::UsageStatusPollSecs)
        },
    },
    TextSpec {
        setting: TextSetting::PortsPollSecs,
        placeholder: || s::settings::placeholder_example("5"),
        field: |w| &w.ports_poll_input,
        show: |c| c.ports.poll_secs.to_string(),
        current: |c| SettingsPatch::PortsPollSecs(c.ports.poll_secs),
        parse: |input, cx| {
            bounded(
                input,
                2..=MAX_TOML_INT,
                || s::settings::err_ports_poll().into(),
                cx,
            )
            .map(SettingsPatch::PortsPollSecs)
        },
    },
    TextSpec {
        setting: TextSetting::LogsRetentionDays,
        placeholder: || s::settings::placeholder_example("30"),
        field: |w| &w.logs_retention_input,
        show: |c| c.logs.retention_days.to_string(),
        current: |c| SettingsPatch::LogsRetentionDays(c.logs.retention_days),
        parse: |input, cx| {
            bounded(input, 0.., || s::settings::err_whole_number().into(), cx)
                .map(SettingsPatch::LogsRetentionDays)
        },
    },
    TextSpec {
        setting: TextSetting::LogsMaxFileSizeMb,
        placeholder: || s::settings::placeholder_example("10"),
        field: |w| &w.logs_max_size_input,
        show: |c| c.logs.max_file_size_mb.to_string(),
        current: |c| SettingsPatch::LogsMaxFileSizeMb(c.logs.max_file_size_mb),
        parse: |input, cx| {
            bounded(input, 0.., || s::settings::err_whole_number().into(), cx)
                .map(SettingsPatch::LogsMaxFileSizeMb)
        },
    },
    TextSpec {
        setting: TextSetting::PresenceGraceSecs,
        placeholder: || s::settings::placeholder_example("10"),
        field: |w| &w.presence_grace_input,
        show: |c| c.presence.away_grace_secs.to_string(),
        current: |c| SettingsPatch::PresenceGraceSecs(c.presence.away_grace_secs),
        parse: |input, cx| {
            bounded(
                input,
                0..=MAX_TOML_INT,
                || s::settings::err_whole_number().into(),
                cx,
            )
            .map(SettingsPatch::PresenceGraceSecs)
        },
    },
    TextSpec {
        setting: TextSetting::PresenceIdleSecs,
        placeholder: || s::settings::placeholder_example("30"),
        field: |w| &w.presence_idle_input,
        show: |c| c.presence.away_idle_secs.to_string(),
        current: |c| SettingsPatch::PresenceIdleSecs(c.presence.away_idle_secs),
        parse: |input, cx| {
            bounded(
                input,
                0..=MAX_TOML_INT,
                || s::settings::err_whole_number().into(),
                cx,
            )
            .map(SettingsPatch::PresenceIdleSecs)
        },
    },
    TextSpec {
        setting: TextSetting::PresenceIdleForegroundSecs,
        placeholder: || s::settings::placeholder_example("180"),
        field: |w| &w.presence_idle_foreground_input,
        show: |c| c.presence.away_idle_foreground_secs.to_string(),
        current: |c| {
            SettingsPatch::PresenceIdleForegroundSecs(c.presence.away_idle_foreground_secs)
        },
        parse: |input, cx| {
            bounded(
                input,
                0..=MAX_TOML_INT,
                || s::settings::err_whole_number().into(),
                cx,
            )
            .map(SettingsPatch::PresenceIdleForegroundSecs)
        },
    },
    TextSpec {
        setting: TextSetting::AgentInputMaxRows,
        placeholder: || s::settings::placeholder_example("8"),
        field: |w| &w.agent_input_max_rows_input,
        show: |c| c.agent.input_max_rows.to_string(),
        current: |c| SettingsPatch::AgentInputMaxRows(c.agent.input_max_rows),
        parse: |input, cx| {
            bounded(
                input,
                2..=20,
                || s::settings::err_input_max_rows().into(),
                cx,
            )
            .map(SettingsPatch::AgentInputMaxRows)
        },
    },
    TextSpec {
        setting: TextSetting::AgentReadingWidth,
        placeholder: || s::settings::placeholder_example("700"),
        field: |w| &w.agent_reading_width_input,
        show: |c| c.agent.reading_width.to_string(),
        current: |c| SettingsPatch::AgentReadingWidth(c.agent.reading_width),
        parse: |input, cx| {
            bounded(
                input,
                360.0..=2400.0,
                || s::settings::err_reading_width().into(),
                cx,
            )
            .map(SettingsPatch::AgentReadingWidth)
        },
    },
    TextSpec {
        setting: TextSetting::FlowTimeoutMinutes,
        placeholder: || s::settings::placeholder_example("90"),
        field: |w| &w.flow_timeout_minutes_input,
        show: |c| c.flow.timeout_minutes.to_string(),
        current: |c| SettingsPatch::FlowTimeoutMinutes(c.flow.timeout_minutes),
        parse: |input, cx| {
            bounded(input, 0.., || s::settings::err_whole_number().into(), cx)
                .map(SettingsPatch::FlowTimeoutMinutes)
        },
    },
    TextSpec {
        setting: TextSetting::FlowMaxNodeRuns,
        placeholder: || s::settings::placeholder_example("100"),
        field: |w| &w.flow_max_node_runs_input,
        show: |c| c.flow.max_node_runs.to_string(),
        current: |c| SettingsPatch::FlowMaxNodeRuns(c.flow.max_node_runs),
        parse: |input, cx| {
            bounded(input, 0.., || s::settings::err_whole_number().into(), cx)
                .map(SettingsPatch::FlowMaxNodeRuns)
        },
    },
    TextSpec {
        setting: TextSetting::FlowMaxCost,
        placeholder: || s::settings::placeholder_example("5.0"),
        field: |w| &w.flow_max_cost_input,
        show: |c| c.flow.max_cost.to_string(),
        current: |c| SettingsPatch::FlowMaxCost(c.flow.max_cost),
        parse: |input, cx| {
            bounded(input, 0.0.., || s::settings::err_non_negative().into(), cx)
                .map(SettingsPatch::FlowMaxCost)
        },
    },
    TextSpec {
        setting: TextSetting::FlowCostCurrency,
        placeholder: || s::settings::placeholder_example("USD"),
        field: |w| &w.flow_cost_currency_input,
        show: |c| c.flow.cost_currency.clone(),
        current: |c| SettingsPatch::FlowCostCurrency(c.flow.cost_currency.clone()),
        // Free text: the currency is whatever the agent reports its cost in.
        parse: |input, cx| {
            let value = input.read(cx).value().trim().to_string();
            if value.is_empty() {
                Err(s::settings::err_currency().into())
            } else {
                Ok(SettingsPatch::FlowCostCurrency(value))
            }
        },
    },
    TextSpec {
        setting: TextSetting::LeftDefaultWidth,
        placeholder: || s::settings::placeholder_example("220"),
        field: |w| &w.left_default_width_input,
        show: |c| c.left_dock.left_default_width.to_string(),
        current: |c| SettingsPatch::LeftDefaultWidth(c.left_dock.left_default_width),
        parse: |input, cx| {
            bounded(
                input,
                150.0..=400.0,
                || s::settings::err_left_default_width().into(),
                cx,
            )
            .map(SettingsPatch::LeftDefaultWidth)
        },
    },
    TextSpec {
        setting: TextSetting::ShellProgram,
        placeholder: s::settings::placeholder_shell_program,
        field: |w| &w.shell_program_input,
        show: |c| c.shell.program.clone().unwrap_or_default(),
        current: |c| SettingsPatch::ShellProgram(c.shell.program.clone()),
        // An empty field means "no override": the login shell.
        parse: |input, cx| {
            let value = input.read(cx).value().trim().to_string();
            Ok(SettingsPatch::ShellProgram(
                (!value.is_empty()).then_some(value),
            ))
        },
    },
    TextSpec {
        setting: TextSetting::NotifyLongRunningThresholdSecs,
        placeholder: || s::settings::placeholder_example("30"),
        field: |w| &w.notify_long_running_threshold_input,
        show: |c| c.notifications.long_running_threshold_secs.to_string(),
        current: |c| {
            SettingsPatch::NotifyLongRunningThresholdSecs(
                c.notifications.long_running_threshold_secs,
            )
        },
        parse: |input, cx| {
            bounded(
                input,
                1..=86_400,
                || s::settings::err_long_running_threshold().into(),
                cx,
            )
            .map(SettingsPatch::NotifyLongRunningThresholdSecs)
        },
    },
    TextSpec {
        setting: TextSetting::KeepAwakeApprovalWaitMinutes,
        placeholder: || s::settings::placeholder_example("30"),
        field: |w| &w.keep_awake_approval_wait_input,
        show: |c| c.desktop.keep_awake_approval_wait_minutes.to_string(),
        current: |c| {
            SettingsPatch::KeepAwakeApprovalWaitMinutes(c.desktop.keep_awake_approval_wait_minutes)
        },
        parse: |input, cx| {
            bounded(
                input,
                1..=daruda_config::desktop::MAX_APPROVAL_WAIT_MINUTES,
                || s::settings::err_keep_awake_wait().into(),
                cx,
            )
            .map(SettingsPatch::KeepAwakeApprovalWaitMinutes)
        },
    },
];
