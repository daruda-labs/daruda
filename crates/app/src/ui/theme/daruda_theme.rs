//! `DarudaTheme` — runtime container for every themable colour
//! that lives in `app/src/ui/theme/palette.rs`.
//!
//! Every `pub const FOO: Hsla` in `palette` has a matching field
//! here (lowercase + snake_case). `Default::default()` clones the
//! compile-time constants, so an uninitialised `DarudaTheme` is
//! visually identical to the pre-Phase-3 build — the field-based
//! lookup gains its purpose only once `DarudaTheme::from_json`
//! (Phase 3-C) and `Theme::change()` (Phase 3-D) start mutating
//! the live instance.
//!
//! Registered as a GPUI `Global` so any entity can read the current
//! palette through `cx.global::<DarudaTheme>().<slot>` — the same
//! access pattern Zed uses for `SettingsStore` / `ThemeRegistry`.
//! `init(cx)` is idempotent (`if !cx.has_global::<DarudaTheme>()`)
//! so test fixtures and defensive bootstrap calls do not stomp on
//! a live theme.
//!
//! Serde notes:
//! - `gpui::Hsla` deserialises from a CSS-style `"#rrggbb"` /
//!   `"#rrggbbaa"` string (via the upstream `Rgba` round-trip), so a
//!   JSON file reads like `{ "title_bar_bg": "#1a1a1a", ... }`.
//! - Every field is `#[serde(default)]` at the struct level so a
//!   partial JSON file (missing keys, comments-removed minified
//!   form, etc.) still produces a valid `DarudaTheme`; missing keys
//!   fall through to the compile-time `palette` constants.
//!
//! Adding a new theme slot:
//! 1. Add a `pub const FOO: Hsla = ...` to `palette.rs`.
//! 2. Add `foo => FOO,` to the [`daruda_theme_fields!`] call below.
//!
//! No other site needs editing — the macro keeps the struct
//! definition, the field list, and `Default::default()` in lockstep.

use crate::ui::theme::palette;
use gpui::{App, Global, Hsla};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Generate the `DarudaTheme` struct and its `Default` impl from a
/// single `field => CONST,` list. Replaces the previous pattern
/// where the field declaration, the `Default::default()` map, and
/// the docstring lived in three separate sites.
///
/// The macro is intentionally local (not `#[macro_export]`) so the
/// `palette::` resolution stays tied to *this* module's `use`.
macro_rules! daruda_theme_fields {
    ( $( $field:ident => $const:ident ),* $(,)? ) => {
        #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
        #[serde(default)]
        pub struct DarudaTheme {
            $(
                #[doc = concat!("Maps to `palette::", stringify!($const), "`.")]
                pub $field: Hsla,
            )*
        }

        impl Default for DarudaTheme {
            fn default() -> Self {
                Self {
                    $($field: palette::$const,)*
                }
            }
        }
    };
}

