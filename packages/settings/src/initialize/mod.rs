//! Construction and observation of a per-window settings view.

use super::*;
use daruda_config::Config;
mod render_selects;
use render_selects::RenderSelects;

mod behavior_selects;
use behavior_selects::BehaviorSelects;

mod workspace_selects;
use workspace_selects::WorkspaceSelects;

mod appearance_selects;
use appearance_selects::AppearanceSelects;

impl SettingsView {
    pub fn new_with_section(
        active: BuiltinSection,
        shared: SharedSnapshot,
        services: std::rc::Rc<dyn SettingsServices>,
        remote_integrations: gpui::AnyView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let SharedSnapshot {
            accounts,
            account_login_busy,
            auth_statuses,
            agent_vocabulary,
            plugin_skills,
            status_icon_themes,
            update_state,
        } = shared;
        // Defensive idempotent init for test fixtures that open a
        // Settings window without going through `globals::init_all`.
        crate::store::SettingsStore::init(cx);
        let config = crate::store::SettingsStore::global(cx).user().clone();

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

        let AppearanceSelects {
            language_select,
            terminal_preset_select,
            ui_preset_select,
            terminal_font_family_select,
            editor_font_family_select,
            agent_chat_font_family_select,
        } = AppearanceSelects::new(&config, window, cx);

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

        let [
            terminal_font_size_input,
            terminal_line_height_input,
            terminal_cell_width_input,
            editor_font_size_input,
            editor_line_height_input,
            agent_chat_font_size_input,
            agent_chat_line_height_input,
            opacity_input,
            scrollback_input,
            inset_x_input,
            inset_y_input,
            clipboard_streaming_input,
        ] = Self::new_text_inputs(
            [
                TextSetting::TerminalFontSize,
                TextSetting::TerminalLineHeight,
                TextSetting::TerminalCellWidth,
                TextSetting::EditorFontSize,
                TextSetting::EditorLineHeight,
                TextSetting::AgentChatFontSize,
                TextSetting::AgentChatLineHeight,
                TextSetting::WindowOpacity,
                TextSetting::ScrollbackMaxRows,
                TextSetting::TerminalInsetX,
                TextSetting::TerminalInsetY,
                TextSetting::ClipboardStreamingMaxBytes,
            ],
            &config,
            window,
            cx,
            &mut input_subscriptions,
        );
        let WorkspaceSelects {
            editor_select,
            left_dock_size_select,
            status_icon_theme_select,
            file_icon_color_select,
        } = WorkspaceSelects::new(&config, &status_icon_themes, window, cx);

        let [
            panels_grid_columns_input,
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
            left_default_width_input,
            shell_program_input,
            notify_long_running_threshold_input,
            keep_awake_approval_wait_input,
        ] = Self::new_text_inputs(
            [
                TextSetting::PanelsGridColumns,
                TextSetting::ClaudeStatusStaleSecs,
                TextSetting::ClaudeStatusFileTtlDays,
                TextSetting::UsageLimitsPollSecs,
                TextSetting::UsageStatusPollSecs,
                TextSetting::PortsPollSecs,
                TextSetting::LogsRetentionDays,
                TextSetting::LogsMaxFileSizeMb,
                TextSetting::PresenceGraceSecs,
                TextSetting::PresenceIdleSecs,
                TextSetting::PresenceIdleForegroundSecs,
                TextSetting::AgentInputMaxRows,
                TextSetting::AgentReadingWidth,
                TextSetting::FlowTimeoutMinutes,
                TextSetting::FlowMaxNodeRuns,
                TextSetting::FlowMaxCost,
                TextSetting::FlowCostCurrency,
                TextSetting::LeftDefaultWidth,
                TextSetting::ShellProgram,
                TextSetting::NotifyLongRunningThresholdSecs,
                TextSetting::KeepAwakeApprovalWaitMinutes,
            ],
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
        let telegram_token_configured = services.telegram_token_stored();

        let BehaviorSelects {
            keep_awake_mode_select,
            cursor_style_select,
        } = BehaviorSelects::new(&config, window, cx);

        let agent_catalog_search = cx.new(|cx_state| {
            InputState::new(window, cx_state)
                .placeholder(s::settings::agent_catalog_search_placeholder())
        });

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

        let RenderSelects {
            max_fps_select,
            syntax_theme_select,
        } = RenderSelects::new(&config, window, cx);

        for (state, setting) in [
            (&language_select, SelectSetting::Language),
            (&left_dock_size_select, SelectSetting::LeftDockSize),
            (&status_icon_theme_select, SelectSetting::StatusIconTheme),
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
            (&keep_awake_mode_select, SelectSetting::KeepAwakeMode),
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
            keep_awake_mode_select,
            keep_awake_approval_wait_input,
            agent_catalog_search,
            agent_use_modifier_to_send: config.agent.use_modifier_to_send,
            agent_use_reading_width: config.agent.use_reading_width,
            agent_tool_summary_labels: config.agent.tool_summary_labels,
            agent_catalog,
            agent_vocabulary,
            session_host_rows,
            accounts,
            orchestrator_enabled: config.orchestrator.enabled,
            orchestrator_agent_select,
            orchestrator_account_select,
            account_login_busy,
            auth_statuses,
            services,
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
            stop_on_overage: config.usage.stop_on_overage,
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
            left_dock_size_select,
            status_icon_theme_select,
            status_icon_themes,
            file_icon_color_select,
            panels_grid_columns_input,
            claude_status_enable: config.claude_status.enable,
            telegram_enabled: config.telegram.enabled,
            remote_integrations,
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
            plugin_skills,
            _settings_global_subscription: cx.observe_global_in::<crate::store::SettingsStore>(
                window,
                |this, window, cx| {
                    this.adopt_external_settings(window, cx);
                },
            ),
            update_state,
        }
    }

    fn new_text_inputs<const N: usize>(
        settings: [TextSetting; N],
        config: &Config,
        window: &mut Window,
        cx: &mut Context<Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> [Entity<InputState>; N] {
        settings.map(|setting| Self::new_text_field(setting, config, window, cx, subscriptions))
    }
}
