//! Rewrites the one key (or structural catalog) a Settings field patch
//! addresses in the on-disk document.

use super::catalog_tables::{replace_agents, replace_session_hosts, status_bar_item_slug};
use super::sections::{font_key, patch_nested_section, patch_section};
use crate::{Config, CursorStyle, SettingsPatch, remote};

pub(super) fn patch_settings_document(
    doc: &mut toml_edit::DocumentMut,
    config: &Config,
    patch: &SettingsPatch,
) {
    match patch {
        SettingsPatch::RemoteChannels(_) => remote::patch_document(doc, &config.remote),
        SettingsPatch::GeneralLanguage(_) => patch_section(doc, "general", |t| {
            t.insert(
                "language",
                toml_edit::value(config.general.language.clone()),
            );
        }),
        SettingsPatch::TerminalPreset(_) => patch_section(doc, "theme", |t| {
            t.insert(
                "terminal_preset",
                toml_edit::value(config.theme.terminal_preset.clone()),
            );
        }),
        SettingsPatch::UiPreset(_) => patch_section(doc, "theme", |t| {
            t.insert(
                "ui_preset",
                toml_edit::value(config.theme.ui_preset.clone()),
            );
        }),
        // One key each: rewriting the whole `[font]` tree would pin every
        // other font field, including one just reset, to its current value.
        SettingsPatch::TerminalFontFamily(_) => font_key(
            doc,
            "terminal",
            "family",
            config.font.terminal.family.clone(),
        ),
        SettingsPatch::TerminalFontSize(_) => font_key(
            doc,
            "terminal",
            "size",
            f64::from(config.font.terminal.size),
        ),
        SettingsPatch::TerminalLineHeight(_) => font_key(
            doc,
            "terminal",
            "line_height",
            f64::from(config.font.terminal.line_height),
        ),
        SettingsPatch::TerminalCellWidth(_) => font_key(
            doc,
            "terminal",
            "cell_width",
            f64::from(config.font.terminal.cell_width),
        ),
        SettingsPatch::TerminalInsetX(_) => font_key(
            doc,
            "terminal",
            "inset_x",
            f64::from(config.font.terminal.inset_x),
        ),
        SettingsPatch::TerminalInsetY(_) => font_key(
            doc,
            "terminal",
            "inset_y",
            f64::from(config.font.terminal.inset_y),
        ),
        SettingsPatch::EditorFontFamily(_) => {
            font_key(doc, "editor", "family", config.font.editor.family.clone())
        }
        SettingsPatch::EditorFontSize(_) => {
            font_key(doc, "editor", "size", f64::from(config.font.editor.size))
        }
        SettingsPatch::EditorLineHeight(_) => font_key(
            doc,
            "editor",
            "line_height",
            f64::from(config.font.editor.line_height),
        ),
        SettingsPatch::AgentChatFontFamily(_) => font_key(
            doc,
            "agent_chat",
            "family",
            config.font.agent_chat.family.clone(),
        ),
        SettingsPatch::AgentChatFontSize(_) => font_key(
            doc,
            "agent_chat",
            "size",
            f64::from(config.font.agent_chat.size),
        ),
        SettingsPatch::AgentChatLineHeight(_) => font_key(
            doc,
            "agent_chat",
            "line_height",
            f64::from(config.font.agent_chat.line_height),
        ),
        SettingsPatch::CursorStyle(_) => patch_section(doc, "cursor", |t| {
            let value = match config.cursor.style {
                CursorStyle::Block => "block",
                CursorStyle::Underline => "underline",
                CursorStyle::Bar => "bar",
            };
            t.insert("style", toml_edit::value(value));
        }),
        SettingsPatch::AgentUseModifierToSend(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "use_modifier_to_send",
                toml_edit::value(config.agent.use_modifier_to_send),
            );
        }),
        SettingsPatch::AgentUseReadingWidth(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "use_reading_width",
                toml_edit::value(config.agent.use_reading_width),
            );
        }),
        SettingsPatch::AgentToolSummaryLabels(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "tool_summary_labels",
                toml_edit::value(config.agent.tool_summary_labels),
            );
        }),
        SettingsPatch::AgentCatalog(_) => replace_agents(doc, &config.agents),
        SettingsPatch::SessionHosts { .. } => {
            replace_session_hosts(doc, &config.session_hosts, &config.session_host_tombstones)
        }
        SettingsPatch::RenderMaxFps(_) => patch_section(doc, "render", |t| {
            t.insert(
                "max_fps",
                toml_edit::value(i64::from(config.render.max_fps)),
            );
        }),
        SettingsPatch::ShellClosePaneOnExit(_) => patch_section(doc, "shell", |t| {
            t.insert(
                "close_pane_on_exit",
                toml_edit::value(config.shell.close_pane_on_exit),
            );
        }),
        SettingsPatch::WindowOpacity(_) => patch_section(doc, "window", |t| {
            t.insert(
                "opacity",
                toml_edit::value(f64::from(config.window.opacity)),
            );
        }),
        SettingsPatch::WindowBlur(_) => patch_section(doc, "window", |t| {
            t.insert("blur", toml_edit::value(config.window.blur));
        }),
        SettingsPatch::ScrollbackMaxRows(_) => patch_section(doc, "scrollback", |t| {
            t.insert(
                "max_rows",
                toml_edit::value(config.scrollback.max_rows as i64),
            );
        }),
        SettingsPatch::FilesShowHidden(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "files_show_hidden",
                toml_edit::value(config.left_dock.files_show_hidden),
            );
        }),
        SettingsPatch::FilesUseGitignore(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "files_use_gitignore",
                toml_edit::value(config.left_dock.files_use_gitignore),
            );
        }),
        SettingsPatch::UpdateAutoCheck(_) => patch_section(doc, "update", |t| {
            t.insert("auto_check", toml_edit::value(config.update.auto_check));
        }),
        SettingsPatch::ClaudeStatusStaleSecs(_) => patch_section(doc, "claude_status", |t| {
            t.insert(
                "stale_threshold_secs",
                toml_edit::value(config.claude_status.stale_threshold_secs as i64),
            );
        }),
        SettingsPatch::ClaudeStatusFileTtlDays(_) => patch_section(doc, "claude_status", |t| {
            t.insert(
                "file_ttl_days",
                toml_edit::value(config.claude_status.file_ttl_days as i64),
            );
        }),
        SettingsPatch::UsageLimitsPollSecs(_) => {
            patch_nested_section(doc, &["usage", "poll"], |t| {
                t.insert(
                    "limits_secs",
                    toml_edit::value(config.usage.poll.limits_secs as i64),
                );
            })
        }
        SettingsPatch::UsageStatusPollSecs(_) => {
            patch_nested_section(doc, &["usage", "poll"], |t| {
                t.insert(
                    "status_secs",
                    toml_edit::value(config.usage.poll.status_secs as i64),
                );
            })
        }
        SettingsPatch::UsageStopOnOverage(_) => patch_section(doc, "usage", |t| {
            t.insert(
                "stop_on_overage",
                toml_edit::value(config.usage.stop_on_overage),
            );
        }),
        SettingsPatch::PortsPollSecs(_) => patch_section(doc, "ports", |t| {
            t.insert("poll_secs", toml_edit::value(config.ports.poll_secs as i64));
        }),
        SettingsPatch::LogsRetentionDays(_) => patch_section(doc, "logs", |t| {
            t.insert(
                "retention_days",
                toml_edit::value(config.logs.retention_days as i64),
            );
        }),
        SettingsPatch::LogsMaxFileSizeMb(_) => patch_section(doc, "logs", |t| {
            t.insert(
                "max_file_size_mb",
                toml_edit::value(config.logs.max_file_size_mb as i64),
            );
        }),
        SettingsPatch::PresenceGraceSecs(_) => patch_section(doc, "presence", |t| {
            t.insert(
                "away_grace_secs",
                toml_edit::value(config.presence.away_grace_secs as i64),
            );
        }),
        SettingsPatch::PresenceIdleSecs(_) => patch_section(doc, "presence", |t| {
            t.insert(
                "away_idle_secs",
                toml_edit::value(config.presence.away_idle_secs as i64),
            );
        }),
        SettingsPatch::PresenceIdleForegroundSecs(_) => patch_section(doc, "presence", |t| {
            t.insert(
                "away_idle_foreground_secs",
                toml_edit::value(config.presence.away_idle_foreground_secs as i64),
            );
        }),
        SettingsPatch::KeepAwake(_) => patch_section(doc, "desktop", |t| {
            t.insert(
                "keep_awake",
                toml_edit::value(config.desktop.keep_awake.as_str()),
            );
        }),
        SettingsPatch::KeepAwakeApprovalWaitMinutes(_) => patch_section(doc, "desktop", |t| {
            t.insert(
                "keep_awake_approval_wait_minutes",
                toml_edit::value(config.desktop.keep_awake_approval_wait_minutes as i64),
            );
        }),
        SettingsPatch::AgentInputMaxRows(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "input_max_rows",
                toml_edit::value(config.agent.input_max_rows as i64),
            );
        }),
        SettingsPatch::AgentReadingWidth(_) => patch_section(doc, "agent", |t| {
            t.insert(
                "reading_width",
                toml_edit::value(f64::from(config.agent.reading_width)),
            );
        }),
        SettingsPatch::FlowTimeoutMinutes(_) => patch_section(doc, "flow", |t| {
            t.insert(
                "timeout_minutes",
                toml_edit::value(config.flow.timeout_minutes as i64),
            );
        }),
        SettingsPatch::FlowMaxNodeRuns(_) => patch_section(doc, "flow", |t| {
            t.insert(
                "max_node_runs",
                toml_edit::value(config.flow.max_node_runs as i64),
            );
        }),
        SettingsPatch::FlowMaxCost(_) => patch_section(doc, "flow", |t| {
            t.insert("max_cost", toml_edit::value(config.flow.max_cost));
        }),
        SettingsPatch::FlowCostCurrency(_) => patch_section(doc, "flow", |t| {
            t.insert(
                "cost_currency",
                toml_edit::value(config.flow.cost_currency.clone()),
            );
        }),
        SettingsPatch::LeftCollapsedByDefault(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "left_collapsed_by_default",
                toml_edit::value(config.left_dock.left_collapsed_by_default),
            );
        }),
        SettingsPatch::PreviewTab(_) => patch_section(doc, "file_viewer", |t| {
            t.insert(
                "preview_tab",
                toml_edit::value(config.file_viewer.preview_tab),
            );
        }),
        SettingsPatch::LeftDefaultWidth(_) => patch_section(doc, "left_dock", |t| {
            t.insert(
                "left_default_width",
                toml_edit::value(f64::from(config.left_dock.left_default_width)),
            );
        }),
        SettingsPatch::ShellNaturalTextEditing(_) => patch_section(doc, "shell", |t| {
            t.insert(
                "natural_text_editing",
                toml_edit::value(config.shell.natural_text_editing),
            );
        }),
        SettingsPatch::ShellProgram(_) => patch_section(doc, "shell", |t| {
            match &config.shell.program {
                Some(program) => t.insert("program", toml_edit::value(program.clone())),
                None => t.remove("program"),
            };
        }),
        SettingsPatch::NotifyOsc9(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "osc9_enabled",
                toml_edit::value(config.notifications.osc9_enabled),
            );
        }),
        SettingsPatch::NotifyOsc777(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "osc777_enabled",
                toml_edit::value(config.notifications.osc777_enabled),
            );
        }),
        SettingsPatch::NotifyAttention(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "attention_enabled",
                toml_edit::value(config.notifications.attention_enabled),
            );
        }),
        SettingsPatch::NotifyLongRunning(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "long_running_enabled",
                toml_edit::value(config.notifications.long_running_enabled),
            );
        }),
        SettingsPatch::ClipboardCopyOnSelect(_) => patch_section(doc, "clipboard", |t| {
            t.insert(
                "copy_on_select",
                toml_edit::value(config.clipboard.copy_on_select),
            );
        }),
        SettingsPatch::GitConfirmCommit(_) => patch_section(doc, "git", |t| {
            t.insert(
                "confirm_commit",
                toml_edit::value(config.git.confirm_commit),
            );
        }),
        SettingsPatch::GitConfirmPush(_) => patch_section(doc, "git", |t| {
            t.insert("confirm_push", toml_edit::value(config.git.confirm_push));
        }),
        SettingsPatch::GitDefaultCommitMessage(_) => patch_section(doc, "git", |t| {
            t.insert(
                "default_commit_message",
                toml_edit::value(config.git.default_commit_message),
            );
        }),
        SettingsPatch::NotifySkipFocusedPane(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "skip_focused_pane",
                toml_edit::value(config.notifications.skip_focused_pane),
            );
        }),
        SettingsPatch::NotifyHook(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "hook_notification_enabled",
                toml_edit::value(config.notifications.hook_notification_enabled),
            );
        }),
        SettingsPatch::NotifyAgentCompletion(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "agent_completion_enabled",
                toml_edit::value(config.notifications.agent_completion_enabled),
            );
        }),
        SettingsPatch::NotifyAgentWaiting(_) => patch_section(doc, "notifications", |t| {
            t.insert(
                "agent_waiting_enabled",
                toml_edit::value(config.notifications.agent_waiting_enabled),
            );
        }),
        SettingsPatch::TelegramOnlyWhenAway(_) => patch_section(doc, "telegram", |t| {
            t.insert(
                "only_when_away",
                toml_edit::value(config.telegram.only_when_away),
            );
        }),
        SettingsPatch::NotifyLongRunningThresholdSecs(_) => {
            patch_section(doc, "notifications", |t| {
                t.insert(
                    "long_running_threshold_secs",
                    toml_edit::value(config.notifications.long_running_threshold_secs as i64),
                );
            })
        }
        SettingsPatch::SyntaxTheme(_) => patch_section(doc, "file_viewer", |t| {
            t.insert(
                "syntax_theme",
                toml_edit::value(config.file_viewer.syntax_theme.clone()),
            );
        }),
        SettingsPatch::ClipboardStreamingMaxBytes(_) => patch_section(doc, "clipboard", |t| {
            t.insert(
                "streaming_max_bytes",
                toml_edit::value(config.clipboard.streaming_max_bytes as i64),
            );
        }),
        SettingsPatch::PreferredEditor(_) => patch_section(doc, "editor", |t| {
            t.insert(
                "preferred",
                toml_edit::value(config.editor.preferred.clone()),
            );
        }),
        SettingsPatch::PanelsGridColumns(_) => patch_section(doc, "panels", |t| {
            t.insert(
                "grid_columns",
                toml_edit::value(i64::from(config.panels.grid_columns)),
            );
        }),
        SettingsPatch::ToggleStatusBarItem(_) | SettingsPatch::StatusBarHiddenItems(_) => {
            patch_section(doc, "status_bar", |t| {
                let mut items = toml_edit::Array::new();
                for item in &config.status_bar.hidden_items {
                    items.push(status_bar_item_slug(*item));
                }
                t.insert("hidden_items", toml_edit::value(items));
                t.remove("visible_items");
            })
        }
        SettingsPatch::FileIconColorMode(_) => patch_section(doc, "left_dock", |t| {
            let mode = match config.left_dock.file_icon_color_mode {
                crate::IconColorMode::Color => "color",
                crate::IconColorMode::Monochrome => "monochrome",
            };
            t.insert("file_icon_color_mode", toml_edit::value(mode));
        }),
        SettingsPatch::ClaudeStatusEnabled(_) => patch_section(doc, "claude_status", |t| {
            t.insert("enable", toml_edit::value(config.claude_status.enable));
        }),
        SettingsPatch::TelegramEnabled(_) => patch_section(doc, "telegram", |t| {
            t.insert("enabled", toml_edit::value(config.telegram.enabled));
        }),
        SettingsPatch::TelegramAuthorizedChatId(_) => patch_section(doc, "telegram", |t| {
            if let Some(id) = config.telegram.authorized_chat_id {
                t.insert("authorized_chat_id", toml_edit::value(id));
            } else {
                t.remove("authorized_chat_id");
            }
        }),
        SettingsPatch::OrchestratorEnabled(_) => patch_section(doc, "orchestrator", |t| {
            t.insert("enabled", toml_edit::value(config.orchestrator.enabled));
        }),
        // Each `None` removes its key rather than writing a null: absent is
        // what the deserializer reads as "follow the default", and a written
        // empty string would be a real agent id that resolves to nothing.
        SettingsPatch::OrchestratorAgentId(_) => patch_section(doc, "orchestrator", |t| {
            match &config.orchestrator.agent_id {
                Some(id) => t.insert("agent_id", toml_edit::value(id.clone())),
                None => t.remove("agent_id"),
            };
        }),
        SettingsPatch::OrchestratorAccountId(_) => patch_section(doc, "orchestrator", |t| {
            match config.orchestrator.account_id {
                Some(id) => t.insert("account_id", toml_edit::value(id.0.to_string())),
                None => t.remove("account_id"),
            };
        }),
    }
}
