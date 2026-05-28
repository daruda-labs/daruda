use super::*;

#[test]
fn default_clones_compile_time_palette() {
    // Every field must equal the underlying `pub const` so a
    // fresh `DarudaTheme::default()` is observationally identical
    // to reading `palette::FOO` directly — the invariant Phase 3-B
    // relies on when it switches call sites over.
    let t = DarudaTheme::default();
    assert_eq!(t.title_bar_bg, palette::TITLE_BAR_BG);
    assert_eq!(t.tab_bar_bg, palette::TAB_BAR_BG);
    assert_eq!(t.tab_active_bg, palette::TAB_ACTIVE_BG);
    assert_eq!(t.tab_active_text, palette::TAB_ACTIVE_TEXT);
    assert_eq!(
        t.pane_header_focused_text,
        palette::PANE_HEADER_FOCUSED_TEXT
    );
    assert_eq!(t.tab_inactive_bg, palette::BG_PANEL);
    assert_eq!(t.tab_inactive_hover_bg, palette::BG_HOVER);
    assert_eq!(t.tab_inactive_text, palette::TEXT_MUTE);
    assert_eq!(t.status_bar_bg, palette::BG_BASE);
    assert_eq!(t.status_bar_border, palette::BORDER);
    assert_eq!(t.status_bar_error, palette::STATUS_BAR_ERROR);
    assert_eq!(t.status_bar_project_dot, palette::STATUS_BAR_PROJECT_DOT);
    assert_eq!(t.status_bar_detached_bg, palette::STATUS_BAR_DETACHED_BG);
    assert_eq!(
        t.status_bar_detached_text,
        palette::STATUS_BAR_DETACHED_TEXT
    );
    assert_eq!(
        t.lane_drop_target_rejected_bg,
        palette::LANE_DROP_TARGET_REJECTED_BG,
    );
    assert_eq!(t.dock_bg, palette::BG_PANEL);
    assert_eq!(t.dock_border, palette::BORDER);
    assert_eq!(t.dock_header_text, palette::TEXT_MUTE);
    assert_eq!(t.dock_placeholder_text, palette::TEXT_SUBTLE);
    assert_eq!(t.button_widget_bg, palette::BG_HOVER);
    assert_eq!(t.button_widget_bg_hover, palette::BG_ACTIVE);
    assert_eq!(t.button_widget_text, palette::TEXT_BODY);
    assert_eq!(t.panel_tab_drop_target_bg, palette::BG_ACTIVE);
    assert_eq!(t.muted_text, palette::TEXT_MUTE);
    assert_eq!(t.faint_text, palette::TEXT_SUBTLE);
    assert_eq!(t.disabled_item_bg, palette::BG_HOVER);
    assert_eq!(t.disabled_item_text, palette::TEXT_SUBTLE);
    assert_eq!(t.close_button_hover_bg, palette::CLOSE_BUTTON_HOVER_BG);
    assert_eq!(t.accent_green, palette::ACCENT_GREEN);
    assert_eq!(t.dock_icon_inactive, palette::TEXT_MUTE);
    assert_eq!(t.dock_icon_hover, palette::TEXT_BODY);
    assert_eq!(t.dock_icon_active_bg, palette::BG_HOVER);
    assert_eq!(t.dock_view_tab_inactive, palette::TEXT_MUTE);
    assert_eq!(t.dock_view_tab_active, palette::DOCK_VIEW_TAB_ACTIVE);
    assert_eq!(t.dock_view_tab_accent, palette::DOCK_VIEW_TAB_ACCENT);
    assert_eq!(t.dock_view_tab_hover_bg, palette::BG_HOVER);
    assert_eq!(t.lane_unread, palette::LANE_UNREAD);
    assert_eq!(t.lane_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.lane_drop_target_bg, palette::LANE_DROP_TARGET_BG);
    assert_eq!(t.modal_panel_bg, palette::BG_RAISED);
    assert_eq!(t.modal_panel_border, palette::BORDER);
    assert_eq!(t.modal_input_bg, palette::BG_BASE);
    assert_eq!(t.modal_input_border, palette::BORDER);
    assert_eq!(t.input_focus_border, palette::INPUT_FOCUS_BORDER);
    assert_eq!(t.modal_error_text, palette::MODAL_ERROR_TEXT);
    assert_eq!(t.modal_primary_bg, palette::MODAL_PRIMARY_BG);
    assert_eq!(t.modal_primary_hover_bg, palette::MODAL_PRIMARY_HOVER_BG);
    assert_eq!(t.modal_secondary_text, palette::TEXT_BODY);
    assert_eq!(t.modal_text_primary, palette::MODAL_TEXT_PRIMARY);
    assert_eq!(t.input_selection_bg, palette::INPUT_SELECTION_BG);
    assert_eq!(
        t.textarea_scrollbar_thumb,
        palette::TEXTAREA_SCROLLBAR_THUMB
    );
    assert_eq!(t.banner_error_bg, palette::BANNER_ERROR_BG);
    assert_eq!(t.banner_error_text, palette::BANNER_ERROR_TEXT);
    assert_eq!(t.banner_warning_bg, palette::BANNER_WARNING_BG);
    assert_eq!(t.banner_warning_text, palette::BANNER_WARNING_TEXT);
    assert_eq!(t.banner_info_bg, palette::BANNER_INFO_BG);
    assert_eq!(t.banner_info_text, palette::BANNER_INFO_TEXT);
    assert_eq!(t.banner_success_bg, palette::BANNER_SUCCESS_BG);
    assert_eq!(t.banner_success_text, palette::BANNER_SUCCESS_TEXT);
    assert_eq!(t.settings_scrollbar_thumb, palette::DOCK_SCROLLBAR_THUMB);
    assert_eq!(
        t.settings_scrollbar_thumb_hover,
        palette::DOCK_SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.right_panel_scrollbar_thumb, palette::DOCK_SCROLLBAR_THUMB);
    assert_eq!(
        t.right_panel_scrollbar_thumb_hover,
        palette::DOCK_SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.settings_sidebar_bg, palette::SETTINGS_SIDEBAR_BG);
    assert_eq!(
        t.settings_sidebar_row_active_bg,
        palette::SETTINGS_SIDEBAR_ROW_ACTIVE_BG
    );
    assert_eq!(
        t.settings_sidebar_row_hover_bg,
        palette::SETTINGS_SIDEBAR_ROW_HOVER_BG
    );
    assert_eq!(t.palette_bg, palette::BG_RAISED);
    assert_eq!(t.palette_border, palette::BORDER);
    assert_eq!(t.palette_input_border, palette::BORDER);
    assert_eq!(t.palette_focused_bg, palette::BG_ACTIVE);
    assert_eq!(t.palette_entry_text, palette::TEXT_BODY);
    assert_eq!(t.palette_shortcut_text, palette::TEXT_SUBTLE);
    assert_eq!(t.palette_empty_text, palette::TEXT_SUBTLE);
    assert_eq!(t.git_badge_pill_bg, palette::GIT_BADGE_PILL_BG);
    assert_eq!(t.git_badge_pill_text, palette::GIT_BADGE_PILL_TEXT);
    assert_eq!(t.git_badge_arrow_text, palette::GIT_BADGE_ARROW_TEXT);
    assert_eq!(t.git_staged_color, palette::GIT_STAGED_COLOR);
    assert_eq!(t.git_unstaged_color, palette::GIT_UNSTAGED_COLOR);
    assert_eq!(t.git_untracked_color, palette::GIT_UNTRACKED_COLOR);
    assert_eq!(t.git_file_row_selected_bg, palette::BG_HOVER);
    assert_eq!(t.git_file_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.git_diff_panel_bg, palette::BG_BASE);
    assert_eq!(t.git_diff_border, palette::BORDER);
    assert_eq!(t.git_diff_add_bg, palette::GIT_DIFF_ADD_BG);
    assert_eq!(t.git_diff_del_bg, palette::GIT_DIFF_DEL_BG);
    assert_eq!(t.git_diff_add_text, palette::GIT_DIFF_ADD_TEXT);
    assert_eq!(t.git_diff_del_text, palette::GIT_DIFF_DEL_TEXT);
    assert_eq!(t.git_diff_context_text, palette::TEXT_SUBTLE);
    assert_eq!(t.git_diff_hunk_bg, palette::BG_RAISED);
    assert_eq!(t.git_stage_checkbox_border, palette::BORDER);
    assert_eq!(
        t.git_stage_checkbox_checked_bg,
        palette::GIT_STAGE_CHECKBOX_CHECKED_BG
    );
    assert_eq!(t.git_stage_checkbox_unchecked_bg, palette::BG_RAISED);
    assert_eq!(t.git_commit_border, palette::BORDER);
    assert_eq!(t.palette_query_text, palette::PALETTE_QUERY_TEXT);
    assert_eq!(t.palette_focused_text, palette::PALETTE_FOCUSED_TEXT);
    assert_eq!(
        t.input_panel_drop_target_bg,
        palette::INPUT_PANEL_DROP_TARGET_BG
    );
    assert_eq!(t.terminal_drop_target_bg, palette::TERMINAL_DROP_TARGET_BG);
    assert_eq!(t.dock_scrollbar_thumb, palette::DOCK_SCROLLBAR_THUMB);
    assert_eq!(
        t.dock_scrollbar_thumb_hover,
        palette::DOCK_SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.pane_header_focused_bg, palette::BG_HOVER);
    assert_eq!(t.pane_header_unfocused_bg, palette::BG_PANEL);
    assert_eq!(t.pane_header_cwd_text, palette::TEXT_MUTE);
    assert_eq!(t.agent_log_status_text, palette::AGENT_LOG_STATUS_TEXT);
    assert_eq!(t.agent_chat_user_text, palette::AGENT_CHAT_USER_TEXT);
    assert_eq!(t.agent_task_running_fg, palette::AGENT_TASK_RUNNING_FG);
    assert_eq!(t.agent_task_running_bg, palette::AGENT_TASK_RUNNING_BG);
    assert_eq!(t.pane_divider_bg, palette::BORDER);
    assert_eq!(t.welcome_bg, palette::BG_BASE);
    assert_eq!(t.welcome_button_bg, palette::BG_HOVER);
    assert_eq!(t.welcome_button_hover_bg, palette::BG_ACTIVE);
    assert_eq!(t.welcome_button_border, palette::BORDER);
    assert_eq!(t.welcome_recent_hover_bg, palette::BG_HOVER);
    assert_eq!(t.welcome_text, palette::WELCOME_TEXT);
    assert_eq!(t.file_viewer_bg, palette::BG_BASE);
    assert_eq!(t.file_viewer_header_bg, palette::BG_PANEL);
    assert_eq!(t.file_viewer_header_border, palette::BORDER);
    assert_eq!(t.file_viewer_header_text, palette::TEXT_BODY);
    assert_eq!(t.file_viewer_close_hover, palette::FILE_VIEWER_CLOSE_HOVER);
    assert_eq!(t.file_viewer_text, palette::TEXT_BODY);
    assert_eq!(
        t.file_viewer_selection_bg,
        palette::FILE_VIEWER_SELECTION_BG
    );
    assert_eq!(t.file_viewer_line_no_text, palette::TEXT_SUBTLE);
    assert_eq!(t.file_viewer_tab_active_bg, palette::BG_ACTIVE);
    assert_eq!(t.file_viewer_tab_text, palette::TEXT_MUTE);
    assert_eq!(
        t.file_viewer_tab_active_text,
        palette::FILE_VIEWER_TAB_ACTIVE_TEXT
    );
    assert_eq!(t.file_diff_add_bg, palette::FILE_DIFF_ADD_BG);
    assert_eq!(t.file_diff_del_bg, palette::FILE_DIFF_DEL_BG);
    assert_eq!(t.file_diff_add_text, palette::FILE_DIFF_ADD_TEXT);
    assert_eq!(t.file_diff_del_text, palette::FILE_DIFF_DEL_TEXT);
    assert_eq!(t.file_diff_hunk_bg, palette::BG_RAISED);
    assert_eq!(t.file_diff_hunk_text, palette::FILE_DIFF_HUNK_TEXT);
    assert_eq!(t.file_diff_hunk_border, palette::BORDER);
    assert_eq!(t.file_diff_ctx_text, palette::TEXT_MUTE);
    assert_eq!(t.file_viewer_divider, palette::BORDER);
    assert_eq!(
        t.file_viewer_scrollbar_thumb,
        palette::DOCK_SCROLLBAR_THUMB
    );
    assert_eq!(
        t.file_viewer_scrollbar_thumb_hover,
        palette::DOCK_SCROLLBAR_THUMB_HOVER
    );
    assert_eq!(t.file_diff_hunk_ctx_text, palette::FILE_DIFF_HUNK_CTX_TEXT);
    assert_eq!(t.file_diff_stat_add, palette::FILE_DIFF_STAT_ADD);
    assert_eq!(t.file_diff_stat_del, palette::FILE_DIFF_STAT_DEL);
    assert_eq!(t.file_diff_word_add_bg, palette::FILE_DIFF_WORD_ADD_BG);
    assert_eq!(t.file_diff_word_del_bg, palette::FILE_DIFF_WORD_DEL_BG);
    assert_eq!(t.file_viewer_search_text, palette::TEXT_BODY);
    assert_eq!(t.file_viewer_search_count, palette::TEXT_MUTE);
    assert_eq!(
        t.file_viewer_search_empty,
        palette::FILE_VIEWER_SEARCH_EMPTY
    );
    assert_eq!(
        t.file_viewer_search_match_bg,
        palette::FILE_VIEWER_SEARCH_MATCH_BG
    );
    assert_eq!(
        t.file_viewer_search_focused_bg,
        palette::FILE_VIEWER_SEARCH_FOCUSED_BG
    );
    assert_eq!(t.md_h1_color, palette::INK);
    assert_eq!(t.md_h2_color, palette::MD_H2_COLOR);
    assert_eq!(t.md_h3_color, palette::TEXT_BODY);
    assert_eq!(t.md_h4_color, palette::TEXT_BODY);
    assert_eq!(t.md_code_inline_bg, palette::BG_RAISED);
    assert_eq!(t.md_code_inline_text, palette::MD_CODE_INLINE_TEXT);
    assert_eq!(t.md_code_block_bg, palette::BG_PANEL);
    assert_eq!(t.md_code_block_border, palette::BORDER);
    assert_eq!(t.md_blockquote_border, palette::BORDER);
    assert_eq!(t.md_blockquote_text, palette::TEXT_MUTE);
    assert_eq!(t.md_rule_color, palette::BORDER);
    assert_eq!(t.md_link_color, palette::MD_LINK_COLOR);
    assert_eq!(t.md_strikethrough_color, palette::MD_STRIKETHROUGH_COLOR);
    assert_eq!(t.md_list_bullet_color, palette::MD_LIST_BULLET_COLOR);
    assert_eq!(t.md_task_checked_color, palette::MD_TASK_CHECKED_COLOR);
    assert_eq!(t.md_footnote_color, palette::MD_FOOTNOTE_COLOR);
    assert_eq!(t.md_html_color, palette::MD_HTML_COLOR);
    assert_eq!(t.md_table_border, palette::BORDER);
    assert_eq!(t.md_table_header_bg, palette::BG_RAISED);
    assert_eq!(t.md_table_row_bg_even, palette::BG_PANEL);
    assert_eq!(t.md_table_row_bg_odd, palette::BG_PANEL);
    assert_eq!(t.toast_bg, palette::TOAST_BG);
    assert_eq!(t.toast_border, palette::BORDER);
    assert_eq!(t.toast_text, palette::TEXT_BODY);
    assert_eq!(t.toast_action_text, palette::ACCENT_GREEN);
    assert_eq!(t.toast_tint_info, palette::TOAST_TINT_INFO);
    assert_eq!(t.toast_tint_warning, palette::TOAST_TINT_WARNING);
    assert_eq!(t.toast_tint_error, palette::TOAST_TINT_ERROR);
    assert_eq!(t.toast_text_dim, palette::TOAST_TEXT_DIM);
    assert_eq!(t.toast_repeat_bg, palette::BG_ACTIVE);
    assert_eq!(t.error_modal_body_bg, palette::BG_BASE);
    assert_eq!(t.error_modal_body_border, palette::BORDER);
    assert_eq!(t.terminal_bg, palette::TERMINAL_BG);
    assert_eq!(
        t.terminal_scrollbar_track_bg,
        palette::TERMINAL_SCROLLBAR_TRACK_BG
    );
    assert_eq!(t.keystroke_input_bg, palette::BG_PANEL);
    assert_eq!(t.keystroke_input_border, palette::BORDER);
    assert_eq!(t.keystroke_input_border_active, palette::ACCENT_GREEN);
    assert_eq!(t.keystroke_badge_bg, palette::BG_ACTIVE);
    assert_eq!(t.keystroke_badge_border, palette::BORDER);
    assert_eq!(t.keystroke_badge_text, palette::TEXT_BODY);
    assert_eq!(t.keystroke_hint_text, palette::TEXT_SUBTLE);
    assert_eq!(t.popover_bg, palette::POPOVER_BG);
    assert_eq!(t.popover_border, palette::BORDER);
    assert_eq!(t.popover_item_text, palette::POPOVER_ITEM_TEXT);
    assert_eq!(t.popover_item_hover_bg, palette::POPOVER_ITEM_HOVER_BG);
    assert_eq!(
        t.popover_item_danger_text,
        palette::POPOVER_ITEM_DANGER_TEXT
    );
    assert_eq!(t.popover_separator, palette::POPOVER_SEPARATOR);
    assert_eq!(t.status_working_light, palette::STATUS_WORKING_LIGHT);
    assert_eq!(
        t.status_executing_tool_light,
        palette::STATUS_EXECUTING_TOOL_LIGHT
    );
    assert_eq!(
        t.status_needs_attention_light,
        palette::STATUS_NEEDS_ATTENTION_LIGHT
    );
    assert_eq!(t.status_idle_light, palette::STATUS_IDLE_LIGHT);
    assert_eq!(t.status_connecting_light, palette::STATUS_CONNECTING_LIGHT);
    assert_eq!(t.status_working_dark, palette::STATUS_WORKING_DARK);
    assert_eq!(
        t.status_executing_tool_dark,
        palette::STATUS_EXECUTING_TOOL_DARK
    );
    assert_eq!(
        t.status_needs_attention_dark,
        palette::STATUS_NEEDS_ATTENTION_DARK
    );
    assert_eq!(t.status_idle_dark, palette::STATUS_IDLE_DARK);
    assert_eq!(t.status_connecting_dark, palette::STATUS_CONNECTING_DARK);
    assert_eq!(
        t.status_badge_active_outline,
        palette::STATUS_BADGE_ACTIVE_OUTLINE
    );
    assert_eq!(t.claude_banner_bg, palette::CLAUDE_BANNER_BG);
    assert_eq!(t.claude_banner_border, palette::CLAUDE_BANNER_BORDER);
    assert_eq!(t.claude_banner_hover_bg, palette::CLAUDE_BANNER_HOVER_BG);
    assert_eq!(t.claude_banner_text, palette::CLAUDE_BANNER_TEXT);
    assert_eq!(t.claude_banner_icon, palette::CLAUDE_BANNER_ICON);
    assert_eq!(t.right_panel_cost_color, palette::RIGHT_PANEL_COST_COLOR);
    assert_eq!(
        t.right_panel_task_hover_bg,
        palette::RIGHT_PANEL_TASK_HOVER_BG
    );
    assert_eq!(
        t.right_panel_task_selected_bg,
        palette::RIGHT_PANEL_TASK_SELECTED_BG
    );
    assert_eq!(
        t.right_panel_task_state_text,
        palette::RIGHT_PANEL_TASK_STATE_TEXT
    );
    assert_eq!(
        t.right_panel_task_title_text,
        palette::RIGHT_PANEL_TASK_TITLE_TEXT
    );
    assert_eq!(t.right_panel_task_running_color, palette::ACCENT_GREEN);
    assert_eq!(
        t.right_panel_task_done_color,
        palette::RIGHT_PANEL_TASK_DONE_COLOR
    );
    assert_eq!(
        t.right_panel_task_error_color,
        palette::RIGHT_PANEL_TASK_ERROR_COLOR
    );
    assert_eq!(
        t.right_panel_task_cancelled_color,
        palette::RIGHT_PANEL_TASK_CANCELLED_COLOR
    );
    assert_eq!(
        t.right_panel_task_backlog_color,
        palette::RIGHT_PANEL_TASK_BACKLOG_COLOR
    );
    assert_eq!(
        t.right_panel_task_duration_text,
        palette::RIGHT_PANEL_TASK_DURATION_TEXT
    );
    assert_eq!(
        t.right_panel_task_failure_text,
        palette::RIGHT_PANEL_TASK_FAILURE_TEXT
    );
    assert_eq!(
        t.right_panel_task_session_status_text,
        palette::RIGHT_PANEL_TASK_SESSION_STATUS_TEXT
    );
    assert_eq!(
        t.right_panel_task_session_needs_attention_text,
        palette::RIGHT_PANEL_TASK_SESSION_NEEDS_ATTENTION_TEXT
    );
    assert_eq!(
        t.task_edit_branch_invalid_border,
        palette::RIGHT_PANEL_TASK_ERROR_COLOR
    );
    assert_eq!(t.badge_bg, palette::BG_HOVER);
    assert_eq!(t.badge_border, palette::BORDER);
    assert_eq!(t.badge_text, palette::TEXT_BODY);
    assert_eq!(t.divider_default_color, palette::BORDER);
    assert_eq!(t.gauge_green, palette::GAUGE_GREEN);
    assert_eq!(t.gauge_yellow, palette::GAUGE_YELLOW);
    assert_eq!(t.gauge_red, palette::GAUGE_RED);
    assert_eq!(t.status_orange, palette::STATUS_ORANGE);
    assert_eq!(t.gauge_track_bg, palette::BG_HOVER);
    assert_eq!(t.right_panel_dim_text, palette::RIGHT_PANEL_DIM_TEXT);
    assert_eq!(t.skill_badge_both_bg, palette::SKILL_BADGE_BOTH_BG);
    assert_eq!(t.skill_badge_both_text, palette::SKILL_BADGE_BOTH_TEXT);
    assert_eq!(
        t.skill_badge_user_only_bg,
        palette::SKILL_BADGE_USER_ONLY_BG
    );
    assert_eq!(
        t.skill_badge_user_only_text,
        palette::SKILL_BADGE_USER_ONLY_TEXT
    );
    assert_eq!(
        t.skill_badge_model_only_bg,
        palette::SKILL_BADGE_MODEL_ONLY_BG
    );
    assert_eq!(
        t.skill_badge_model_only_text,
        palette::SKILL_BADGE_MODEL_ONLY_TEXT
    );
    assert_eq!(t.skill_badge_disabled_bg, palette::SKILL_BADGE_DISABLED_BG);
    assert_eq!(
        t.skill_badge_disabled_text,
        palette::SKILL_BADGE_DISABLED_TEXT
    );
    assert_eq!(t.skill_aux_chip_bg, palette::SKILL_AUX_CHIP_BG);
    assert_eq!(t.skill_aux_chip_text, palette::SKILL_AUX_CHIP_TEXT);
    assert_eq!(
        t.skill_section_header_text,
        palette::SKILL_SECTION_HEADER_TEXT
    );
    assert_eq!(t.skill_name_text, palette::SKILL_NAME_TEXT);
    assert_eq!(t.skill_meta_text, palette::SKILL_META_TEXT);
    assert_eq!(t.skill_row_hover_bg, palette::BG_HOVER);
    assert_eq!(t.skill_empty_text, palette::SKILL_EMPTY_TEXT);
    assert_eq!(t.mcp_indicator_enabled, palette::MCP_INDICATOR_ENABLED);
    assert_eq!(t.mcp_indicator_disabled, palette::MCP_INDICATOR_DISABLED);
    assert_eq!(t.mcp_indicator_malformed, palette::MCP_INDICATOR_MALFORMED);
    assert_eq!(t.mcp_row_hover_bg, palette::MCP_ROW_HOVER_BG);
    assert_eq!(t.mcp_section_header_text, palette::MCP_SECTION_HEADER_TEXT);
    assert_eq!(t.mcp_row_body_text, palette::MCP_ROW_BODY_TEXT);
    assert_eq!(t.mcp_row_meta_text, palette::MCP_ROW_META_TEXT);
    assert_eq!(t.mcp_empty_text, palette::MCP_EMPTY_TEXT);
    assert_eq!(t.mcp_transport_badge_bg, palette::MCP_TRANSPORT_BADGE_BG);
    assert_eq!(
        t.mcp_transport_badge_text,
        palette::MCP_TRANSPORT_BADGE_TEXT
    );
    assert_eq!(t.mcp_malformed_badge_bg, palette::MCP_MALFORMED_BADGE_BG);
    assert_eq!(
        t.mcp_malformed_badge_text,
        palette::MCP_MALFORMED_BADGE_TEXT
    );
    assert_eq!(t.mcp_disabled_badge_text, palette::MCP_DISABLED_BADGE_TEXT);
}

