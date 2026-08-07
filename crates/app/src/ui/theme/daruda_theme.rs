//! Runtime container for app-side theme colours.
//!
//! Each `palette::FOO` has a matching snake_case field; defaults copy the
//! compile-time palette, while JSON overrides can replace any subset. The
//! field list macro keeps the struct, defaults, and generated docs in lockstep.

use crate::ui::theme::palette;
use gpui::{App, Global, Hsla};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Generate the struct and defaults from one `field => CONST` list.
/// Kept local so `palette::` resolution stays tied to this module.
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

        impl DarudaTheme {
            /// Return an inactive-pane dim copy, preserving alpha like the
            /// terminal's per-cell dimming.
            pub fn dimmed(&self, amount: f32) -> Self {
                Self {
                    $($field: super::dim_toward_gray(self.$field, amount),)*
                }
            }
        }
    };
}

daruda_theme_fields! {
    title_bar_bg => BG_BASE,
    tab_inactive_bg => BG_PANEL,
    tab_inactive_hover_bg => BG_HOVER,
    border => BORDER,
    overlay_hover => OVERLAY_HOVER,
    overlay_active => OVERLAY_ACTIVE,
    overlay_prominent => OVERLAY_PROMINENT,
    overlay_selected => OVERLAY_SELECTED,
    text_primary => TEXT_PRIMARY,
    text_muted => TEXT_MUTE,
    text_body => TEXT_BODY,
    text_subtle => TEXT_SUBTLE,
    scrollbar_thumb => SCROLLBAR_THUMB,
    status_bar_bg => BG_BASE,
    status_bar_project_dot => STATUS_BAR_PROJECT_DOT,
    status_bar_detached_bg => STATUS_BAR_DETACHED_BG,
    status_bar_detached_text => STATUS_BAR_DETACHED_TEXT,
    status_bar_account_hover_bg => BG_HOVER,
    dock_bg => BG_PANEL,
    button_widget_bg => BG_HOVER,
    button_widget_bg_hover => BG_ACTIVE,
    panel_tab_drop_target_bg => BG_ACTIVE,
    disabled_item_bg => BG_HOVER,
    dock_icon_active_bg => BG_HOVER,
    dock_view_tab_hover_bg => BG_HOVER,
    lane_row_hover_bg => BG_HOVER,
    lane_drop_target_bg => LANE_DROP_TARGET_BG,
    lane_drop_target_rejected_bg => LANE_DROP_TARGET_REJECTED_BG,
    lane_card_bg => BG_PANEL,
    lane_card_hover_bg => BG_HOVER,
    lane_card_active_bg => BG_ACTIVE,
    modal_panel_bg => BG_RAISED,
    modal_input_bg => BG_ACTIVE,
    banner_error_bg => BANNER_ERROR_BG,
    banner_error_text => BANNER_ERROR_TEXT,
    banner_warning_bg => BANNER_WARNING_BG,
    banner_warning_text => BANNER_WARNING_TEXT,
    banner_info_bg => BANNER_INFO_BG,
    banner_info_text => BANNER_INFO_TEXT,
    banner_success_bg => BANNER_SUCCESS_BG,
    banner_success_text => BANNER_SUCCESS_TEXT,
    settings_scrollbar_thumb_hover => SCROLLBAR_THUMB_HOVER,
    right_panel_scrollbar_thumb_hover => SCROLLBAR_THUMB_HOVER,
    task_edit_bg => BG_RAISED,
    task_edit_scrollbar_thumb_hover => SCROLLBAR_THUMB_HOVER,
    settings_sidebar_bg => SETTINGS_SIDEBAR_BG,
    palette_bg => BG_RAISED,
    palette_focused_bg => BG_ACTIVE,
    git_file_row_selected_bg => BG_HOVER,
    git_file_row_hover_bg => BG_HOVER,
    git_diff_panel_bg => BG_EDITOR,
    git_diff_hunk_bg => BG_RAISED,
    git_stage_checkbox_checked_bg => GIT_STAGE_CHECKBOX_CHECKED_BG,
    git_stage_checkbox_unchecked_bg => BG_RAISED,
    input_panel_drop_target_bg => INPUT_PANEL_DROP_TARGET_BG,
    terminal_drop_target_bg => TERMINAL_DROP_TARGET_BG,
    dock_scrollbar_thumb_hover => SCROLLBAR_THUMB_HOVER,
    pane_header_focused_bg => BG_HOVER,
    pane_header_unfocused_bg => BG_PANEL,
    welcome_bg => BG_BASE,
    welcome_button_bg => BG_HOVER,
    welcome_button_hover_bg => BG_ACTIVE,
    welcome_recent_hover_bg => BG_HOVER,
    file_viewer_bg => BG_EDITOR,
    file_diff_add_bg => FILE_DIFF_ADD_BG,
    file_diff_del_bg => FILE_DIFF_DEL_BG,
    file_diff_add_text => FILE_DIFF_ADD_TEXT,
    file_diff_del_text => FILE_DIFF_DEL_TEXT,
    file_diff_hunk_bg => BG_RAISED,
    file_diff_hunk_text => FILE_DIFF_HUNK_TEXT,
    file_viewer_scrollbar_thumb_hover => SCROLLBAR_THUMB_HOVER,
    file_diff_hunk_ctx_text => FILE_DIFF_HUNK_CTX_TEXT,
    file_diff_stat_add => FILE_DIFF_STAT_ADD,
    file_diff_stat_del => FILE_DIFF_STAT_DEL,
    file_diff_word_add_bg => FILE_DIFF_WORD_ADD_BG,
    file_diff_word_del_bg => FILE_DIFF_WORD_DEL_BG,
    file_viewer_search_empty => FILE_VIEWER_SEARCH_EMPTY,
    file_viewer_search_match_bg => FILE_VIEWER_SEARCH_MATCH_BG,
    file_viewer_search_focused_bg => FILE_VIEWER_SEARCH_FOCUSED_BG,
    md_h1_color => INK,
    md_h2_color => MD_H2_COLOR,
    md_h3_color => TEXT_BODY,
    md_h4_color => TEXT_BODY,
    md_code_inline_bg => BG_RAISED,
    md_code_block_bg => BG_PANEL,
    md_footnote_color => MD_FOOTNOTE_COLOR,
    md_table_header_bg => BG_RAISED,
    md_table_row_bg_even => BG_PANEL,
    md_table_row_bg_odd => BG_PANEL,
    toast_action_text => PRIMARY,
    toast_repeat_bg => BG_ACTIVE,
    error_modal_body_bg => BG_BASE,
    terminal_bg => TERMINAL_BG,
    terminal_scrollbar_track_bg => SCROLLBAR_TRACK_BG,
    keystroke_input_bg => BG_PANEL,
    keystroke_input_border_active => PRIMARY,
    keystroke_badge_bg => BG_ACTIVE,
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
    agent_banner_bg => AGENT_BANNER_BG,
    agent_banner_border => AGENT_BANNER_BORDER,
    agent_banner_hover_bg => AGENT_BANNER_HOVER_BG,
    agent_banner_icon => AGENT_BANNER_ICON,
    right_panel_task_running_color => SIGNAL_GREEN,
    task_edit_branch_invalid_border => ERROR,
    badge_bg => BG_HOVER,
    gauge_track_bg => BG_HOVER,
    skill_aux_chip_bg => SKILL_AUX_CHIP_BG,
    skill_row_hover_bg => BG_HOVER,
    mcp_indicator_malformed => MCP_INDICATOR_MALFORMED,
    mcp_malformed_badge_bg => MCP_MALFORMED_BADGE_BG,
    mcp_malformed_badge_text => MCP_MALFORMED_BADGE_TEXT,
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

    /// Whether this is a dark theme, judged by the base background's
    /// lightness. Theme-agnostic (works for custom presets), used to pick a
    /// matching diagram theme so rendered content (e.g. mermaid) stays legible
    /// on the current surface.
    pub fn is_dark(&self) -> bool {
        self.title_bar_bg.l < 0.5
    }

    /// Parse a `DarudaTheme` from JSON. Missing keys fall through to
    /// the compile-time palette defaults thanks to the struct-level
    /// `#[serde(default)]` — a partial theme file is therefore valid
    /// and merges over the built-in dark palette.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Serialise this theme to JSON. Used by the build-time
    /// `daruda_dark.json` generator so the bundled asset stays in
    /// lockstep with `palette` const tweaks.
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