daruda_theme_fields! {
    title_bar_bg => TITLE_BAR_BG,
    tab_bar_bg => TAB_BAR_BG,
    tab_bar_border => TAB_BAR_BORDER,
    tab_active_bg => TAB_ACTIVE_BG,
    tab_active_text => TAB_ACTIVE_TEXT,
    pane_header_focused_text => PANE_HEADER_FOCUSED_TEXT,
    tab_inactive_bg => TAB_INACTIVE_BG,
    tab_inactive_hover_bg => TAB_INACTIVE_HOVER_BG,
    tab_inactive_text => TAB_INACTIVE_TEXT,
    status_bar_bg => STATUS_BAR_BG,
    status_bar_border => STATUS_BAR_BORDER,
    status_bar_error => STATUS_BAR_ERROR,
    status_bar_project_dot => STATUS_BAR_PROJECT_DOT,
    status_bar_detached_bg => STATUS_BAR_DETACHED_BG,
    status_bar_detached_text => STATUS_BAR_DETACHED_TEXT,
    dock_bg => DOCK_BG,
    dock_border => DOCK_BORDER,
    dock_header_text => DOCK_HEADER_TEXT,
    dock_placeholder_text => DOCK_PLACEHOLDER_TEXT,
    button_widget_bg => BUTTON_WIDGET_BG,
    button_widget_bg_hover => BUTTON_WIDGET_BG_HOVER,
    button_widget_text => BUTTON_WIDGET_TEXT,
    panel_tab_drop_target_bg => PANEL_TAB_DROP_TARGET_BG,
    muted_text => MUTED_TEXT,
    faint_text => FAINT_TEXT,
    disabled_item_bg => DISABLED_ITEM_BG,
    disabled_item_text => DISABLED_ITEM_TEXT,
    close_button_hover_bg => CLOSE_BUTTON_HOVER_BG,
    accent_green => ACCENT_GREEN,
    dock_icon_inactive => DOCK_ICON_INACTIVE,
    dock_icon_hover => DOCK_ICON_HOVER,
    dock_icon_active_bg => DOCK_ICON_ACTIVE_BG,
    dock_view_tab_inactive => DOCK_VIEW_TAB_INACTIVE,
    dock_view_tab_active => DOCK_VIEW_TAB_ACTIVE,
    dock_view_tab_accent => DOCK_VIEW_TAB_ACCENT,
    dock_view_tab_hover_bg => DOCK_VIEW_TAB_HOVER_BG,
    lane_unread => LANE_UNREAD,
    lane_row_hover_bg => LANE_ROW_HOVER_BG,
    lane_drop_target_bg => LANE_DROP_TARGET_BG,
    lane_drop_target_rejected_bg => LANE_DROP_TARGET_REJECTED_BG,
    lane_card_bg => LANE_CARD_BG,
    lane_card_border => LANE_CARD_BORDER,
    lane_card_hover_bg => LANE_CARD_HOVER_BG,
    lane_card_active_bg => LANE_CARD_ACTIVE_BG,
    lane_row_active_bg => LANE_ROW_ACTIVE_BG,
    modal_panel_bg => MODAL_PANEL_BG,
    modal_panel_border => MODAL_PANEL_BORDER,
    modal_input_bg => MODAL_INPUT_BG,
    modal_input_border => MODAL_INPUT_BORDER,
    input_focus_border => INPUT_FOCUS_BORDER,
    modal_error_text => MODAL_ERROR_TEXT,
    modal_primary_bg => MODAL_PRIMARY_BG,
    modal_primary_hover_bg => MODAL_PRIMARY_HOVER_BG,
    modal_secondary_text => MODAL_SECONDARY_TEXT,
    modal_text_primary => MODAL_TEXT_PRIMARY,
    input_selection_bg => INPUT_SELECTION_BG,
    textarea_scrollbar_thumb => TEXTAREA_SCROLLBAR_THUMB,
    banner_error_bg => BANNER_ERROR_BG,
    banner_error_text => BANNER_ERROR_TEXT,
    banner_warning_bg => BANNER_WARNING_BG,
    banner_warning_text => BANNER_WARNING_TEXT,
    banner_info_bg => BANNER_INFO_BG,
    banner_info_text => BANNER_INFO_TEXT,
    banner_success_bg => BANNER_SUCCESS_BG,
    banner_success_text => BANNER_SUCCESS_TEXT,
    settings_scrollbar_thumb => SETTINGS_SCROLLBAR_THUMB,
    settings_scrollbar_thumb_hover => SETTINGS_SCROLLBAR_THUMB_HOVER,
    right_panel_scrollbar_thumb => RIGHT_PANEL_SCROLLBAR_THUMB,
    right_panel_scrollbar_thumb_hover => RIGHT_PANEL_SCROLLBAR_THUMB_HOVER,
    settings_sidebar_bg => SETTINGS_SIDEBAR_BG,
    settings_sidebar_row_active_bg => SETTINGS_SIDEBAR_ROW_ACTIVE_BG,
    settings_sidebar_row_hover_bg => SETTINGS_SIDEBAR_ROW_HOVER_BG,
    palette_bg => PALETTE_BG,
    palette_border => PALETTE_BORDER,
    palette_input_border => PALETTE_INPUT_BORDER,
    palette_focused_bg => PALETTE_FOCUSED_BG,
    palette_entry_text => PALETTE_ENTRY_TEXT,
    palette_shortcut_text => PALETTE_SHORTCUT_TEXT,
    palette_empty_text => PALETTE_EMPTY_TEXT,
    git_badge_pill_bg => GIT_BADGE_PILL_BG,
    git_badge_pill_text => GIT_BADGE_PILL_TEXT,
    git_badge_arrow_text => GIT_BADGE_ARROW_TEXT,
    git_staged_color => GIT_STAGED_COLOR,
    git_unstaged_color => GIT_UNSTAGED_COLOR,
    git_untracked_color => GIT_UNTRACKED_COLOR,
    git_file_row_selected_bg => GIT_FILE_ROW_SELECTED_BG,
    git_file_row_hover_bg => GIT_FILE_ROW_HOVER_BG,
    git_diff_panel_bg => GIT_DIFF_PANEL_BG,
    git_diff_border => GIT_DIFF_BORDER,
    git_diff_add_bg => GIT_DIFF_ADD_BG,
    git_diff_del_bg => GIT_DIFF_DEL_BG,
    git_diff_add_text => GIT_DIFF_ADD_TEXT,
    git_diff_del_text => GIT_DIFF_DEL_TEXT,
    git_diff_context_text => GIT_DIFF_CONTEXT_TEXT,
    git_diff_hunk_bg => GIT_DIFF_HUNK_BG,
    git_stage_checkbox_border => GIT_STAGE_CHECKBOX_BORDER,
    git_stage_checkbox_checked_bg => GIT_STAGE_CHECKBOX_CHECKED_BG,
    git_stage_checkbox_unchecked_bg => GIT_STAGE_CHECKBOX_UNCHECKED_BG,
    git_commit_border => GIT_COMMIT_BORDER,
    palette_query_text => PALETTE_QUERY_TEXT,
    palette_focused_text => PALETTE_FOCUSED_TEXT,
    input_panel_drop_target_bg => INPUT_PANEL_DROP_TARGET_BG,
    terminal_drop_target_bg => TERMINAL_DROP_TARGET_BG,
    dock_scrollbar_thumb => DOCK_SCROLLBAR_THUMB,
    dock_scrollbar_thumb_hover => DOCK_SCROLLBAR_THUMB_HOVER,
    pane_header_focused_bg => PANE_HEADER_FOCUSED_BG,
    pane_header_unfocused_bg => PANE_HEADER_UNFOCUSED_BG,
    pane_header_cwd_text => PANE_HEADER_CWD_TEXT,
    agent_log_status_text => AGENT_LOG_STATUS_TEXT,
    agent_chat_user_text => AGENT_CHAT_USER_TEXT,
    agent_task_running_fg => AGENT_TASK_RUNNING_FG,
    agent_task_running_bg => AGENT_TASK_RUNNING_BG,
    pane_divider_bg => PANE_DIVIDER_BG,
    welcome_bg => WELCOME_BG,
    welcome_button_bg => WELCOME_BUTTON_BG,
    welcome_button_hover_bg => WELCOME_BUTTON_HOVER_BG,
    welcome_button_border => WELCOME_BUTTON_BORDER,
    welcome_recent_hover_bg => WELCOME_RECENT_HOVER_BG,
    welcome_text => WELCOME_TEXT,
    file_viewer_bg => FILE_VIEWER_BG,
    file_viewer_header_bg => FILE_VIEWER_HEADER_BG,
    file_viewer_header_border => FILE_VIEWER_HEADER_BORDER,
    file_viewer_header_text => FILE_VIEWER_HEADER_TEXT,
    file_viewer_close_hover => FILE_VIEWER_CLOSE_HOVER,
    file_viewer_text => FILE_VIEWER_TEXT,
    file_viewer_selection_bg => FILE_VIEWER_SELECTION_BG,
    file_viewer_line_no_text => FILE_VIEWER_LINE_NO_TEXT,
    file_viewer_tab_active_bg => FILE_VIEWER_TAB_ACTIVE_BG,
    file_viewer_tab_text => FILE_VIEWER_TAB_TEXT,
    file_viewer_tab_active_text => FILE_VIEWER_TAB_ACTIVE_TEXT,
    file_diff_add_bg => FILE_DIFF_ADD_BG,
    file_diff_del_bg => FILE_DIFF_DEL_BG,
    file_diff_add_text => FILE_DIFF_ADD_TEXT,
    file_diff_del_text => FILE_DIFF_DEL_TEXT,
    file_diff_hunk_bg => FILE_DIFF_HUNK_BG,
    file_diff_hunk_text => FILE_DIFF_HUNK_TEXT,
    file_diff_hunk_border => FILE_DIFF_HUNK_BORDER,
    file_diff_ctx_text => FILE_DIFF_CTX_TEXT,
    file_viewer_divider => FILE_VIEWER_DIVIDER,
    file_viewer_scrollbar_thumb => FILE_VIEWER_SCROLLBAR_THUMB,
    file_viewer_scrollbar_thumb_hover => FILE_VIEWER_SCROLLBAR_THUMB_HOVER,
    file_diff_hunk_ctx_text => FILE_DIFF_HUNK_CTX_TEXT,
    file_diff_stat_add => FILE_DIFF_STAT_ADD,
    file_diff_stat_del => FILE_DIFF_STAT_DEL,
    file_diff_word_add_bg => FILE_DIFF_WORD_ADD_BG,
    file_diff_word_del_bg => FILE_DIFF_WORD_DEL_BG,
    file_viewer_search_text => FILE_VIEWER_SEARCH_TEXT,
    file_viewer_search_count => FILE_VIEWER_SEARCH_COUNT,
    file_viewer_search_empty => FILE_VIEWER_SEARCH_EMPTY,
    file_viewer_search_match_bg => FILE_VIEWER_SEARCH_MATCH_BG,
    file_viewer_search_focused_bg => FILE_VIEWER_SEARCH_FOCUSED_BG,
    md_h1_color => MD_H1_COLOR,
    md_h2_color => MD_H2_COLOR,
    md_h3_color => MD_H3_COLOR,
    md_h4_color => MD_H4_COLOR,
    md_code_inline_bg => MD_CODE_INLINE_BG,
    md_code_inline_text => MD_CODE_INLINE_TEXT,
    md_code_block_bg => MD_CODE_BLOCK_BG,
    md_code_block_border => MD_CODE_BLOCK_BORDER,
    md_blockquote_border => MD_BLOCKQUOTE_BORDER,
    md_blockquote_text => MD_BLOCKQUOTE_TEXT,
    md_rule_color => MD_RULE_COLOR,
    md_link_color => MD_LINK_COLOR,
    md_strikethrough_color => MD_STRIKETHROUGH_COLOR,
    md_list_bullet_color => MD_LIST_BULLET_COLOR,
    md_task_checked_color => MD_TASK_CHECKED_COLOR,
    md_footnote_color => MD_FOOTNOTE_COLOR,
    md_html_color => MD_HTML_COLOR,
    md_table_border => MD_TABLE_BORDER,
    md_table_header_bg => MD_TABLE_HEADER_BG,
    md_table_row_bg_even => MD_TABLE_ROW_BG_EVEN,
    md_table_row_bg_odd => MD_TABLE_ROW_BG_ODD,
    toast_bg => TOAST_BG,
    toast_border => TOAST_BORDER,
    toast_text => TOAST_TEXT,
    toast_action_text => TOAST_ACTION_TEXT,
    toast_tint_info => TOAST_TINT_INFO,
    toast_tint_warning => TOAST_TINT_WARNING,
    toast_tint_error => TOAST_TINT_ERROR,
    toast_text_dim => TOAST_TEXT_DIM,
    toast_repeat_bg => TOAST_REPEAT_BG,
    error_modal_body_bg => ERROR_MODAL_BODY_BG,
    error_modal_body_border => ERROR_MODAL_BODY_BORDER,
    terminal_bg => TERMINAL_BG,
    terminal_scrollbar_track_bg => TERMINAL_SCROLLBAR_TRACK_BG,
    keystroke_input_bg => KEYSTROKE_INPUT_BG,
    keystroke_input_border => KEYSTROKE_INPUT_BORDER,
    keystroke_input_border_active => KEYSTROKE_INPUT_BORDER_ACTIVE,
    keystroke_badge_bg => KEYSTROKE_BADGE_BG,
    keystroke_badge_border => KEYSTROKE_BADGE_BORDER,
    keystroke_badge_text => KEYSTROKE_BADGE_TEXT,
    keystroke_hint_text => KEYSTROKE_HINT_TEXT,
    popover_bg => POPOVER_BG,
    popover_border => POPOVER_BORDER,
    popover_item_text => POPOVER_ITEM_TEXT,
    popover_item_hover_bg => POPOVER_ITEM_HOVER_BG,
    popover_item_danger_text => POPOVER_ITEM_DANGER_TEXT,
    popover_separator => POPOVER_SEPARATOR,
    status_working_light => STATUS_WORKING_LIGHT,
    status_executing_tool_light => STATUS_EXECUTING_TOOL_LIGHT,
    status_needs_attention_light => STATUS_NEEDS_ATTENTION_LIGHT,
    status_idle_light => STATUS_IDLE_LIGHT,
    status_connecting_light => STATUS_CONNECTING_LIGHT,
    status_working_dark => STATUS_WORKING_DARK,
    status_executing_tool_dark => STATUS_EXECUTING_TOOL_DARK,
    status_needs_attention_dark => STATUS_NEEDS_ATTENTION_DARK,
    status_idle_dark => STATUS_IDLE_DARK,
    status_connecting_dark => STATUS_CONNECTING_DARK,
    status_badge_active_outline => STATUS_BADGE_ACTIVE_OUTLINE,
    claude_banner_bg => CLAUDE_BANNER_BG,
    claude_banner_border => CLAUDE_BANNER_BORDER,
    claude_banner_hover_bg => CLAUDE_BANNER_HOVER_BG,
    claude_banner_text => CLAUDE_BANNER_TEXT,
    claude_banner_icon => CLAUDE_BANNER_ICON,
    right_panel_cost_color => RIGHT_PANEL_COST_COLOR,
    right_panel_task_hover_bg => RIGHT_PANEL_TASK_HOVER_BG,
    right_panel_task_selected_bg => RIGHT_PANEL_TASK_SELECTED_BG,
    right_panel_task_state_text => RIGHT_PANEL_TASK_STATE_TEXT,
    right_panel_task_title_text => RIGHT_PANEL_TASK_TITLE_TEXT,
    right_panel_task_running_color => RIGHT_PANEL_TASK_RUNNING_COLOR,
    right_panel_task_done_color => RIGHT_PANEL_TASK_DONE_COLOR,
    right_panel_task_error_color => RIGHT_PANEL_TASK_ERROR_COLOR,
    right_panel_task_cancelled_color => RIGHT_PANEL_TASK_CANCELLED_COLOR,
    right_panel_task_backlog_color => RIGHT_PANEL_TASK_BACKLOG_COLOR,
    right_panel_task_duration_text => RIGHT_PANEL_TASK_DURATION_TEXT,
    right_panel_task_failure_text => RIGHT_PANEL_TASK_FAILURE_TEXT,
    right_panel_task_session_status_text => RIGHT_PANEL_TASK_SESSION_STATUS_TEXT,
    right_panel_task_session_needs_attention_text => RIGHT_PANEL_TASK_SESSION_NEEDS_ATTENTION_TEXT,
    task_edit_branch_invalid_border => TASK_EDIT_BRANCH_INVALID_BORDER,
    badge_bg => BADGE_BG,
    badge_border => BADGE_BORDER,
    badge_text => BADGE_TEXT,
    divider_default_color => DIVIDER_DEFAULT_COLOR,
    gauge_green => GAUGE_GREEN,
    gauge_yellow => GAUGE_YELLOW,
    gauge_red => GAUGE_RED,
    status_orange => STATUS_ORANGE,
    gauge_track_bg => GAUGE_TRACK_BG,
    right_panel_dim_text => RIGHT_PANEL_DIM_TEXT,
    skill_badge_both_bg => SKILL_BADGE_BOTH_BG,
    skill_badge_both_text => SKILL_BADGE_BOTH_TEXT,
    skill_badge_user_only_bg => SKILL_BADGE_USER_ONLY_BG,
    skill_badge_user_only_text => SKILL_BADGE_USER_ONLY_TEXT,
    skill_badge_model_only_bg => SKILL_BADGE_MODEL_ONLY_BG,
    skill_badge_model_only_text => SKILL_BADGE_MODEL_ONLY_TEXT,
    skill_badge_disabled_bg => SKILL_BADGE_DISABLED_BG,
    skill_badge_disabled_text => SKILL_BADGE_DISABLED_TEXT,
    skill_aux_chip_bg => SKILL_AUX_CHIP_BG,
    skill_aux_chip_text => SKILL_AUX_CHIP_TEXT,
    skill_section_header_text => SKILL_SECTION_HEADER_TEXT,
    skill_name_text => SKILL_NAME_TEXT,
    skill_meta_text => SKILL_META_TEXT,
    skill_row_hover_bg => SKILL_ROW_HOVER_BG,
    skill_empty_text => SKILL_EMPTY_TEXT,
    mcp_indicator_enabled => MCP_INDICATOR_ENABLED,
    mcp_indicator_disabled => MCP_INDICATOR_DISABLED,
    mcp_indicator_malformed => MCP_INDICATOR_MALFORMED,
    mcp_row_hover_bg => MCP_ROW_HOVER_BG,
    mcp_section_header_text => MCP_SECTION_HEADER_TEXT,
    mcp_row_body_text => MCP_ROW_BODY_TEXT,
    mcp_row_meta_text => MCP_ROW_META_TEXT,
    mcp_empty_text => MCP_EMPTY_TEXT,
    mcp_transport_badge_bg => MCP_TRANSPORT_BADGE_BG,
    mcp_transport_badge_text => MCP_TRANSPORT_BADGE_TEXT,
    mcp_malformed_badge_bg => MCP_MALFORMED_BADGE_BG,
    mcp_malformed_badge_text => MCP_MALFORMED_BADGE_TEXT,
    mcp_disabled_badge_text => MCP_DISABLED_BADGE_TEXT,
}

