//! Construction and observation of a per-window settings view.

use super::*;

impl SettingsView {
    pub fn new_with_section(
        active: BuiltinSection,
        data_dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Defensive idempotent init for test fixtures that open a
        // Settings window without going through `globals::init_all`.
        crate::settings_store::SettingsStore::init(cx);
        let config = crate::settings_store::SettingsStore::global(cx)
            .user()
            .clone();

        let sidebar_search_input = cx.new(|cx_state| {
            InputState::new(window, cx_state).placeholder(s::settings::search_placeholder())
        });
        let sidebar_focus_handles = BuiltinSection::ALL
            .iter()
            .copied()
            .map(|section| (section, cx.focus_handle().tab_stop(true)))
            .collect();
        let advanced_focus_handles = BuiltinSection::ALL
            .iter()
            .copied()
            .map(|section| (section, cx.focus_handle().tab_stop(true)))
            .collect();

        // Language select — options driven by the canonical locale list so
        // adding a new locale only requires updating SUPPORTED_LOCALES.
        let lang = SharedString::from(config.general.language.clone());
        let language_select = cx.new(|cx| {
            let opts: Vec<select::SelectOption> = daruda_config::SUPPORTED_LOCALES
                .iter()
                .map(|&slug| {
                    let label = match slug {
                        "auto" => s::settings::language_auto(),
                        "en" => s::settings::language_en(),
                        "ko" => s::settings::language_ko(),
                        other => other.to_owned(),
                    };
                    select::SelectOption::new(slug, label)
                })
                .collect();
            select::state_with_options(opts, Some(&lang), window, cx)
        });

        // Terminal preset select — cell palette (fg/bg + ANSI 16).
        let terminal_preset = SharedString::from(config.theme.terminal_preset.clone());
        let terminal_preset_select = cx.new(|cx| {
            let opts = daruda_config::THEME_PRESETS
                .iter()
                .map(|p| SelectOption::new(p.name, p.display_name))
                .collect();
            select::state_with_options(opts, Some(&terminal_preset), window, cx)
        });

        // UI preset select — chrome palette (workspace, modal, status bar, …).
        let ui_preset = SharedString::from(config.theme.ui_preset.clone());
        let ui_preset_select = cx.new(|cx| {
            let system = SelectOption::new(
                daruda_config::ui_theme_presets::SYSTEM,
                s::settings::ui_preset_system(),
            );
            let opts = std::iter::once(system)
                .chain(
                    daruda_config::UI_THEME_PRESETS
                        .iter()
                        .map(|p| SelectOption::new(p.name, p.display_name)),
                )
                .collect();
            select::state_with_options(opts, Some(&ui_preset), window, cx)
        });

        let terminal_font_options = font_select_options(cx, &[&config.font.terminal.family]);
        let editor_font_options = font_select_options(cx, &[&config.font.editor.family]);
        let agent_chat_font_options = font_select_options(cx, &[&config.font.agent_chat.family]);
        let terminal_font_family = SharedString::from(config.font.terminal.family.clone());
        let terminal_font_family_select = cx.new(|cx| {
            select::state_with_options(
                terminal_font_options,
                Some(&terminal_font_family),
                window,
                cx,
            )
        });
        let editor_font_family = SharedString::from(config.font.editor.family.clone());
        let editor_font_family_select = cx.new(|cx| {
            select::state_with_options(editor_font_options, Some(&editor_font_family), window, cx)
        });
        let agent_chat_font_family = SharedString::from(config.font.agent_chat.family.clone());
        let agent_chat_font_family_select = cx.new(|cx| {
            select::state_with_options(
                agent_chat_font_options,
                Some(&agent_chat_font_family),
                window,
                cx,
            )
        });

        // Collected as each field below is built, instead of assembled in one
        // block after the fact — keeps "construct → subscribe → focus handle
        // → section jump target" together per field. `section_focus_targets`
        // is the single source for both `focus_section` (first handle) and
        // `focus_next_input` (full per-section cycle order).
        let mut input_subscriptions: Vec<Subscription> = Vec::new();
        input_subscriptions.push(Self::subscribe_sidebar_search(
            &sidebar_search_input,
            window,
            cx,
        ));
        let mut section_focus_targets: HashMap<BuiltinSection, Vec<FocusHandle>> = HashMap::new();

        let terminal_font_size_input = Self::new_text_field(
            TextSetting::TerminalFontSize,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let terminal_line_height_input = Self::new_text_field(
            TextSetting::TerminalLineHeight,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let terminal_cell_width_input = Self::new_text_field(
            TextSetting::TerminalCellWidth,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let editor_font_size_input = Self::new_text_field(
            TextSetting::EditorFontSize,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let editor_line_height_input = Self::new_text_field(
            TextSetting::EditorLineHeight,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let agent_chat_font_size_input = Self::new_text_field(
            TextSetting::AgentChatFontSize,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let agent_chat_line_height_input = Self::new_text_field(
            TextSetting::AgentChatLineHeight,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let opacity_input = Self::new_text_field(
            TextSetting::WindowOpacity,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let scrollback_input = Self::new_text_field(
            TextSetting::ScrollbackMaxRows,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let inset_x_input = Self::new_text_field(
            TextSetting::TerminalInsetX,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let inset_y_input = Self::new_text_field(
            TextSetting::TerminalInsetY,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let clipboard_streaming_input = Self::new_text_field(
            TextSetting::ClipboardStreamingMaxBytes,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        // External editor select — "" (empty, the config default) means the
        // OS default handler; every other value is a `daruda_config::editor`
        // preset name.
        let preferred_editor = SharedString::from(config.editor.preferred.clone());
        let editor_select = cx.new(|cx| {
            let mut opts = vec![select::SelectOption::new(
                "",
                s::settings::editor_system_default(),
            )];
            opts.extend(
                daruda_config::EXTERNAL_EDITOR_PRESETS
                    .iter()
                    .map(|p| select::SelectOption::new(p.name, p.display_name)),
            );
            select::state_with_options(opts, Some(&preferred_editor), window, cx)
        });
        let file_icon_color_select = cx.new(|cx| {
            let opts = vec![
                select::SelectOption::new(ICON_COLOR, s::settings::icon_color_color()),
                select::SelectOption::new(ICON_MONOCHROME, s::settings::icon_color_monochrome()),
            ];
            let current =
                SharedString::new_static(icon_color_value(&config.left_dock.file_icon_color_mode));
            select::state_with_options(opts, Some(&current), window, cx)
        });
        // Second (and last) text input on the merged Dock page — after
        // the Sidebar subsection's checkboxes (no text input) and before
        // the Bottom Dock subsection's own fields, so it's simply
        // appended to the same section's tab-cycle list.
        let panels_grid_columns_input = Self::new_text_field(
            TextSetting::PanelsGridColumns,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let claude_status_stale_input = Self::new_text_field(
            TextSetting::ClaudeStatusStaleSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let claude_status_ttl_input = Self::new_text_field(
            TextSetting::ClaudeStatusFileTtlDays,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let usage_limits_poll_input = Self::new_text_field(
            TextSetting::UsageLimitsPollSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let usage_status_poll_input = Self::new_text_field(
            TextSetting::UsageStatusPollSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let ports_poll_input = Self::new_text_field(
            TextSetting::PortsPollSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let logs_retention_input = Self::new_text_field(
            TextSetting::LogsRetentionDays,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let logs_max_size_input = Self::new_text_field(
            TextSetting::LogsMaxFileSizeMb,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let presence_grace_input = Self::new_text_field(
            TextSetting::PresenceGraceSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let presence_idle_input = Self::new_text_field(
            TextSetting::PresenceIdleSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let presence_idle_foreground_input = Self::new_text_field(
            TextSetting::PresenceIdleForegroundSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let agent_input_max_rows_input = Self::new_text_field(
            TextSetting::AgentInputMaxRows,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let agent_reading_width_input = Self::new_text_field(
            TextSetting::AgentReadingWidth,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let flow_timeout_minutes_input = Self::new_text_field(
            TextSetting::FlowTimeoutMinutes,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let flow_max_node_runs_input = Self::new_text_field(
            TextSetting::FlowMaxNodeRuns,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let flow_max_cost_input = Self::new_text_field(
            TextSetting::FlowMaxCost,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let flow_cost_currency_input = Self::new_text_field(
            TextSetting::FlowCostCurrency,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let left_default_width_input = Self::new_text_field(
            TextSetting::LeftDefaultWidth,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let shell_program_input = Self::new_text_field(
            TextSetting::ShellProgram,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let notify_long_running_threshold_input = Self::new_text_field(
            TextSetting::NotifyLongRunningThresholdSecs,
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        // Never pre-filled with the real token (`default_value`) — a stored
        // secret is never re-displayed in a text field, so this field can't
        // go through `new_text_field` (which always sets a default value).
        // The "Token configured" status line covers presence instead.
        let telegram_token_input = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::settings::telegram_token_placeholder())
                .masked(true)
        });
        input_subscriptions.push(Self::subscribe_draft_input(
            &telegram_token_input,
            window,
            cx,
        ));
        section_focus_targets
            .entry(BuiltinSection::RemoteControl)
            .or_default()
            .push(telegram_token_input.read(cx).focus_handle(cx));
        let telegram_token_configured = crate::telegram::keychain::read_token().is_some();
        let remote_channel_settings =
            cx.new(|cx| crate::remote_channel::settings::ChannelSettings::new(window, cx));

        let cursor_style_str: SharedString = match config.cursor.style {
            daruda_config::CursorStyle::Block => "block".into(),
            daruda_config::CursorStyle::Underline => "underline".into(),
            daruda_config::CursorStyle::Bar => "bar".into(),
        };
        let cursor_style_select = cx.new(|cx| {
            select::state_with_options(
                vec![
                    SelectOption::new("block", s::settings::cursor_block()),
                    SelectOption::new("underline", s::settings::cursor_underline()),
                    SelectOption::new("bar", s::settings::cursor_bar()),
                ],
                Some(&cursor_style_str),
                window,
                cx,
            )
        });

        let agent_catalog_search = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::settings::agent_catalog_search_placeholder())
        });

        // Vocabulary is shared app-wide so every open window sees a
        // connection's advertisement; it is read from the host's store.
        crate::workspace::agent_vocabulary_global::install_path(cx, &data_dir);
        let agent_vocabulary = crate::workspace::agent_vocabulary_global::snapshot(cx, &data_dir);

        // Entries that resolve get an editable row; entries that don't (a preset
        // id daruda no longer knows, or one that needs a manual install) have no
        // fields to edit and are kept verbatim — a config the editor cannot
        // represent must not be a config the editor deletes. Both kinds stay in
        // one list at their config position.
        let agent_catalog: Vec<AgentCatalogItem> = config
            .agents
            .iter()
            .map(|entry| Self::agent_catalog_item(entry, &agent_vocabulary, window, cx))
            .collect();

        let session_host_rows: Vec<SessionHostRow> = config
            .session_hosts
            .iter()
            .map(|entry| Self::session_host_row_from_entry(entry, window, cx))
            .collect();

        let max_fps_str: SharedString = config.render.max_fps.to_string().into();
        let max_fps_select = cx.new(|cx| {
            let opts = daruda_config::ALLOWED_MAX_FPS
                .iter()
                .map(|fps| {
                    SelectOption::new(
                        SharedString::from(fps.to_string()),
                        s::settings::max_fps_option(*fps),
                    )
                })
                .collect();
            select::state_with_options(opts, Some(&max_fps_str), window, cx)
        });

        // Resolve through the palette so a legacy / unknown stored value
        // (e.g. an old "base16-ocean.dark") still shows its effective
        // palette selected instead of a blank dropdown.
        let syntax_theme = SharedString::from(
            crate::ui::theme::SyntaxPalette::from_config_name(&config.file_viewer.syntax_theme)
                .config_name(),
        );
        let syntax_theme_select = cx.new(|cx| {
            let opts = SYNTAX_THEMES
                .iter()
                .map(|v| SelectOption::new(*v, syntax_theme_label(v)))
                .collect();
            select::state_with_options(opts, Some(&syntax_theme), window, cx)
        });

        for (state, setting) in [
            (&language_select, SelectSetting::Language),
            (&file_icon_color_select, SelectSetting::FileIconColorMode),
            (&terminal_preset_select, SelectSetting::TerminalPreset),
            (&ui_preset_select, SelectSetting::UiPreset),
            (
                &terminal_font_family_select,
                SelectSetting::TerminalFontFamily,
            ),
            (&editor_font_family_select, SelectSetting::EditorFontFamily),
            (
                &agent_chat_font_family_select,
                SelectSetting::AgentChatFontFamily,
            ),
            (&cursor_style_select, SelectSetting::CursorStyle),
            (&max_fps_select, SelectSetting::RenderMaxFps),
            (&syntax_theme_select, SelectSetting::SyntaxTheme),
            (&editor_select, SelectSetting::PreferredEditor),
        ] {
            input_subscriptions.push(Self::subscribe_select_setting(state, setting, window, cx));
        }
        // The query narrows the preset lists as it is typed.
        input_subscriptions.push(cx.subscribe_in(
            &agent_catalog_search,
            window,
            |_this, _state, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            },
        ));
        let editable_rows = || {
            agent_catalog.iter().filter_map(|item| match item {
                AgentCatalogItem::Editable(row) => Some(row),
                AgentCatalogItem::Unresolved(_) => None,
            })
        };
        for row in editable_rows() {
            Self::subscribe_agent_row(row, window, cx, &mut input_subscriptions);
        }

        // First-field jump target for the Agent section (its full tab-cycle
        // list is dynamic — see `focus_next_input` — since rows are
        // added/removed at runtime).
        if let Some(row) = editable_rows().next() {
            section_focus_targets
                .entry(BuiltinSection::Agent)
                .or_default()
                .push(row.id_input.read(cx).focus_handle(cx));
        }

        for row in &session_host_rows {
            Self::subscribe_session_host_row(row, window, cx, &mut input_subscriptions);
        }
        // First-field jump target for the Session Hosts section — mirrors
        // the Agent section above. An empty catalog is a valid state (see
        // `daruda_config::Config::session_hosts`), so there may be nothing
        // to jump to; `focus_section` already falls back to the panel focus
        // handle when the map has no entry.
        if let Some(row) = session_host_rows.first() {
            section_focus_targets
                .entry(BuiltinSection::SessionHosts)
                .or_default()
                .push(row.label_input.read(cx).focus_handle(cx));
        }

        let _updater_subscription =
            crate::update::Updater::get(cx).map(|e| cx.observe(&e, |_, _, cx| cx.notify()));

        // Managed accounts come from the app-wide Global (single source of
        // truth). Install it from disk if this window opened before any
        // Workspace (idempotent), then mirror the current value — refreshed
        // on change by `_accounts_global_subscription`.
        crate::workspace::accounts_global::install_if_absent(
            cx,
            daruda_store::accounts::load_accounts().unwrap_or_default(),
        );
        let accounts = crate::workspace::accounts_global::snapshot(cx);
        let account_login_busy = crate::workspace::accounts_global::login_busy(cx);
        // Built here rather than beside the other selects above: the account
        // picker's options come from `accounts`, which is only in scope now.
        let orchestrator_agent_select = cx.new(|cx| {
            select::state_with_options(
                sections::orchestrator::agent_options(&config),
                Some(&sections::orchestrator::agent_select_value(&config)),
                window,
                cx,
            )
        });
        let orchestrator_account_select = cx.new(|cx| {
            select::state_with_options(
                sections::orchestrator::account_options(&accounts.accounts),
                Some(&sections::orchestrator::account_select_value(&config)),
                window,
                cx,
            )
        });
        for (state, setting) in [
            (&orchestrator_agent_select, SelectSetting::OrchestratorAgent),
            (
                &orchestrator_account_select,
                SelectSetting::OrchestratorAccount,
            ),
        ] {
            input_subscriptions.push(Self::subscribe_select_setting(state, setting, window, cx));
        }
        // Rebuilding the account picker needs the current window.
        let _accounts_global_subscription = cx
            .observe_global_in::<crate::workspace::accounts_global::AccountsGlobal>(
                window,
                |this, window, cx| {
                    this.accounts = crate::workspace::accounts_global::snapshot(cx);
                    this.account_login_busy = crate::workspace::accounts_global::login_busy(cx);
                    this.refresh_orchestrator_account_select(window, cx);
                    cx.notify();
                },
            );
        let _agent_vocabulary_global_subscription = cx.observe_global_in::<
            crate::workspace::agent_vocabulary_global::AgentVocabularyGlobal,
        >(window, |this, window, cx| {
            this.agent_vocabulary =
                crate::workspace::agent_vocabulary_global::snapshot(cx, &this.data_dir);
            for index in 0..this.agent_catalog.len() {
                this.refresh_agent_row_vocabulary(index, window, cx);
            }
        });
        // Same mirror shape for the sign-in readings: a Workspace produces
        // them off-thread, so they land after this window is already open.
        crate::workspace::auth_status_global::install_if_absent(cx);
        let auth_statuses = crate::workspace::auth_status_global::snapshot(cx);
        let _auth_status_subscription = cx
            .observe_global::<crate::workspace::auth_status_global::AuthStatusGlobal>(
                |this, cx| {
                    this.auth_statuses = crate::workspace::auth_status_global::snapshot(cx);
                    cx.notify();
                },
            );

        Self {
            panel_focus_handle: cx.focus_handle(),
            base_config: config.clone(),
            active_section: active,
            sidebar_search_input,
            sidebar_focus_handles,
            advanced_focus_handles,
            project_open: true,
            section_focus_targets,
            language_select,
            terminal_preset_select,
            ui_preset_select,
            terminal_font_family_select,
            terminal_font_size_input,
            terminal_line_height_input,
            terminal_cell_width_input,
            editor_font_family_select,
            editor_font_size_input,
            editor_line_height_input,
            agent_chat_font_family_select,
            agent_chat_font_size_input,
            agent_chat_line_height_input,
            cursor_style_select,
            agent_catalog_search,
            agent_use_modifier_to_send: config.agent.use_modifier_to_send,
            agent_use_reading_width: config.agent.use_reading_width,
            agent_tool_summary_labels: config.agent.tool_summary_labels,
            agent_catalog,
            agent_vocabulary,
            data_dir,
            session_host_rows,
            accounts,
            orchestrator_enabled: config.orchestrator.enabled,
            orchestrator_agent_select,
            orchestrator_account_select,
            account_login_busy,
            auth_statuses,
            max_fps_select,
            close_pane_on_exit: config.shell.close_pane_on_exit,
            opacity_input,
            window_blur: config.window.blur,
            scrollback_input,
            inset_x_input,
            inset_y_input,
            files_show_hidden: config.left_dock.files_show_hidden,
            files_use_gitignore: config.left_dock.files_use_gitignore,
            update_auto_check: config.update.auto_check,
            claude_status_stale_input,
            claude_status_ttl_input,
            usage_limits_poll_input,
            usage_status_poll_input,
            ports_poll_input,
            logs_retention_input,
            logs_max_size_input,
            presence_grace_input,
            presence_idle_input,
            presence_idle_foreground_input,
            agent_input_max_rows_input,
            agent_reading_width_input,
            flow_timeout_minutes_input,
            flow_max_node_runs_input,
            flow_max_cost_input,
            flow_cost_currency_input,
            left_collapsed_by_default: config.left_dock.left_collapsed_by_default,
            preview_tab: config.file_viewer.preview_tab,
            left_default_width_input,
            shell_program_input,
            advanced_open: Default::default(),
            shell_natural_text_editing: config.shell.natural_text_editing,
            notify_osc9: config.notifications.osc9_enabled,
            notify_osc777: config.notifications.osc777_enabled,
            notify_attention: config.notifications.attention_enabled,
            notify_long_running: config.notifications.long_running_enabled,
            notify_skip_focused_pane: config.notifications.skip_focused_pane,
            clipboard_copy_on_select: config.clipboard.copy_on_select,
            git_confirm_commit: config.git.confirm_commit,
            git_confirm_push: config.git.confirm_push,
            git_default_commit_message: config.git.default_commit_message,
            notify_hook: config.notifications.hook_notification_enabled,
            notify_agent_completion: config.notifications.agent_completion_enabled,
            notify_agent_waiting: config.notifications.agent_waiting_enabled,
            telegram_only_when_away: config.telegram.only_when_away,
            notify_long_running_threshold_input,
            syntax_theme_select,
            clipboard_streaming_input,
            editor_select,
            file_icon_color_select,
            panels_grid_columns_input,
            claude_status_enable: config.claude_status.enable,
            telegram_enabled: config.telegram.enabled,
            remote_channel_settings,
            telegram_token_input,
            telegram_token_configured,
            telegram_authorized_chat_id: config.telegram.authorized_chat_id,
            telegram_pair_code: None,
            telegram_pair_command_copy: CopyFeedback::default(),
            telegram_botfather_copy: CopyFeedback::default(),
            scroll_handle: gpui::ScrollHandle::new(),
            sidebar_scroll_handle: gpui::ScrollHandle::new(),
            _input_subscriptions: input_subscriptions,
            error: None,
            conflict: None,
            plugin_ops_in_flight: std::collections::HashSet::new(),
            plugin_installs: sections::plugin::read_plugin_installs_indexed(),
            plugin_selected: None,
            plugin_view_skill: None,
            _skills_global_subscription: cx.observe_global::<crate::agent::skills::SkillsState>(
                |this, cx| {
                    this.plugin_installs = sections::plugin::read_plugin_installs_indexed();
                    cx.notify();
                },
            ),
            _settings_global_subscription: cx
                .observe_global_in::<crate::settings_store::SettingsStore>(
                    window,
                    |this, window, cx| {
                        this.adopt_external_settings(window, cx);
                    },
                ),
            _agent_vocabulary_global_subscription,
            _accounts_global_subscription,
            _auth_status_subscription,
            _updater_subscription,
        }
    }
}