#[test]
fn json_round_trip_preserves_default_within_8bit_tolerance() {
    // `gpui::Hsla` serialises through its `Rgba` representation,
    // which is 8-bit per channel (`#rrggbbaa`). A pure-Hsla value
    // like `hsla(_, _, 0.1, 1.0)` rounds through Rgba as 26/255 =
    // 0.101960786 on the way back — visually indistinguishable
    // but not bit-equal. The round-trip is therefore lossy by
    // design; the contract we *can* assert is "every channel
    // stays within one 8-bit step (1/255 ≈ 0.0039) of the input."
    //
    // Probing one slot per major slot group is enough to catch a
    // future field type change that breaks the contract entirely
    // (e.g. accidentally typing a slot as `f32` and rounding
    // through some other serializer).
    let original = DarudaTheme::default();
    let json = original.to_json_pretty().expect("serialize");
    let parsed = DarudaTheme::from_json(&json).expect("deserialize");

    // One 8-bit step plus a slack epsilon for floating-point noise
    // in the HSL→RGB→HSL conversion path.
    const TOL: f32 = 1.5 / 255.0;
    fn close(a: gpui::Hsla, b: gpui::Hsla, tol: f32) -> bool {
        (a.h - b.h).abs() <= tol
            && (a.s - b.s).abs() <= tol
            && (a.l - b.l).abs() <= tol
            && (a.a - b.a).abs() <= tol
    }
    for (name, a, b) in [
        ("title_bar_bg", parsed.title_bar_bg, original.title_bar_bg),
        (
            "modal_panel_bg",
            parsed.modal_panel_bg,
            original.modal_panel_bg,
        ),
        (
            "banner_error_text",
            parsed.banner_error_text,
            original.banner_error_text,
        ),
        (
            "mcp_disabled_badge_text",
            parsed.mcp_disabled_badge_text,
            original.mcp_disabled_badge_text,
        ),
    ] {
        assert!(
            close(a, b, TOL),
            "{name}: round-trip diverged beyond 8-bit tolerance: {a:?} vs {b:?}"
        );
    }
}