impl Global for DarudaTheme {}

impl DarudaTheme {
    /// Idempotent installer — registers a `DarudaTheme` Global if
    /// one is not already present. Production entry (`main.rs`) and
    /// test fixtures (`test_support::init_gpui_component`) both call
    /// this; whichever runs first wins, the other is a no-op.
    pub fn init(cx: &mut App) {
        if !cx.has_global::<DarudaTheme>() {
            cx.set_global(DarudaTheme::default());
        }
    }

    /// Parse a `DarudaTheme` from JSON. Missing keys fall through to
    /// the compile-time palette defaults thanks to the struct-level
    /// `#[serde(default)]` — a partial theme file is therefore valid
    /// and merges over the built-in dark palette.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Serialise this theme to JSON. Used by:
    /// - the build-time `daruda_dark.json` generator (so the bundled
    ///   asset stays in lockstep with `palette` const tweaks).
    /// - the Phase-3-D Settings UI's "Export current theme" affordance.
    ///
    /// Pretty-printed for human-friendly diffs.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Return the JSON Schema describing the shape of a daruda theme
    /// file. Used to (re)generate `assets/themes/theme.schema.json`,
    /// which user theme JSON files can reference via
    /// `"$schema": "https://.../theme.schema.json"` so editors that
    /// understand JSON Schema (VS Code, JetBrains, Zed) surface
    /// autocomplete for every slot.
    ///
    /// `gpui::Hsla` already implements `JsonSchema`; the derive
    /// produces one schema entry per field. Field order in the
    /// schema follows the macro's declaration order.
    pub fn json_schema() -> schemars::Schema {
        schemars::schema_for!(DarudaTheme)
    }
}

#[cfg(test)]
mod tests;
