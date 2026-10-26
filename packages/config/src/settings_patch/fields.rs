//! Ordinary Settings field mappings shared by snapshot and TOML patching.

macro_rules! with_settings_fields {
    ($callback:ident) => {
        $callback! {
            GeneralLanguage: general.language, "general.language", clone, |value: String| value;
            TerminalPreset: theme.terminal_preset, "theme.terminal_preset", clone, |value: String| value;
            UiPreset: theme.ui_preset, "theme.ui_preset", clone, |value: String| value;
            TerminalFontFamily: font.terminal.family, "font.terminal.family", clone, |value: String| value;
            TerminalFontSize: font.terminal.size, "font.terminal.size", copy, |value: f32| f64::from(value);
            TerminalLineHeight: font.terminal.line_height, "font.terminal.line_height", copy, |value: f32| f64::from(value);
            TerminalCellWidth: font.terminal.cell_width, "font.terminal.cell_width", copy, |value: f32| f64::from(value);
            EditorFontFamily: font.editor.family, "font.editor.family", clone, |value: String| value;
            EditorFontSize: font.editor.size, "font.editor.size", copy, |value: f32| f64::from(value);
            EditorLineHeight: font.editor.line_height, "font.editor.line_height", copy, |value: f32| f64::from(value);
            AgentChatFontFamily: font.agent_chat.family, "font.agent_chat.family", clone, |value: String| value;
            AgentChatFontSize: font.agent_chat.size, "font.agent_chat.size", copy, |value: f32| f64::from(value);
            AgentChatLineHeight: font.agent_chat.line_height, "font.agent_chat.line_height", copy, |value: f32| f64::from(value);
            AgentUseModifierToSend: agent.use_modifier_to_send, "agent.use_modifier_to_send", copy, |value: bool| value;
            AgentUseReadingWidth: agent.use_reading_width, "agent.use_reading_width", copy, |value: bool| value;
            AgentToolSummaryLabels: agent.tool_summary_labels, "agent.tool_summary_labels", copy, |value: bool| value;
            RenderMaxFps: render.max_fps, "render.max_fps", copy, |value: u32| i64::from(value);
            ShellClosePaneOnExit: shell.close_pane_on_exit, "shell.close_pane_on_exit", copy, |value: bool| value;
            WindowOpacity: window.opacity, "window.opacity", copy, |value: f32| f64::from(value);
            WindowBlur: window.blur, "window.blur", copy, |value: bool| value;
            ScrollbackMaxRows: scrollback.max_rows, "scrollback.max_rows", copy, |value: usize| value as i64;
            TerminalInsetX: font.terminal.inset_x, "font.terminal.inset_x", copy, |value: f32| f64::from(value);
            TerminalInsetY: font.terminal.inset_y, "font.terminal.inset_y", copy, |value: f32| f64::from(value);
            FilesShowHidden: left_dock.files_show_hidden, "left_dock.files_show_hidden", copy, |value: bool| value;
            FilesUseGitignore: left_dock.files_use_gitignore, "left_dock.files_use_gitignore", copy, |value: bool| value;
            UpdateAutoCheck: update.auto_check, "update.auto_check", copy, |value: bool| value;
            ClaudeStatusStaleSecs: claude_status.stale_threshold_secs, "claude_status.stale_threshold_secs", copy, |value: u64| value as i64;
            ClaudeStatusFileTtlDays: claude_status.file_ttl_days, "claude_status.file_ttl_days", copy, |value: u32| value as i64;
            UsageLimitsPollSecs: usage.poll.limits_secs, "usage.poll.limits_secs", copy, |value: u64| value as i64;
            UsageStatusPollSecs: usage.poll.status_secs, "usage.poll.status_secs", copy, |value: u64| value as i64;
            UsageStopOnOverage: usage.stop_on_overage, "usage.stop_on_overage", copy, |value: bool| value;
            PortsPollSecs: ports.poll_secs, "ports.poll_secs", copy, |value: u64| value as i64;
            LogsRetentionDays: logs.retention_days, "logs.retention_days", copy, |value: u32| value as i64;
            LogsMaxFileSizeMb: logs.max_file_size_mb, "logs.max_file_size_mb", copy, |value: u32| value as i64;
            PresenceGraceSecs: presence.away_grace_secs, "presence.away_grace_secs", copy, |value: u64| value as i64;
            PresenceIdleSecs: presence.away_idle_secs, "presence.away_idle_secs", copy, |value: u64| value as i64;
            PresenceIdleForegroundSecs: presence.away_idle_foreground_secs, "presence.away_idle_foreground_secs", copy, |value: u64| value as i64;
            KeepAwakeApprovalWaitMinutes: desktop.keep_awake_approval_wait_minutes, "desktop.keep_awake_approval_wait_minutes", copy, |value: u64| value as i64;
            AgentInputMaxRows: agent.input_max_rows, "agent.input_max_rows", copy, |value: u8| value as i64;
            AgentReadingWidth: agent.reading_width, "agent.reading_width", copy, |value: f32| f64::from(value);
            FlowTimeoutMinutes: flow.timeout_minutes, "flow.timeout_minutes", copy, |value: u32| value as i64;
            FlowMaxNodeRuns: flow.max_node_runs, "flow.max_node_runs", copy, |value: u32| value as i64;
            FlowMaxCost: flow.max_cost, "flow.max_cost", copy, |value: f64| value;
            FlowCostCurrency: flow.cost_currency, "flow.cost_currency", clone, |value: String| value;
            LeftCollapsedByDefault: left_dock.left_collapsed_by_default, "left_dock.left_collapsed_by_default", copy, |value: bool| value;
            PreviewTab: file_viewer.preview_tab, "file_viewer.preview_tab", copy, |value: bool| value;
            LeftDefaultWidth: left_dock.left_default_width, "left_dock.left_default_width", copy, |value: f32| f64::from(value);
            ShellNaturalTextEditing: shell.natural_text_editing, "shell.natural_text_editing", copy, |value: bool| value;
            NotifyOsc9: notifications.osc9_enabled, "notifications.osc9_enabled", copy, |value: bool| value;
            NotifyOsc777: notifications.osc777_enabled, "notifications.osc777_enabled", copy, |value: bool| value;
            NotifyAttention: notifications.attention_enabled, "notifications.attention_enabled", copy, |value: bool| value;
            NotifyLongRunning: notifications.long_running_enabled, "notifications.long_running_enabled", copy, |value: bool| value;
            NotifySkipFocusedPane: notifications.skip_focused_pane, "notifications.skip_focused_pane", copy, |value: bool| value;
            ClipboardCopyOnSelect: clipboard.copy_on_select, "clipboard.copy_on_select", copy, |value: bool| value;
            GitConfirmCommit: git.confirm_commit, "git.confirm_commit", copy, |value: bool| value;
            GitConfirmPush: git.confirm_push, "git.confirm_push", copy, |value: bool| value;
            GitDefaultCommitMessage: git.default_commit_message, "git.default_commit_message", copy, |value: bool| value;
            NotifyHook: notifications.hook_notification_enabled, "notifications.hook_notification_enabled", copy, |value: bool| value;
            NotifyAgentCompletion: notifications.agent_completion_enabled, "notifications.agent_completion_enabled", copy, |value: bool| value;
            NotifyAgentWaiting: notifications.agent_waiting_enabled, "notifications.agent_waiting_enabled", copy, |value: bool| value;
            TelegramOnlyWhenAway: telegram.only_when_away, "telegram.only_when_away", copy, |value: bool| value;
            NotifyLongRunningThresholdSecs: notifications.long_running_threshold_secs, "notifications.long_running_threshold_secs", copy, |value: u64| value as i64;
            SyntaxTheme: file_viewer.syntax_theme, "file_viewer.syntax_theme", clone, |value: String| value;
            ClipboardStreamingMaxBytes: clipboard.streaming_max_bytes, "clipboard.streaming_max_bytes", copy, |value: usize| value as i64;
            PreferredEditor: editor.preferred, "editor.preferred", clone, |value: String| value;
            PanelsGridColumns: panels.grid_columns, "panels.grid_columns", copy, |value: u8| i64::from(value);
            ClaudeStatusEnabled: claude_status.enable, "claude_status.enable", copy, |value: bool| value;
            TelegramEnabled: telegram.enabled, "telegram.enabled", copy, |value: bool| value;
            OrchestratorEnabled: orchestrator.enabled, "orchestrator.enabled", copy, |value: bool| value;
        }
    };
}

macro_rules! field_value {
    (copy, $value:expr) => {
        *$value
    };
    (clone, $value:expr) => {
        $value.clone()
    };
}

pub(crate) use {field_value, with_settings_fields};

#[cfg(test)]
mod tests;