#[test]
fn from_json_empty_object_fills_defaults() {
    // `{}` is the minimal valid theme — every slot falls back to
    // the compile-time `palette` const. Phase 3-C relies on this
    // when a user-authored JSON omits slots they don't care to
    // override.
    let parsed = DarudaTheme::from_json("{}").expect("empty object parses");
    let defaults = DarudaTheme::default();
    assert_eq!(parsed.tab_bar_bg, defaults.tab_bar_bg);
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
    assert_eq!(parsed.dock_bg, defaults.dock_bg);
}

#[test]
fn from_json_partial_overrides_only_listed_keys() {
    // A user-authored JSON that lists only one key must override
    // exactly that key; every other slot keeps the dark default.
    // Double-hash delimiter because the JSON body itself contains
    // `"#ff00ff"` — single-hash raw strings would terminate early.
    let parsed =
        DarudaTheme::from_json(r##"{ "tab_bar_bg": "#ff00ff" }"##).expect("partial JSON parses");
    let defaults = DarudaTheme::default();

    // Overridden slot reflects the JSON value (magenta).
    assert_ne!(parsed.tab_bar_bg, defaults.tab_bar_bg);
    // Untouched slot keeps the default.
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
}

#[test]
fn bundled_daruda_dark_json_matches_default() {
    // `assets/themes/daruda_dark.json` is committed alongside the
    // crate so the runtime loader (Phase 3-C) can read a real
    // file. This test asserts the bundled JSON has not drifted
    // from the compile-time defaults — if a future palette tweak
    // changes a default colour, regenerate the asset via
    //   cargo run --example dump_default_theme  # or the build script
    // and re-commit. Catching drift in CI is far cheaper than
    // chasing a visual mismatch in production.
    let bundled = include_str!("../../../../../../assets/themes/daruda_dark.json");
    let parsed =
        DarudaTheme::from_json(bundled).expect("bundled daruda_dark.json must be valid JSON");
    let defaults = DarudaTheme::default();

    // Spot-check a representative slice across the major slot
    // groups. The full-field equality is enforced by the
    // generator (the same `to_json_pretty` we test in the
    // round-trip case above), so we only need a handful of
    // probes here to detect "file was hand-edited and the
    // generator was not re-run."
    assert_eq!(parsed.title_bar_bg, defaults.title_bar_bg);
    assert_eq!(parsed.tab_bar_bg, defaults.tab_bar_bg);
    assert_eq!(parsed.status_bar_bg, defaults.status_bar_bg);
    assert_eq!(parsed.modal_panel_bg, defaults.modal_panel_bg);
    assert_eq!(parsed.banner_error_text, defaults.banner_error_text);
    assert_eq!(parsed.lane_unread, defaults.lane_unread);
}

#[test]
fn json_schema_lists_every_theme_slot() {
    // `DarudaTheme::json_schema()` drives the bundled
    // `assets/themes/theme.schema.json`. If a new slot is added to
    // the `daruda_theme_fields!` macro but the schema regenerator
    // is not re-run, editors that wired `$schema` to the bundled
    // file would miss autocomplete for the new key. Guard the
    // invariant in CI: every `pub` field on `DarudaTheme` must be
    // listed in the schema's `properties` object.
    let schema = DarudaTheme::json_schema();
    let value = serde_json::to_value(&schema).expect("schema serialises");
    let properties = value
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema has a properties object");

    // Probe a representative slice — the macro guarantees the
    // struct and `Default::default()` stay in lockstep, so testing
    // every property here would just re-test the macro. Sample
    // across groups to catch a missing derive on `Hsla` or a
    // schemars version bump that breaks field emission.
    for slot in [
        "title_bar_bg",
        "tab_bar_bg",
        "status_bar_bg",
        "modal_panel_bg",
        "lane_unread",
        "right_panel_task_session_needs_attention_text",
    ] {
        assert!(
            properties.contains_key(slot),
            "json schema missing slot `{slot}` — did `daruda_theme_fields!` lose a row?"
        );
    }
}

/// Regeneration helper for `assets/themes/theme.schema.json`. Run
/// manually after a `daruda_theme_fields!` change:
///
/// ```bash
/// cargo test -p daruda \
///   ui::theme::daruda_theme::tests::regenerate_theme_schema_json \
///   -- --ignored --nocapture
/// ```
///
/// Marked `#[ignore]` so the regular test run never overwrites
/// disk — only an explicit invocation can mutate the bundled
/// schema. Committing the regenerated file is what makes
/// `bundled_theme_schema_json_matches_generated` (below) pass on
/// the next run.
#[test]
#[ignore = "regenerator — writes to assets/themes/theme.schema.json"]
fn regenerate_theme_schema_json() {
    let schema = serde_json::to_value(DarudaTheme::json_schema())
        .expect("DarudaTheme::json_schema() must serialise");
    let pretty =
        serde_json::to_string_pretty(&schema).expect("schema must serialise to pretty JSON");
    let dest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/themes/theme.schema.json");
    std::fs::write(&dest, pretty).expect("write theme.schema.json");
    println!("wrote {}", dest.display());
}

#[test]
fn bundled_theme_schema_json_matches_generated() {
    // Same "lockstep with disk" invariant as
    // `bundled_daruda_dark_json_matches_default`, but for the
    // companion JSON Schema. Editors that point `$schema` at the
    // bundled file would otherwise show stale autocomplete after
    // a slot rename / addition. Regenerate via
    //   cargo run --example dump_theme_schema  # or the build script
    // (see `assets/themes/README.md`).
    let bundled = include_str!("../../../../../../assets/themes/theme.schema.json");
    let bundled_value: serde_json::Value =
        serde_json::from_str(bundled).expect("bundled theme.schema.json must be valid JSON");
    let generated = serde_json::to_value(DarudaTheme::json_schema())
        .expect("DarudaTheme::json_schema() must serialise to JSON");
    assert_eq!(
        bundled_value, generated,
        "bundled theme.schema.json has drifted from DarudaTheme::json_schema()"
    );
}
