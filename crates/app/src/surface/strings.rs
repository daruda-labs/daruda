//! Menu labels and any other app-shell display text.
//!
//! Localisation hook-point for the menu bar. Terminal-view text
//! (search overlay, fallback title) is in
//! `daruda_terminal::ux::strings`; this file is only for the app's
//! chrome.

use super::constants::APP_NAME;

// ============================================================================
// Top-level menu names
// ============================================================================

pub fn menu_app() -> &'static str {
    APP_NAME
}
pub fn menu_file() -> String {
    rust_i18n::t!("menu.file").into_owned()
}
pub fn menu_view() -> String {
    rust_i18n::t!("menu.view").into_owned()
}
pub fn menu_edit() -> String {
    rust_i18n::t!("menu.edit").into_owned()
}
pub fn menu_worktree() -> String {
    rust_i18n::t!("menu.worktree").into_owned()
}
pub fn menu_window() -> String {
    rust_i18n::t!("menu.window").into_owned()
}
pub fn menu_help() -> String {
    rust_i18n::t!("menu.help").into_owned()
}

// ============================================================================
// App menu
// ============================================================================

pub fn menu_quit_app() -> String {
    rust_i18n::t!("menu.quit_app").into_owned()
}
pub fn menu_services() -> String {
    rust_i18n::t!("menu.services").into_owned()
}
pub fn menu_settings() -> String {
    rust_i18n::t!("menu.settings").into_owned()
}
pub fn menu_open_project_config() -> String {
    rust_i18n::t!("menu.open_project_config").into_owned()
}

// ============================================================================
// File menu
// ============================================================================

pub fn menu_new_window() -> String {
    rust_i18n::t!("menu.new_window").into_owned()
}
pub fn menu_open() -> String {
    rust_i18n::t!("menu.open").into_owned()
}
pub fn menu_open_in_new_window() -> String {
    rust_i18n::t!("menu.open_in_new_window").into_owned()
}
pub fn menu_open_recent() -> String {
    rust_i18n::t!("menu.open_recent").into_owned()
}
pub fn menu_open_recent_in_new_window() -> String {
    rust_i18n::t!("menu.open_recent_in_new_window").into_owned()
}
pub fn menu_close_project() -> String {
    rust_i18n::t!("menu.close_project").into_owned()
}
pub fn menu_no_recent() -> String {
    rust_i18n::t!("menu.no_recent").into_owned()
}
pub fn menu_new_tab() -> String {
    rust_i18n::t!("common.new_tab").into_owned()
}
pub fn menu_close_pane() -> String {
    rust_i18n::t!("common.close_pane").into_owned()
}
pub fn menu_close_tab() -> String {
    rust_i18n::t!("common.close_tab").into_owned()
}

// ============================================================================
// View menu
// ============================================================================

pub fn menu_split_right() -> String {
    rust_i18n::t!("common.split_right").into_owned()
}
pub fn menu_split_down() -> String {
    rust_i18n::t!("common.split_down").into_owned()
}
pub fn menu_next_pane() -> String {
    rust_i18n::t!("menu.next_pane").into_owned()
}
pub fn menu_prev_pane() -> String {
    rust_i18n::t!("menu.prev_pane").into_owned()
}
pub fn menu_focus_pane_left() -> String {
    rust_i18n::t!("menu.focus_pane_left").into_owned()
}
pub fn menu_focus_pane_right() -> String {
    rust_i18n::t!("menu.focus_pane_right").into_owned()
}
pub fn menu_focus_pane_up() -> String {
    rust_i18n::t!("menu.focus_pane_up").into_owned()
}
pub fn menu_focus_pane_down() -> String {
    rust_i18n::t!("menu.focus_pane_down").into_owned()
}
pub fn menu_move_tab_left() -> String {
    rust_i18n::t!("common.move_tab_left").into_owned()
}
pub fn menu_move_tab_right() -> String {
    rust_i18n::t!("common.move_tab_right").into_owned()
}

// ============================================================================
// Edit menu
// ============================================================================

pub fn menu_copy() -> String {
    rust_i18n::t!("menu.copy").into_owned()
}
pub fn menu_paste() -> String {
    rust_i18n::t!("menu.paste").into_owned()
}
pub fn menu_select_all() -> String {
    rust_i18n::t!("menu.select_all").into_owned()
}
pub fn menu_find() -> String {
    rust_i18n::t!("menu.find").into_owned()
}
pub fn menu_find_next() -> String {
    rust_i18n::t!("menu.find_next").into_owned()
}
pub fn menu_find_prev() -> String {
    rust_i18n::t!("menu.find_prev").into_owned()
}
pub fn menu_clear_buffer() -> String {
    rust_i18n::t!("menu.clear_buffer").into_owned()
}
pub fn menu_clear_scrollback() -> String {
    rust_i18n::t!("menu.clear_scrollback").into_owned()
}

// ============================================================================
// View menu (additions)
// ============================================================================

pub fn menu_toggle_full_screen() -> String {
    rust_i18n::t!("menu.toggle_full_screen").into_owned()
}
pub fn menu_toggle_left_dock() -> String {
    rust_i18n::t!("menu.toggle_left_dock").into_owned()
}
pub fn menu_toggle_bottom_dock() -> String {
    rust_i18n::t!("menu.toggle_bottom_dock").into_owned()
}
pub fn menu_toggle_right_dock() -> String {
    rust_i18n::t!("menu.toggle_right_dock").into_owned()
}
pub fn menu_jump_prompt_prev() -> String {
    rust_i18n::t!("menu.jump_prompt_prev").into_owned()
}
pub fn menu_jump_prompt_next() -> String {
    rust_i18n::t!("menu.jump_prompt_next").into_owned()
}

// ============================================================================
// Lane menu
// ============================================================================

pub fn menu_activate_lane_1() -> String {
    rust_i18n::t!("menu.activate_lane_1").into_owned()
}
pub fn menu_activate_lane_2() -> String {
    rust_i18n::t!("menu.activate_lane_2").into_owned()
}
pub fn menu_activate_lane_3() -> String {
    rust_i18n::t!("menu.activate_lane_3").into_owned()
}
pub fn menu_activate_lane_4() -> String {
    rust_i18n::t!("menu.activate_lane_4").into_owned()
}
pub fn menu_activate_lane_5() -> String {
    rust_i18n::t!("menu.activate_lane_5").into_owned()
}
pub fn menu_activate_lane_6() -> String {
    rust_i18n::t!("menu.activate_lane_6").into_owned()
}
pub fn menu_activate_lane_7() -> String {
    rust_i18n::t!("menu.activate_lane_7").into_owned()
}
pub fn menu_activate_lane_8() -> String {
    rust_i18n::t!("menu.activate_lane_8").into_owned()
}
pub fn menu_activate_lane_9() -> String {
    rust_i18n::t!("menu.activate_lane_9").into_owned()
}

// ============================================================================
// Window menu
// ============================================================================

pub fn menu_minimize() -> String {
    rust_i18n::t!("menu.minimize").into_owned()
}
pub fn menu_zoom() -> String {
    rust_i18n::t!("menu.zoom").into_owned()
}
pub fn menu_edit_window_title() -> String {
    rust_i18n::t!("menu.edit_window_title").into_owned()
}

pub fn edit_window_title_modal_title() -> String {
    rust_i18n::t!("modal.edit_window_title_title").into_owned()
}
pub fn edit_window_title_placeholder() -> String {
    rust_i18n::t!("modal.edit_window_title_placeholder").into_owned()
}

// ============================================================================
// Help menu
// ============================================================================

pub fn menu_daruda_help() -> String {
    rust_i18n::t!("menu.daruda_help").into_owned()
}
pub fn menu_keyboard_shortcuts() -> String {
    rust_i18n::t!("menu.keyboard_shortcuts").into_owned()
}
pub fn menu_edit_keymap() -> String {
    rust_i18n::t!("menu.edit_keymap").into_owned()
}
pub fn menu_report_issue() -> String {
    rust_i18n::t!("menu.report_issue").into_owned()
}
pub fn menu_github_repo() -> String {
    rust_i18n::t!("menu.github_repo").into_owned()
}

// ============================================================================
// External URLs (Help menu targets)
// ============================================================================

pub const URL_GITHUB_REPO: &str = "https://github.com/daruda-ai/daruda";
pub const URL_REPORT_ISSUE: &str = "https://github.com/daruda-ai/daruda/issues/new";
pub const URL_HELP: &str = "https://github.com/daruda-ai/daruda#readme";

// ============================================================================
// Dock panel labels — kept as &'static str because panel_name() -> &'static str
// ============================================================================

pub const DOCK_PANEL_AGENT_TASKS: &str = "Agent Tasks";
pub const DOCK_PANEL_FILES: &str = "Files";
pub const DOCK_PANEL_GIT: &str = "Git";
pub const DOCK_PANEL_MACROS: &str = "Macros";
pub const DOCK_PANEL_OUTPUT: &str = "Output";
pub const DOCK_PANEL_WORKTREES: &str = "Projects";

// ============================================================================
// Right panel tab labels
// ============================================================================

pub fn right_panel_tab_usage() -> String {
    rust_i18n::t!("dock.right_tab_usage").into_owned()
}
pub fn right_panel_tab_skills() -> String {
    rust_i18n::t!("dock.right_tab_skills").into_owned()
}
pub fn right_panel_tab_tools() -> String {
    rust_i18n::t!("dock.right_tab_tools").into_owned()
}
pub fn right_panel_tab_tasks() -> String {
    rust_i18n::t!("dock.right_tab_tasks").into_owned()
}

// ============================================================================
// Dock (left dock) tab labels
// ============================================================================

pub fn sidebar_tab_worktrees() -> String {
    rust_i18n::t!("dock.sidebar_tab_worktrees").into_owned()
}
pub fn sidebar_tab_git() -> String {
    rust_i18n::t!("dock.sidebar_tab_git").into_owned()
}
pub fn sidebar_tab_files() -> String {
    rust_i18n::t!("dock.sidebar_tab_files").into_owned()
}

// ============================================================================
// Right panel — Tasks tab labels
// ============================================================================

/// Filter dropdown labels.
pub fn task_filter_all() -> String {
    rust_i18n::t!("task.filter_all").into_owned()
}
pub fn task_filter_backlog() -> String {
    rust_i18n::t!("task.filter_backlog").into_owned()
}
pub fn task_filter_running() -> String {
    rust_i18n::t!("task.filter_running").into_owned()
}
pub fn task_filter_done() -> String {
    rust_i18n::t!("task.filter_done").into_owned()
}

/// `[+ New]` button label.
pub fn task_new_button() -> String {
    rust_i18n::t!("task.new_button").into_owned()
}

/// Tasks tab search bar — substring filter over title / prompt /
/// notes / branch_name. Placement and behaviour mirror the Skills tab
/// search (`SKILLS_SEARCH_*`). Cleared via the in-field `✕` overlay.
pub fn task_search_placeholder() -> String {
    rust_i18n::t!("task.search_placeholder").into_owned()
}
pub fn task_search_empty_prefix() -> String {
    rust_i18n::t!("task.search_empty_prefix").into_owned()
}
pub const TASK_SEARCH_CLEAR_ICON: &str = "✕";

/// Action labels rendered inside the status-pill dropdown. The pill
/// itself shows the task's current state; the dropdown lists the
/// transitions and meta actions valid for that state.
pub fn task_action_start() -> String {
    rust_i18n::t!("task.action_start").into_owned()
}
pub fn task_action_open() -> String {
    rust_i18n::t!("task.action_open").into_owned()
}
pub fn task_action_stop() -> String {
    rust_i18n::t!("task.action_stop").into_owned()
}
pub fn task_action_delete() -> String {
    rust_i18n::t!("common.btn_delete").into_owned()
}
pub fn task_action_reopen() -> String {
    rust_i18n::t!("task.action_reopen").into_owned()
}
pub fn task_action_retry() -> String {
    rust_i18n::t!("task.action_retry").into_owned()
}
pub fn task_action_edit() -> String {
    rust_i18n::t!("common.btn_edit").into_owned()
}
/// `View error` (Error state). Opens an OK-only alert dialog
/// showing the full `TaskState::Error.message` so users can see the
/// truncated row text in full.
pub fn task_action_view_error() -> String {
    rust_i18n::t!("task.action_view_error").into_owned()
}
/// Title shown on the alert dialog opened by `View error`.
pub fn task_error_dialog_title() -> String {
    rust_i18n::t!("task.error_dialog_title").into_owned()
}
/// OK-button label on the View error alert dialog.
pub fn task_error_dialog_close() -> String {
    rust_i18n::t!("common.btn_close").into_owned()
}

// Task picker modal — command-palette dispatch titles. Read as
// "Start Task", "Cancel Task", … so the user knows which action they
// are committing to before picking a target.
pub fn task_picker_title_start() -> String {
    rust_i18n::t!("task.picker_title_start").into_owned()
}
pub fn task_picker_title_cancel() -> String {
    rust_i18n::t!("task.picker_title_cancel").into_owned()
}
pub fn task_picker_title_reopen() -> String {
    rust_i18n::t!("task.picker_title_reopen").into_owned()
}
pub fn task_picker_title_retry() -> String {
    rust_i18n::t!("task.picker_title_retry").into_owned()
}
pub fn task_picker_title_delete() -> String {
    rust_i18n::t!("task.picker_title_delete").into_owned()
}
pub fn task_picker_title_edit() -> String {
    rust_i18n::t!("task.picker_title_edit").into_owned()
}

/// Body of the delete-task confirmation dialog. `title` is the task's
/// display title, interpolated inside curly quotes.
pub fn task_confirm_delete_body(title: &str) -> String {
    rust_i18n::t!("task.confirm_delete_body", title => title).into_owned()
}

// Task picker modal — empty-state messages shown when no task is
// eligible for the chosen action.
pub fn task_picker_empty_start() -> String {
    rust_i18n::t!("task.picker_empty_start").into_owned()
}
pub fn task_picker_empty_cancel() -> String {
    rust_i18n::t!("task.picker_empty_cancel").into_owned()
}
pub fn task_picker_empty_edit() -> String {
    rust_i18n::t!("task.picker_empty_edit").into_owned()
}
pub fn task_picker_empty_reopen() -> String {
    rust_i18n::t!("task.picker_empty_reopen").into_owned()
}
pub fn task_picker_empty_retry() -> String {
    rust_i18n::t!("task.picker_empty_retry").into_owned()
}
pub fn task_picker_empty_delete() -> String {
    rust_i18n::t!("task.picker_empty_delete").into_owned()
}

/// `[📄 Open file]` button shown next to the Prompt section header in
/// the TaskEdit pane. Click opens
/// `<wt>/.daruda/task-<branch>.md` in a fresh file viewer tab when
/// the task has a lane (Backlog / draft tasks disable the button
/// since no on-disk file exists yet).
pub fn task_edit_open_file_button() -> String {
    rust_i18n::t!("task.edit_open_file_button").into_owned()
}

/// Subtext rendered directly under the "Notes" label in the TaskEdit
/// pane. Notes are stored on the `Task` and surface in search, but
/// `render_task_prompt` deliberately excludes them — the hint makes
/// that contract visible so users don't expect the agent to read
/// their journal. Kept agent-agnostic since `AgentType` is reserved
/// for future codex / gemini / copilot expansion.
pub fn task_edit_notes_hint() -> String {
    rust_i18n::t!("task.edit_notes_hint").into_owned()
}

/// Field label for the base-lane selector on the TaskEdit pane.
pub fn task_edit_base_label() -> String {
    rust_i18n::t!("task.edit_base_label").into_owned()
}

/// Trailing suffix on the option that maps to "no explicit base
/// lane — fall through to the project's active lane at
/// start_task time". Used as both the placeholder and the
/// first-option label so empty drafts read as "Active lane"
/// rather than blank.
pub fn task_edit_base_active_label() -> String {
    rust_i18n::t!("task.edit_base_active_label").into_owned()
}

/// Field labels on the TaskEdit pane form. Title / Branch / Prompt /
/// Notes are generic form-field labels routed through `common.field_*`
/// (the same shared-token pattern as `skills_field_name` →
/// `common.field_name`); the domain wrapper name is kept so call sites
/// don't change. Auto-execute stays task-specific.
pub fn task_edit_title_label() -> String {
    rust_i18n::t!("common.field_title").into_owned()
}
pub fn task_edit_branch_label() -> String {
    rust_i18n::t!("common.field_branch").into_owned()
}
pub fn task_edit_prompt_label() -> String {
    rust_i18n::t!("common.field_prompt").into_owned()
}
pub fn task_edit_notes_label() -> String {
    rust_i18n::t!("common.field_notes").into_owned()
}
pub fn task_edit_auto_execute_label() -> String {
    rust_i18n::t!("task.edit_auto_execute_label").into_owned()
}

/// Field label for the execution-surface selector on the TaskEdit pane.
pub fn task_edit_surface_label() -> String {
    rust_i18n::t!("task.edit_surface_label").into_owned()
}
/// Option label — run the task on a terminal CLI session.
pub fn task_edit_surface_terminal() -> String {
    rust_i18n::t!("task.edit_surface_terminal").into_owned()
}
/// Option label — run the task on an in-app Agent chat (ACP) session.
pub fn task_edit_surface_agent_chat() -> String {
    rust_i18n::t!("task.edit_surface_agent_chat").into_owned()
}

/// Glyph appended to the status pill label as the dropdown chevron.
/// Leading space provides the visual gap between the label and the
/// triangle.
pub const TASK_PILL_CHEVRON: &str = " ▾";

/// TaskEdit pane close prompt — copy mirrors Zed `pane.rs:1981-1998`.
/// Draft branch uses a stronger "Discard new task?" heading since the
/// work has never been persisted.
pub fn task_edit_save_prompt_prefix() -> String {
    rust_i18n::t!("task.edit_save_prompt_prefix").into_owned()
}
pub fn task_edit_save_prompt_suffix() -> String {
    rust_i18n::t!("task.edit_save_prompt_suffix").into_owned()
}
pub fn task_edit_discard_draft_prompt() -> String {
    rust_i18n::t!("task.edit_discard_draft_prompt").into_owned()
}
pub fn task_edit_save() -> String {
    rust_i18n::t!("common.btn_save").into_owned()
}
pub fn task_edit_save_draft() -> String {
    rust_i18n::t!("task.edit_save_draft").into_owned()
}
pub fn task_edit_discard() -> String {
    rust_i18n::t!("task.edit_discard").into_owned()
}
pub fn task_edit_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}

/// Tab-title dirty indicator. Painted before the title with a
/// trailing space so titles align across dirty / clean tabs.
pub const TAB_TITLE_DIRTY_DOT: &str = "● ";

/// Prompt-file watcher conflict prompt. Fires when an
/// external editor rewrites `<wt>/.daruda/task-<branch>.md` and the
/// pane already has unsaved edits.
pub const PROMPT_WATCHER_HEADING_PREFIX: &str = "“";
pub fn prompt_watcher_heading_suffix() -> String {
    rust_i18n::t!("task.watcher_heading_suffix").into_owned()
}
pub fn prompt_watcher_detail() -> String {
    rust_i18n::t!("task.watcher_detail").into_owned()
}
pub fn prompt_watcher_use_disk() -> String {
    rust_i18n::t!("task.watcher_use_disk").into_owned()
}
pub fn prompt_watcher_keep_mine() -> String {
    rust_i18n::t!("task.watcher_keep_mine").into_owned()
}
pub fn prompt_watcher_diff() -> String {
    rust_i18n::t!("task.watcher_diff").into_owned()
}

/// Tab / window batch close prompt. Single 3-button modal
/// summarising every dirty TaskEdit pane in the closing scope.
pub fn tab_close_batch_heading() -> String {
    rust_i18n::t!("task.batch_close_heading").into_owned()
}
pub fn tab_close_batch_save_all() -> String {
    rust_i18n::t!("task.batch_save_all").into_owned()
}
pub fn tab_close_batch_discard_all() -> String {
    rust_i18n::t!("task.batch_discard_all").into_owned()
}

/// Title for the toast surfaced when one or more panes in a Save-all
/// batch fail to commit because their branch input is invalid. The
/// detail message lists the affected task titles.
pub fn task_batch_save_failed_title() -> String {
    rust_i18n::t!("task.batch_save_failed_title").into_owned()
}

/// Done end-reason flavour appended inline as `Done (<flavour>)`.
pub fn task_done_flavour_stop() -> String {
    rust_i18n::t!("task.done_flavour_stop").into_owned()
}
pub fn task_done_flavour_prompt_input_exit() -> String {
    rust_i18n::t!("task.done_flavour_prompt_input_exit").into_owned()
}
pub fn task_done_flavour_logout() -> String {
    rust_i18n::t!("task.done_flavour_logout").into_owned()
}
pub fn task_done_flavour_other() -> String {
    rust_i18n::t!("task.done_flavour_other").into_owned()
}

/// Subtask UI strings. The section title is suffixed with the
/// `(done/total done)` counter at render time; the auto/manual labels
/// surface the `source_session_id` namespace split.
pub fn task_subtask_section_title() -> String {
    rust_i18n::t!("task.subtask_section_title").into_owned()
}
pub fn task_subtask_progress_suffix() -> String {
    rust_i18n::t!("task.subtask_progress_suffix").into_owned()
}
pub fn task_subtask_add_placeholder() -> String {
    rust_i18n::t!("task.subtask_add_placeholder").into_owned()
}
pub fn task_subtask_draft_hint() -> String {
    rust_i18n::t!("task.subtask_draft_hint").into_owned()
}
pub fn task_subtask_auto_label() -> String {
    rust_i18n::t!("task.subtask_auto_label").into_owned()
}
pub fn task_subtask_manual_label() -> String {
    rust_i18n::t!("task.subtask_manual_label").into_owned()
}

// ============================================================================
// Right panel — Usage tab labels
// ============================================================================

// ----------------------------------------------------------------
// Plan-limit gauges
// ----------------------------------------------------------------

/// Header title for the Usage tab (product brand — same in all locales).
pub fn usage_brand_title() -> String {
    rust_i18n::t!("usage.brand_title").into_owned()
}
/// Section heading rendered above the plan-limit gauge block.
pub fn usage_limits_section_label() -> String {
    rust_i18n::t!("usage.limits_section_label").into_owned()
}

/// Label for the 5-hour rolling window gauge.
pub fn usage_limit_5h_label() -> String {
    rust_i18n::t!("usage.limit_5h_label").into_owned()
}
/// Label for the 7-day rolling window gauge.
pub fn usage_limit_7d_label() -> String {
    rust_i18n::t!("usage.limit_7d_label").into_owned()
}
/// Label for the 7-day Opus-scoped window gauge. Only rendered when
/// the plan meters Opus separately (`seven_day_opus` is present).
pub fn usage_limit_opus_label() -> String {
    rust_i18n::t!("usage.limit_opus_label").into_owned()
}
/// Placeholder label for either gauge when the OAuth token is
/// unavailable, the API call failed, or the window is missing from
/// the response.
pub fn usage_limit_unavailable() -> String {
    rust_i18n::t!("usage.limit_unavailable").into_owned()
}

// ----------------------------------------------------------------
// Activity dashboard (today's stats + 7-day chart + totals)
// ----------------------------------------------------------------

/// Section heading above the three "today's activity" stat cards.
pub fn usage_section_today() -> String {
    rust_i18n::t!("usage.section_today").into_owned()
}
/// Section heading above the 7-day bar chart.
pub fn usage_section_7day() -> String {
    rust_i18n::t!("usage.section_7day").into_owned()
}
/// Section heading above the all-time totals row.
pub fn usage_section_total() -> String {
    rust_i18n::t!("usage.section_total").into_owned()
}
/// "Messages" stat-card label.
pub fn usage_stat_messages() -> String {
    rust_i18n::t!("usage.stat_messages").into_owned()
}
/// "Sessions" stat-card label.
pub fn usage_stat_sessions() -> String {
    rust_i18n::t!("usage.stat_sessions").into_owned()
}
/// "Tool Calls" stat-card label.
pub fn usage_stat_tool_calls() -> String {
    rust_i18n::t!("usage.stat_tool_calls").into_owned()
}
/// "Total Messages" totals-row label.
pub fn usage_total_messages() -> String {
    rust_i18n::t!("usage.total_messages").into_owned()
}
/// "Total Sessions" totals-row label.
pub fn usage_total_sessions() -> String {
    rust_i18n::t!("usage.total_sessions").into_owned()
}
/// "Active Days" totals-row label.
pub fn usage_total_active_days() -> String {
    rust_i18n::t!("usage.total_active_days").into_owned()
}
/// Refresh-badge label before any fetch has landed (no cache age to
/// show yet). Carries its own `↻` glyph.
pub fn usage_refresh() -> String {
    rust_i18n::t!("usage.refresh").into_owned()
}
/// Refresh-badge label while a manual refresh is in flight.
pub fn usage_refreshing() -> String {
    rust_i18n::t!("usage.refreshing").into_owned()
}
/// Cache-age badge for a refresh less than a minute ago.
pub fn usage_cache_just_now() -> String {
    rust_i18n::t!("usage.cache_just_now").into_owned()
}
/// Cache-age badge, `n` whole minutes since the last refresh.
pub fn usage_cache_minutes(n: u64) -> String {
    rust_i18n::t!("usage.cache_minutes", n => n).into_owned()
}
/// Cache-age badge, `n` whole hours since the last refresh.
pub fn usage_cache_hours(n: u64) -> String {
    rust_i18n::t!("usage.cache_hours", n => n).into_owned()
}
/// Cache-age badge, `n` whole days since the last refresh.
pub fn usage_cache_days(n: u64) -> String {
    rust_i18n::t!("usage.cache_days", n => n).into_owned()
}
/// Weekday abbreviation for a chart bar. `idx` is 0=Sunday .. 6=Saturday
/// (matching `chrono::Weekday::num_days_from_sunday`). Out-of-range
/// values fall back to an empty label rather than panicking.
pub fn usage_weekday_label(idx: u8) -> String {
    match idx {
        0 => rust_i18n::t!("usage.weekday_sun").into_owned(),
        1 => rust_i18n::t!("usage.weekday_mon").into_owned(),
        2 => rust_i18n::t!("usage.weekday_tue").into_owned(),
        3 => rust_i18n::t!("usage.weekday_wed").into_owned(),
        4 => rust_i18n::t!("usage.weekday_thu").into_owned(),
        5 => rust_i18n::t!("usage.weekday_fri").into_owned(),
        6 => rust_i18n::t!("usage.weekday_sat").into_owned(),
        _ => String::new(),
    }
}

/// Format a "resets in …" countdown for the gauge subtitle. Lives
/// here (not on `Duration`) so the precise rounding behaviour is
/// covered by tests next to the rest of the surface strings.
///
/// Buckets:
/// - `≥ 1 day` → `"<d>d <h>h"`
/// - `≥ 1 hour` → `"<h>h <m>m"`
/// - `≥ 1 minute` → `"<m>m"`
/// - `1..60 seconds` → `"<1m"` (avoids the awkward "Resets in 0m"
///   that would linger for a whole minute before the reset)
/// - `0` → `"now"`
///
/// Always prefixed with `"Resets in "` so callers concatenate a
/// single string and don't have to reason about pluralization.
pub fn format_reset_countdown(remaining: std::time::Duration) -> String {
    let secs = remaining.as_secs();
    if secs == 0 {
        return "Resets now".to_string();
    }
    let mins_total = secs / 60;
    let hours_total = mins_total / 60;
    let days_total = hours_total / 24;
    if days_total >= 1 {
        format!("Resets in {}d {}h", days_total, hours_total % 24)
    } else if hours_total >= 1 {
        format!("Resets in {}h {}m", hours_total, mins_total % 60)
    } else if mins_total >= 1 {
        format!("Resets in {}m", mins_total)
    } else {
        "Resets in <1m".to_string()
    }
}

// ----------------------------------------------------------------
// Service status pill
// ----------------------------------------------------------------

/// Label shown on the green pill when Anthropic Statuspage reports
/// "operational". The Übersicht widget unconditionally hides the
/// upstream description on the green path because Statuspage tends
/// to leave stale "All systems normal" descriptions; daruda mirrors
/// that behavior.
pub fn status_label_operational() -> String {
    rust_i18n::t!("status.operational").into_owned()
}
/// Default label for `minor` indicator when the response carries no
/// description (rare but possible).
pub fn status_label_minor_default() -> String {
    rust_i18n::t!("status.minor_default").into_owned()
}
/// Default label for `major` indicator when the response carries no
/// description.
pub fn status_label_major_default() -> String {
    rust_i18n::t!("status.major_default").into_owned()
}
/// Default label for `critical` indicator when the response carries
/// no description.
pub fn status_label_critical_default() -> String {
    rust_i18n::t!("status.critical_default").into_owned()
}
/// Label shown when the indicator is `Unknown` (parse miss or
/// before-first-fetch). Distinct from operational so the renderer
/// can dim the pill instead of pretending green.
pub fn status_label_unknown() -> String {
    rust_i18n::t!("status.unknown").into_owned()
}

/// Resolve the user-visible label for a service-status snapshot.
///
/// Mirrors the Übersicht widget rule: `None` ignores the upstream
/// description (always show the hard-coded "Operational"), other
/// indicators prefer the upstream description and fall back to a
/// canned default when it's empty. `Unknown` ignores description
/// entirely — a parse miss shouldn't surface garbage strings.
pub fn service_status_label(status: &daruda_claude::ServiceStatus) -> String {
    use daruda_claude::StatusIndicator;
    match status.indicator {
        StatusIndicator::None => status_label_operational().to_string(),
        StatusIndicator::Unknown => status_label_unknown().to_string(),
        StatusIndicator::Minor => {
            if status.description.is_empty() {
                status_label_minor_default().to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Major => {
            if status.description.is_empty() {
                status_label_major_default().to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Critical => {
            if status.description.is_empty() {
                status_label_critical_default().to_string()
            } else {
                status.description.clone()
            }
        }
    }
}

// ============================================================================
// Welcome screen
// ============================================================================

pub fn welcome_title() -> String {
    rust_i18n::t!("welcome.title").into_owned()
}
pub const WELCOME_VERSION: &str = "v0.1.0";
pub fn welcome_open_folder() -> String {
    rust_i18n::t!("welcome.open_folder").into_owned()
}
pub fn welcome_recent() -> String {
    rust_i18n::t!("welcome.recent").into_owned()
}
pub fn welcome_new_empty() -> String {
    rust_i18n::t!("welcome.new_empty").into_owned()
}
pub fn welcome_no_recent() -> String {
    rust_i18n::t!("welcome.no_recent").into_owned()
}

/// Short changelog line shown at the bottom of the welcome panel.
/// Announces the post-multi-project shortcut semantics — `Cmd+O` now
/// adds the project to the current window (policy-aware) instead of
/// spawning a new one. Plain text; no Markdown rendering.
pub fn welcome_changelog_open_policy() -> String {
    rust_i18n::t!("welcome.changelog_open_policy").into_owned()
}

// ============================================================================
// File viewer (pane-area viewer opened from Git Changes dock)
// ============================================================================

pub fn file_viewer_loading() -> String {
    rust_i18n::t!("common.loading").into_owned()
}
pub fn file_viewer_binary() -> String {
    rust_i18n::t!("file_viewer.binary").into_owned()
}
pub fn file_viewer_deleted() -> String {
    rust_i18n::t!("file_viewer.deleted").into_owned()
}
pub fn file_viewer_empty_diff() -> String {
    rust_i18n::t!("file_viewer.empty_diff").into_owned()
}
pub fn file_viewer_staged_badge() -> String {
    rust_i18n::t!("file_viewer.staged_badge").into_owned()
}
pub const FILE_VIEWER_PATH_SEP: &str = std::path::MAIN_SEPARATOR_STR;
pub fn file_viewer_tab_raw() -> String {
    rust_i18n::t!("file_viewer.tab_raw").into_owned()
}
pub fn file_viewer_tab_preview() -> String {
    rust_i18n::t!("file_viewer.tab_preview").into_owned()
}
pub fn file_viewer_tab_changes() -> String {
    rust_i18n::t!("file_viewer.tab_changes").into_owned()
}
pub fn file_viewer_show_all() -> String {
    rust_i18n::t!("file_viewer.show_all").into_owned()
}
pub fn file_viewer_hide_unchanged() -> String {
    rust_i18n::t!("file_viewer.hide_unchanged").into_owned()
}
pub const FILE_VIEWER_CLOSE: &str = "×";
pub fn file_viewer_no_newline() -> String {
    rust_i18n::t!("file_viewer.no_newline").into_owned()
}
/// Tooltip on the rendered-markdown code-block copy button (idle state).
pub fn code_block_copy() -> String {
    rust_i18n::t!("code_block.copy").into_owned()
}

/// Tooltip on the rendered-markdown code-block copy button after a copy.
pub fn code_block_copied() -> String {
    rust_i18n::t!("code_block.copied").into_owned()
}

pub fn file_viewer_copy_abs_path() -> String {
    rust_i18n::t!("file_viewer.copy_abs_path").into_owned()
}
pub fn file_viewer_copy_rel_path() -> String {
    rust_i18n::t!("file_viewer.copy_rel_path").into_owned()
}

pub fn file_viewer_more_lines(count: usize) -> String {
    format!("… ({count} more lines)")
}

pub fn file_viewer_byte_truncated(shown: usize, max_bytes: usize, total_count: usize) -> String {
    let size = if max_bytes >= 1024 * 1024 {
        format!("{} MB", max_bytes / (1024 * 1024))
    } else {
        format!("{} KB", max_bytes / 1024)
    };
    if total_count > shown {
        format!("File exceeds {size} — showing first {shown} of {total_count}+ lines")
    } else {
        format!("File exceeds {size} — showing first {shown} lines")
    }
}

pub fn file_viewer_search_placeholder() -> String {
    rust_i18n::t!("file_viewer.search_placeholder").into_owned()
}

// ============================================================================
// Agent chat pane (ACP)
// ============================================================================

/// Tab / pane-header title for an Agent chat pane.
pub fn agent_chat_tab_title() -> String {
    rust_i18n::t!("agent_chat.tab_title").into_owned()
}

/// Banner copy for a dormant (restored, not-yet-connected) Agent chat pane.
/// The session starts on first focus, so this shows only for a visible but
/// unfocused pane.
pub fn agent_chat_idle() -> String {
    rust_i18n::t!("agent_chat.idle").into_owned()
}

/// Placeholder copy shown while the agent session is connecting, before any
/// handshake milestone has been reported yet.
pub fn agent_chat_connecting() -> String {
    rust_i18n::t!("agent_chat.connecting").into_owned()
}

/// Status banner shown while `initialize` is in flight (awaiting the agent's
/// capabilities reply).
pub fn agent_chat_connecting_handshake() -> String {
    rust_i18n::t!("agent_chat.connecting_handshake").into_owned()
}

/// Status banner shown while `session/new` is in flight (fresh session).
pub fn agent_chat_connecting_creating_session() -> String {
    rust_i18n::t!("agent_chat.connecting_creating_session").into_owned()
}

/// Status banner shown while `session/load` is in flight (resuming a
/// persisted session).
pub fn agent_chat_connecting_loading_session() -> String {
    rust_i18n::t!("agent_chat.connecting_loading_session").into_owned()
}

/// Status banner shown while `session/set_mode` is in flight, applying the
/// configured initial mode to a freshly created session.
pub fn agent_chat_connecting_applying_mode() -> String {
    rust_i18n::t!("agent_chat.connecting_applying_mode").into_owned()
}

/// Status banner shown while the app is downloading the Node.js runtime the
/// agent adapter needs (first run on a machine without a usable Node.js).
pub fn agent_chat_runtime_downloading() -> String {
    rust_i18n::t!("agent_chat.runtime_downloading").into_owned()
}

/// Status banner shown while the downloaded Node.js runtime is being verified.
pub fn agent_chat_runtime_verifying() -> String {
    rust_i18n::t!("agent_chat.runtime_verifying").into_owned()
}

/// Status banner shown while the downloaded Node.js runtime is being extracted.
pub fn agent_chat_runtime_extracting() -> String {
    rust_i18n::t!("agent_chat.runtime_extracting").into_owned()
}

/// Status-line reason shown when an Agent chat pane has no resolvable lane
/// working directory to attach a session to. The renderer prepends the error
/// prefix, so this is the bare reason (not itself prefixed).
pub fn agent_chat_no_lane_cwd() -> String {
    rust_i18n::t!("agent_chat.no_lane_cwd").into_owned()
}

/// Status-line reason shown when a fresh Agent chat pane's agent command
/// needs a remote working directory (`{{cwd}}` token — see
/// `daruda_config::agent::command_needs_remote_cwd`) but the active lane has
/// no `remote_cwd` configured. Unlike `agent_chat_no_lane_cwd`, this names
/// the fix (right-click the lane in the sidebar) rather than just stating
/// the symptom.
pub fn agent_chat_no_remote_cwd() -> String {
    rust_i18n::t!("agent_chat.no_remote_cwd").into_owned()
}

/// Hint appended to a connection error when the pane's working directory is
/// a remote (SSH/Docker) path. Guides the user to check the remote path
/// configuration and network connectivity.
pub fn agent_chat_remote_connect_error_hint() -> String {
    rust_i18n::t!("agent_chat.remote_connect_error_hint").into_owned()
}

/// Prefix for the connection-error status line.
pub fn agent_chat_error_prefix() -> String {
    rust_i18n::t!("agent_chat.error_prefix").into_owned()
}

/// Error-line reason set when the connection's event stream closes while the
/// pane is still `Connecting`/`Handshaking`/`PreparingRuntime` — no
/// `Connected`/`Error` ever arrived, so nothing else would surface a failure.
/// The renderer prepends the error prefix, so this is the bare reason.
pub fn agent_chat_error_stream_ended() -> String {
    rust_i18n::t!("agent_chat.error_stream_ended").into_owned()
}

/// Label for the "Retry" button shown in the Error status banner.
pub fn agent_chat_retry() -> String {
    rust_i18n::t!("agent_chat.retry").into_owned()
}

/// Empty-conversation hint shown before the first message.
pub fn agent_chat_empty() -> String {
    rust_i18n::t!("agent_chat.empty").into_owned()
}

/// Toolbar button: expand every foldable block in the conversation.
pub fn agent_chat_expand_all() -> String {
    rust_i18n::t!("agent_chat.expand_all").into_owned()
}

/// Toolbar button: collapse every foldable block in the conversation.
pub fn agent_chat_collapse_all() -> String {
    rust_i18n::t!("agent_chat.collapse_all").into_owned()
}

/// Section label for the bottom plan region.
pub fn agent_chat_plan_label() -> String {
    rust_i18n::t!("agent_chat.plan_label").into_owned()
}

/// Tooltip on the × button that dismisses a completed plan region.
pub fn agent_chat_plan_dismiss() -> String {
    rust_i18n::t!("agent_chat.plan_dismiss").into_owned()
}

/// Working-indicator label for the number of subagents running right now.
pub fn agent_chat_subagent_progress(n: usize) -> String {
    rust_i18n::t!("agent_chat.subagent_progress", n => n).into_owned()
}

/// Tooltip on the activity-bar title showing the session's last-activity time
/// (`SessionInfoUpdate.updated_at`). `time` is a pre-formatted timestamp.
pub fn agent_chat_last_active_tooltip(time: &str) -> String {
    rust_i18n::t!("agent_chat.last_active_tooltip", time = time).into_owned()
}

/// Tooltip on the context meter: the current context-window fill and, when the
/// agent reports it, cumulative session cost. `used`/`size` are pre-formatted
/// token counts; `percent` is the fill ratio; `cost` is a pre-formatted cost
/// string (already `" · <amount currency>"`) or empty when unavailable.
pub fn agent_chat_context_tooltip(used: &str, size: &str, percent: u8, cost: &str) -> String {
    rust_i18n::t!(
        "agent_chat.context_tooltip",
        used = used,
        size = size,
        percent = percent.to_string(),
        cost = cost
    )
    .into_owned()
}

/// Label for a collapsed agent reasoning ("thinking") block.
pub fn agent_chat_thinking_label() -> String {
    rust_i18n::t!("agent_chat.thinking_label").into_owned()
}

/// Label above a tool call's plain-text output.
pub fn agent_chat_tool_output_label() -> String {
    rust_i18n::t!("agent_chat.tool_output_label").into_owned()
}

/// Disclosure label for a tool call's raw input (JSON arguments).
pub fn agent_chat_raw_input_label() -> String {
    rust_i18n::t!("agent_chat.raw_input_label").into_owned()
}

/// Collapsed tool-group header summary, e.g. "3 tool calls".
pub fn agent_chat_tool_group_count(count: usize) -> String {
    rust_i18n::t!("agent_chat.tool_group_count", count = count).into_owned()
}

/// Tool-call status badge — executing.
pub fn agent_chat_tool_status_running() -> String {
    rust_i18n::t!("agent_chat.tool_status_running").into_owned()
}

/// Tool-call status badge — completed successfully.
pub fn agent_chat_tool_status_done() -> String {
    rust_i18n::t!("agent_chat.tool_status_done").into_owned()
}

/// Tool-call status badge — failed.
pub fn agent_chat_tool_status_failed() -> String {
    rust_i18n::t!("agent_chat.tool_status_failed").into_owned()
}

/// Tool-call status badge — cancelled (the turn was stopped before it settled).
pub fn agent_chat_tool_status_cancelled() -> String {
    rust_i18n::t!("agent_chat.tool_status_cancelled").into_owned()
}

/// Chip marking a shell command launched detached (`run_in_background: true`).
pub fn agent_chat_tool_background() -> String {
    rust_i18n::t!("agent_chat.tool_background").into_owned()
}

/// Label above a Task/Agent card's nested subagent tool calls.
pub fn agent_chat_subagent_label() -> String {
    rust_i18n::t!("agent_chat.subagent_label").into_owned()
}

/// Subagent label naming the spawned agent's type (Claude Code's `Task`
/// `subagent_type`), e.g. "Subagent: code-reviewer". Used in place of the
/// generic [`agent_chat_subagent_label`] when the type is known.
pub fn agent_chat_subagent_label_typed(kind: &str) -> String {
    rust_i18n::t!("agent_chat.subagent_label_typed", kind = kind).into_owned()
}

/// Pinned working-footer label while the agent is generating a response.
pub fn agent_chat_working() -> String {
    rust_i18n::t!("agent_chat.working").into_owned()
}

/// Pinned working-footer label while a named tool call is in progress.
pub fn agent_chat_working_tool(name: &str) -> String {
    rust_i18n::t!("agent_chat.working_tool", name = name).into_owned()
}

/// Pinned working-footer label while the turn is blocked on a permission prompt.
pub fn agent_chat_awaiting_permission() -> String {
    rust_i18n::t!("agent_chat.awaiting_permission").into_owned()
}

/// Heading for an inline permission card.
pub fn agent_chat_permission_title() -> String {
    rust_i18n::t!("agent_chat.permission_title").into_owned()
}

/// Prefix shown on a resolved permission card before the chosen option.
pub fn agent_chat_permission_resolved_prefix() -> String {
    rust_i18n::t!("agent_chat.permission_resolved_prefix").into_owned()
}

/// Shown on a permission card whose turn was cancelled before the user
/// decided, in place of the chosen-option line.
pub fn agent_chat_permission_cancelled() -> String {
    rust_i18n::t!("agent_chat.permission_cancelled").into_owned()
}

/// Description for the daruda-injected `/clear` slash command, shown in the
/// completion menu's detail column when the connected agent doesn't advertise
/// its own `clear` command.
pub fn agent_chat_clear_command_desc() -> String {
    rust_i18n::t!("agent_chat.clear_command_desc").into_owned()
}

pub fn file_viewer_search_no_match() -> String {
    rust_i18n::t!("file_viewer.search_no_match").into_owned()
}
pub const FILE_VIEWER_SEARCH_PREV: &str = "◀";
pub const FILE_VIEWER_SEARCH_NEXT: &str = "▶";
pub const FILE_VIEWER_SEARCH_CLOSE_BTN: &str = "✕";
pub const FILE_VIEWER_SEARCH_CLEAR: &str = "✕";

// ============================================================================
// Generic widget UI strings
// ============================================================================

/// Checkmark glyph rendered inside a checked checkbox.
pub const UI_CHECKMARK: &str = "✓";
/// Generic loading placeholder used in list widgets while items are fetched.
pub fn ui_loading() -> String {
    rust_i18n::t!("common.loading").into_owned()
}
/// Keystroke-input hint shown while waiting for keystrokes to be recorded.
pub fn keystroke_hint_recording() -> String {
    rust_i18n::t!("ui.keystroke_hint_recording").into_owned()
}
/// Keystroke-input hint shown when the widget is idle (nothing recorded yet).
pub fn keystroke_hint_idle() -> String {
    rust_i18n::t!("ui.keystroke_hint_idle").into_owned()
}

// ----------------------------------------------------------------
// Files view
// ----------------------------------------------------------------

pub fn files_header_label() -> String {
    rust_i18n::t!("ui.files_header").into_owned()
}
pub fn files_refresh_tooltip() -> String {
    rust_i18n::t!("common.refresh").into_owned()
}
pub const FILES_REFRESH_GLYPH: &str = "⟳";
pub fn files_loading() -> String {
    rust_i18n::t!("common.loading").into_owned()
}
pub fn files_empty_dir() -> String {
    rust_i18n::t!("ui.files_empty_dir").into_owned()
}
pub fn files_load_error_prefix() -> String {
    rust_i18n::t!("ui.files_load_error_prefix").into_owned()
}
pub const FILES_CHEVRON_PENDING: &str = "…";

// ----------------------------------------------------------------
// Lane context menu
// ----------------------------------------------------------------

pub fn ctx_reveal_in_finder() -> String {
    rust_i18n::t!("ctx.reveal_in_finder").into_owned()
}
pub fn ctx_copy_path() -> String {
    rust_i18n::t!("ctx.copy_path").into_owned()
}
pub fn ctx_edit_description() -> String {
    rust_i18n::t!("ctx.edit_description").into_owned()
}
pub fn ctx_edit_remote_cwd() -> String {
    rust_i18n::t!("ctx.edit_remote_cwd").into_owned()
}
/// Hover tooltip on the "Set Remote Path…" context-menu item, spelling
/// out that the setting only takes effect for panes created *after*
/// the change — an already-created (but not yet connected) agent-chat
/// pane keeps whatever cwd was resolved when it was created
/// (`resolve_new_pane_cwd` runs once, at pane-creation time).
pub fn ctx_edit_remote_cwd_hint() -> String {
    rust_i18n::t!("ctx.edit_remote_cwd_hint").into_owned()
}
pub fn ctx_rename() -> String {
    rust_i18n::t!("common.btn_rename").into_owned()
}
pub fn edit_description_modal_title() -> String {
    rust_i18n::t!("modal.edit_description_title").into_owned()
}
pub fn edit_description_placeholder() -> String {
    rust_i18n::t!("modal.edit_description_placeholder").into_owned()
}
pub fn edit_remote_cwd_modal_title() -> String {
    rust_i18n::t!("modal.edit_remote_cwd_title").into_owned()
}
pub fn edit_remote_cwd_placeholder() -> String {
    rust_i18n::t!("modal.edit_remote_cwd_placeholder").into_owned()
}
pub fn rename_modal_title() -> String {
    rust_i18n::t!("modal.rename_worktree_title").into_owned()
}
/// Title for the Remove-lane confirmation modal opened from the
/// left-dock row `×` button and the inaccessible empty-state.
pub fn remove_lane_modal_title() -> String {
    rust_i18n::t!("modal.remove_lane_title").into_owned()
}
pub fn rename_placeholder() -> String {
    rust_i18n::t!("modal.rename_placeholder").into_owned()
}

// Merge into context menu + modal
pub fn ctx_merge_into() -> String {
    rust_i18n::t!("ctx.merge_into").into_owned()
}
pub fn ctx_merge_disabled_dirty() -> String {
    rust_i18n::t!("ctx.merge_disabled_dirty").into_owned()
}
pub fn ctx_merge_disabled_detached() -> String {
    rust_i18n::t!("ctx.merge_disabled_detached").into_owned()
}

// ----------------------------------------------------------------
// Inaccessible lane / project state (Task 2)
// ----------------------------------------------------------------

/// Context-menu "Remove…" item for an inaccessible lane or project.
pub fn ctx_remove() -> String {
    rust_i18n::t!("ctx.remove").into_owned()
}
/// Informational (disabled) context-menu hint shown for an
/// access-denied lane — points the user at the macOS permission grant.
pub fn ctx_grant_full_disk_access() -> String {
    rust_i18n::t!("ctx.grant_full_disk_access").into_owned()
}
/// Short inline row label for a lane/project whose directory is gone.
pub fn projects_directory_missing() -> String {
    rust_i18n::t!("projects.directory_missing").into_owned()
}
/// Short inline row label for a lane/project whose directory is
/// present but unreadable (permission denied).
pub fn projects_permission_denied() -> String {
    rust_i18n::t!("projects.permission_denied").into_owned()
}
/// Main-area empty-state heading shown when the active lane's
/// directory is missing.
pub fn projects_empty_missing_title() -> String {
    rust_i18n::t!("projects.empty_missing_title").into_owned()
}
/// Main-area empty-state body shown when the active lane's directory
/// is missing.
pub fn projects_empty_missing_body() -> String {
    rust_i18n::t!("projects.empty_missing_body").into_owned()
}
/// Main-area empty-state heading shown when the active lane's
/// directory is access-denied.
pub fn projects_empty_denied_title() -> String {
    rust_i18n::t!("projects.empty_denied_title").into_owned()
}
/// Main-area empty-state body shown when the active lane's directory
/// is access-denied.
pub fn projects_empty_denied_body() -> String {
    rust_i18n::t!("projects.empty_denied_body").into_owned()
}
/// Main-area empty-state heading shown when the active lane is
/// accessible but has no open tabs (the user closed them all).
pub fn projects_empty_no_tabs_title() -> String {
    rust_i18n::t!("projects.empty_no_tabs_title").into_owned()
}
/// Main-area empty-state body shown when the active lane is accessible
/// but has no open tabs.
pub fn projects_empty_no_tabs_body() -> String {
    rust_i18n::t!("projects.empty_no_tabs_body").into_owned()
}
pub fn merge_modal_branch_label() -> String {
    rust_i18n::t!("modal.merge_branch_label").into_owned()
}
pub fn merge_modal_no_targets() -> String {
    rust_i18n::t!("modal.merge_no_targets").into_owned()
}
pub fn merge_modal_merging() -> String {
    rust_i18n::t!("modal.merge_merging").into_owned()
}
pub fn merge_modal_already_up_to_date() -> String {
    rust_i18n::t!("modal.merge_already_up_to_date").into_owned()
}
pub fn merge_modal_target_dirty() -> String {
    rust_i18n::t!("modal.merge_target_dirty").into_owned()
}
pub fn merge_modal_conflicts_note() -> String {
    rust_i18n::t!("modal.merge_conflicts_note").into_owned()
}
pub fn merge_modal_abort_merge() -> String {
    rust_i18n::t!("modal.merge_abort").into_owned()
}
pub fn merge_modal_remove_after() -> String {
    rust_i18n::t!("modal.merge_remove_after").into_owned()
}

// ----------------------------------------------------------------
// Bottom dock panels — tab modals + context menu
// ----------------------------------------------------------------

pub fn create_panel_tab_modal_title() -> String {
    rust_i18n::t!("modal.create_panel_tab_title").into_owned()
}
pub fn create_panel_tab_placeholder() -> String {
    rust_i18n::t!("modal.create_panel_tab_placeholder").into_owned()
}
pub fn rename_panel_tab_modal_title() -> String {
    rust_i18n::t!("modal.rename_panel_tab_title").into_owned()
}
pub fn rename_panel_tab_placeholder() -> String {
    rust_i18n::t!("modal.rename_panel_tab_placeholder").into_owned()
}
pub fn ctx_panel_tab_rename() -> String {
    rust_i18n::t!("common.btn_rename").into_owned()
}
pub fn ctx_panel_tab_delete() -> String {
    rust_i18n::t!("ctx.panel_tab_delete").into_owned()
}
pub fn delete_panel_tab_modal_title() -> String {
    rust_i18n::t!("modal.delete_panel_tab_title").into_owned()
}
pub fn delete_panel_tab_confirm_label() -> String {
    rust_i18n::t!("common.btn_delete").into_owned()
}
pub fn ctx_macro_edit() -> String {
    rust_i18n::t!("ctx.macro_edit").into_owned()
}
pub fn ctx_macro_delete() -> String {
    rust_i18n::t!("ctx.macro_delete").into_owned()
}
pub fn delete_macro_modal_title() -> String {
    rust_i18n::t!("modal.delete_macro_title").into_owned()
}
pub fn delete_macro_confirm_label() -> String {
    rust_i18n::t!("common.btn_delete").into_owned()
}

// Bottom dock — row-preset selector (suffix in the tab strip).
pub fn row_preset_1_label() -> String {
    rust_i18n::t!("bottom_dock.row_preset_1").into_owned()
}
pub fn row_preset_2_label() -> String {
    rust_i18n::t!("bottom_dock.row_preset_2").into_owned()
}
pub fn row_preset_3_label() -> String {
    rust_i18n::t!("bottom_dock.row_preset_3").into_owned()
}
pub const ROW_PRESET_CHECK_PREFIX: &str = "\u{2713} ";
pub const ROW_PRESET_UNCHECK_PREFIX: &str = "  ";
pub fn row_preset_tooltip() -> String {
    rust_i18n::t!("bottom_dock.row_preset_tooltip").into_owned()
}

// ----------------------------------------------------------------
// Bottom dock — terminal input panel (B-series)
// ----------------------------------------------------------------

pub fn bottom_input_tab_label() -> String {
    rust_i18n::t!("bottom_dock.input_tab_label").into_owned()
}
pub fn bottom_input_placeholder() -> String {
    rust_i18n::t!("bottom_dock.input_placeholder").into_owned()
}
/// Placeholder shown when the focused pane is an Agent chat with no active
/// mode and plain-Enter submit — "Message agent · Enter to send".
pub fn bottom_input_agent_placeholder() -> String {
    rust_i18n::t!("bottom_dock.input_agent_placeholder").into_owned()
}
/// Placeholder shown when the focused pane is an Agent chat with no active
/// mode and modifier-to-send on — "Message agent · ⌘↵ to send".
pub fn bottom_input_agent_modifier_placeholder() -> String {
    rust_i18n::t!("bottom_dock.input_agent_modifier_placeholder").into_owned()
}
/// Placeholder shown when the focused Agent chat pane has an active mode
/// and plain-Enter submit — "Message agent · <mode> · Enter to send".
pub fn bottom_input_agent_mode_placeholder(mode: &str) -> String {
    rust_i18n::t!("bottom_dock.input_agent_mode_placeholder", mode = mode).into_owned()
}
/// Placeholder shown when the focused Agent chat pane has an active mode
/// and modifier-to-send on — "Message agent · <mode> · ⌘↵ to send".
pub fn bottom_input_agent_mode_modifier_placeholder(mode: &str) -> String {
    rust_i18n::t!(
        "bottom_dock.input_agent_mode_modifier_placeholder",
        mode = mode
    )
    .into_owned()
}

/// Derive the bottom-input placeholder string from focused-pane context.
///
/// Pure function — no side effects, unit-testable.
///
/// - `is_agent` — the focused pane is an Agent chat pane.
/// - `mode_name` — the human-readable label of the agent's current mode
///   (`SessionModeView::name`), when the session advertises modes.
/// - `use_modifier_to_send` — `AgentConfig::use_modifier_to_send`; when
///   `true` the submit key is ⌘↵, otherwise plain Enter.
pub fn bottom_input_placeholder_for_context(
    is_agent: bool,
    mode_name: Option<&str>,
    use_modifier_to_send: bool,
) -> String {
    if !is_agent {
        return bottom_input_placeholder();
    }
    match (mode_name, use_modifier_to_send) {
        (Some(name), false) => bottom_input_agent_mode_placeholder(name),
        (Some(name), true) => bottom_input_agent_mode_modifier_placeholder(name),
        (None, false) => bottom_input_agent_placeholder(),
        (None, true) => bottom_input_agent_modifier_placeholder(),
    }
}
pub fn bottom_input_send_button() -> String {
    rust_i18n::t!("common.btn_submit").into_owned()
}
/// Bottom-input submit-button label while an Agent chat turn is in flight
/// — clicking it cancels the turn instead of sending.
pub fn bottom_input_stop_button() -> String {
    rust_i18n::t!("common.btn_stop").into_owned()
}

// ----------------------------------------------------------------
// Projects view — section header + empty state
// ----------------------------------------------------------------

pub fn projects_section_header() -> String {
    rust_i18n::t!("projects.section_header").into_owned()
}
pub fn projects_empty_state() -> String {
    rust_i18n::t!("projects.empty_state").into_owned()
}
/// CTA button label shown below the empty-state placeholder.
pub fn projects_empty_state_cta() -> String {
    rust_i18n::t!("projects.empty_state_cta").into_owned()
}

/// Section-header `[+]` toggle menu — entry labels. The `[+]` opens a
/// flat context menu instead of dispatching straight to the folder
/// picker so the user can pick between adding a Project or creating a
/// Group (also reachable via `Cmd+Shift+N` / the Command Palette).
pub fn section_add_menu_project() -> String {
    rust_i18n::t!("projects.add_menu_project").into_owned()
}
pub fn section_add_menu_group() -> String {
    rust_i18n::t!("projects.add_menu_group").into_owned()
}

// Group context menu (§5.1) — rename / recolor / collapse / delete.
pub fn group_menu_rename() -> String {
    rust_i18n::t!("group.menu_rename").into_owned()
}
pub fn group_menu_color_red() -> String {
    rust_i18n::t!("group.menu_color_red").into_owned()
}
pub fn group_menu_color_orange() -> String {
    rust_i18n::t!("group.menu_color_orange").into_owned()
}
pub fn group_menu_color_yellow() -> String {
    rust_i18n::t!("group.menu_color_yellow").into_owned()
}
pub fn group_menu_color_lime() -> String {
    rust_i18n::t!("group.menu_color_lime").into_owned()
}
pub fn group_menu_color_green() -> String {
    rust_i18n::t!("group.menu_color_green").into_owned()
}
pub fn group_menu_color_teal() -> String {
    rust_i18n::t!("group.menu_color_teal").into_owned()
}
pub fn group_menu_color_cyan() -> String {
    rust_i18n::t!("group.menu_color_cyan").into_owned()
}
pub fn group_menu_color_blue() -> String {
    rust_i18n::t!("group.menu_color_blue").into_owned()
}
pub fn group_menu_color_indigo() -> String {
    rust_i18n::t!("group.menu_color_indigo").into_owned()
}
pub fn group_menu_color_purple() -> String {
    rust_i18n::t!("group.menu_color_purple").into_owned()
}
pub fn group_menu_color_pink() -> String {
    rust_i18n::t!("group.menu_color_pink").into_owned()
}
pub fn group_menu_color_clear() -> String {
    rust_i18n::t!("group.menu_color_clear").into_owned()
}
pub fn group_menu_collapse() -> String {
    rust_i18n::t!("group.menu_collapse").into_owned()
}
pub fn group_menu_expand() -> String {
    rust_i18n::t!("group.menu_expand").into_owned()
}
pub fn group_menu_delete() -> String {
    rust_i18n::t!("group.menu_delete").into_owned()
}
pub fn group_rename_dialog_title() -> String {
    rust_i18n::t!("modal.group_rename_title").into_owned()
}
pub fn group_rename_dialog_placeholder() -> String {
    rust_i18n::t!("modal.group_rename_placeholder").into_owned()
}

/// Color presets exposed by the Group context menu. Hex strings are
/// stored on `SerializedGroup::color` so the dock's group header
/// chip can decode them via `gpui::Rgba::try_from(...)` without any
/// daruda-specific palette lookup.
pub const GROUP_PRESET_RED: &str = "#f87171";
pub const GROUP_PRESET_ORANGE: &str = "#fb923c";
pub const GROUP_PRESET_YELLOW: &str = "#facc15";
pub const GROUP_PRESET_LIME: &str = "#a3e635";
pub const GROUP_PRESET_GREEN: &str = "#4ade80";
pub const GROUP_PRESET_TEAL: &str = "#2dd4bf";
pub const GROUP_PRESET_CYAN: &str = "#22d3ee";
pub const GROUP_PRESET_BLUE: &str = "#60a5fa";
pub const GROUP_PRESET_INDIGO: &str = "#818cf8";
pub const GROUP_PRESET_PURPLE: &str = "#a78bfa";
pub const GROUP_PRESET_PINK: &str = "#f472b6";

// Project context menu (§5.1) — rename / move to group / delete /
// open in new window.
pub fn project_menu_rename() -> String {
    rust_i18n::t!("project.menu_rename").into_owned()
}
pub fn project_menu_move_to_group() -> String {
    rust_i18n::t!("project.menu_move_to_group").into_owned()
}
pub fn project_menu_delete() -> String {
    rust_i18n::t!("project.menu_delete").into_owned()
}
pub fn project_menu_open_in_new_window() -> String {
    rust_i18n::t!("project.menu_open_in_new_window").into_owned()
}

// ----------------------------------------------------------------
// Agent integration banner (dock prompt to install hooks)
// ----------------------------------------------------------------

pub const AGENT_BANNER_ICON: &str = "ⓘ";
pub fn agent_banner_title() -> String {
    rust_i18n::t!("claude.banner_title").into_owned()
}
pub fn agent_banner_hint() -> String {
    rust_i18n::t!("claude.banner_hint").into_owned()
}

pub fn agent_consent_title() -> String {
    rust_i18n::t!("claude.consent_title").into_owned()
}
pub fn agent_consent_body() -> String {
    rust_i18n::t!("claude.consent_body").into_owned()
}
pub fn agent_consent_confirm() -> String {
    rust_i18n::t!("claude.consent_confirm").into_owned()
}

/// Per-badge tooltip — appended after the session_id prefix to mark
/// the truncation. Localized separately from the active suffix so a
/// single en-dash / horizontal-ellipsis swap covers every badge.
pub const AGENT_BADGE_TOOLTIP_ELLIPSIS: &str = "…";
/// Suffix appended to the active session's badge tooltip to identify
/// the one that's bound to the focused tab. Empty for inactive
/// siblings.
pub fn agent_badge_tooltip_active_suffix() -> String {
    rust_i18n::t!("claude.badge_active_suffix").into_owned()
}
/// Sub-row label preceding the session badges (e.g. `"3 sessions:"`).
/// Rendered as `format!("{count}{SUFFIX}")`.
pub fn agent_sessions_label_suffix() -> String {
    rust_i18n::t!("claude.sessions_label_suffix").into_owned()
}

// ----------------------------------------------------------------
// Git Changes view
// ----------------------------------------------------------------

/// Placeholder text for the git commit message input.
pub fn git_commit_placeholder() -> String {
    rust_i18n::t!("git.commit_placeholder").into_owned()
}
/// Button label for the commit action in the git commit footer.
pub fn git_commit_btn() -> String {
    rust_i18n::t!("git.commit_btn").into_owned()
}
/// Button label for the push action in the git commit footer.
pub fn git_push_btn() -> String {
    rust_i18n::t!("git.push_btn").into_owned()
}

/// Placeholder shown while the first `git status` for a lane is
/// still in flight (cache miss).
pub fn git_loading_changes() -> String {
    rust_i18n::t!("git.loading_changes").into_owned()
}
/// Empty-state shown when the active lane is a Git repo with a clean
/// working tree.
pub fn git_no_changes() -> String {
    rust_i18n::t!("git.no_changes").into_owned()
}
/// Placeholder shown when the active lane is not a Git repository.
pub fn git_not_a_repository() -> String {
    rust_i18n::t!("git.not_a_repository").into_owned()
}
/// Manual-refresh button label inside the loading placeholder.
pub fn git_refresh_btn() -> String {
    rust_i18n::t!("common.refresh").into_owned()
}

/// Title for the discard-file confirmation dialog.
pub fn git_confirm_discard_title() -> String {
    rust_i18n::t!("git.confirm_discard_title").into_owned()
}
/// OK button label for the discard-file confirmation dialog.
pub fn git_confirm_discard_ok() -> String {
    rust_i18n::t!("git.confirm_discard_ok").into_owned()
}

/// Single-conflict banner shown at the top of the Git Changes view when
/// `git status` reports one merge conflict. Multi-conflict variants are
/// formatted inline with the count.
pub fn git_conflict_banner_single() -> String {
    rust_i18n::t!("git.conflict_banner_single").into_owned()
}

/// Button label in the non-git worktree placeholder. Click runs
/// `git init` in the lane path.
pub fn git_init_btn() -> String {
    rust_i18n::t!("git.init_btn").into_owned()
}

/// Title for the push confirmation dialog.
pub fn git_confirm_push_title() -> String {
    rust_i18n::t!("git.confirm_push_title").into_owned()
}
/// Body text for the push confirmation dialog.
pub fn git_confirm_push_body() -> String {
    rust_i18n::t!("git.confirm_push_body").into_owned()
}
/// OK button label for the push confirmation dialog.
pub fn git_confirm_push_ok() -> String {
    rust_i18n::t!("git.confirm_push_ok").into_owned()
}

/// Title for the commit confirmation dialog.
pub fn git_confirm_commit_title() -> String {
    rust_i18n::t!("git.confirm_commit_title").into_owned()
}
/// OK button label for the commit confirmation dialog.
pub fn git_confirm_commit_ok() -> String {
    rust_i18n::t!("git.confirm_commit_ok").into_owned()
}

/// Title for the amend confirmation dialog.
pub fn git_confirm_amend_title() -> String {
    rust_i18n::t!("git.confirm_amend_title").into_owned()
}
/// Body text for the amend confirmation dialog.
pub fn git_confirm_amend_body() -> String {
    rust_i18n::t!("git.confirm_amend_body").into_owned()
}
/// OK button label for the amend confirmation dialog.
pub fn git_confirm_amend_ok() -> String {
    rust_i18n::t!("git.confirm_amend_ok").into_owned()
}
/// Toast shown when the previous commit message can't be loaded for amend
/// (no commits yet, or an empty tip-commit message).
pub fn git_amend_load_failed() -> String {
    rust_i18n::t!("git.amend_load_failed").into_owned()
}
/// Primary commit-button label while in amend mode (replaces "Commit").
pub fn git_amend_btn() -> String {
    rust_i18n::t!("git.amend_btn").into_owned()
}
/// Dropdown label while in amend mode — backs out to a normal commit.
pub fn git_cancel_amend() -> String {
    rust_i18n::t!("git.cancel_amend").into_owned()
}
/// Warning when the user clears the commit box and tries to amend with no message.
pub fn git_amend_needs_message() -> String {
    rust_i18n::t!("git.amend_needs_message").into_owned()
}

/// Branch label fallback when HEAD is detached.
pub fn git_detached_label() -> String {
    rust_i18n::t!("git.detached_label").into_owned()
}
/// Section header for staged files in the Git Changes panel.
pub fn git_section_staged() -> String {
    rust_i18n::t!("git.section_staged").into_owned()
}
/// Section header for unstaged / untracked files in the Git Changes panel.
pub fn git_section_changes() -> String {
    rust_i18n::t!("git.section_changes").into_owned()
}
/// Button label to stage all unstaged files at once.
pub fn git_stage_all() -> String {
    rust_i18n::t!("git.stage_all").into_owned()
}
/// Button label shown when all files are staged — clicks unstages everything.
pub fn git_unstage_all() -> String {
    rust_i18n::t!("git.unstage_all").into_owned()
}
/// Button label for the fetch action in the git remote bar.
pub fn git_fetch_btn() -> String {
    rust_i18n::t!("git.fetch_btn").into_owned()
}
/// Button label for the pull action in the git remote bar.
pub fn git_pull_btn() -> String {
    rust_i18n::t!("git.pull_btn").into_owned()
}
/// Context menu — stage a single file.
pub fn ctx_git_stage() -> String {
    rust_i18n::t!("ctx.git_stage").into_owned()
}
/// Context menu — unstage a single file.
pub fn ctx_git_unstage() -> String {
    rust_i18n::t!("ctx.git_unstage").into_owned()
}
/// Context menu — discard working-tree changes for a file.
pub fn ctx_git_discard() -> String {
    rust_i18n::t!("ctx.git_discard").into_owned()
}
/// Context menu — open the diff viewer for a file.
pub fn ctx_git_open_diff() -> String {
    rust_i18n::t!("ctx.git_open_diff").into_owned()
}
/// Commit dropdown — amend the last commit with the current staged changes.
pub fn ctx_git_commit_amend() -> String {
    rust_i18n::t!("ctx.git_commit_amend").into_owned()
}

// ----------------------------------------------------------------
// Agent chat — role labels
// ----------------------------------------------------------------

/// Chat label for messages authored by the user.
pub fn agent_chat_label_user() -> String {
    rust_i18n::t!("agent.chat_label_user").into_owned()
}
/// Chat label for messages authored by the agent.
pub fn agent_chat_label_agent() -> String {
    rust_i18n::t!("agent.chat_label_agent").into_owned()
}
/// Chat label for system / tool messages injected into the chat stream.
pub fn agent_chat_label_system() -> String {
    rust_i18n::t!("agent.chat_label_system").into_owned()
}

// ----------------------------------------------------------------
// Settings panel
// ----------------------------------------------------------------

/// Status-bar / error-banner copy for the project-config flow.
pub fn project_config_no_project() -> String {
    rust_i18n::t!("settings.project_config_no_project").into_owned()
}
pub fn project_config_no_dir() -> String {
    rust_i18n::t!("settings.project_config_no_dir").into_owned()
}

/// Hover text on the small status-bar dot that indicates a
/// project-layer config file exists for the active project.
pub fn status_bar_project_config_tooltip() -> String {
    rust_i18n::t!("settings.status_bar_project_config_tooltip").into_owned()
}

/// Inline chip label shown in the status bar when the active git
/// lane is on a detached HEAD. Lowercase so it reads as a state
/// tag, not a sentence.
pub fn status_bar_detached_chip() -> String {
    rust_i18n::t!("settings.status_bar_detached_chip").into_owned()
}

/// Initial contents of a freshly-created
/// `~/.config/daruda/projects/<repo>-<hash>/config.toml`. The user
/// edits this file directly; daruda re-reads it on the next config
/// reload (via the recursive watcher under the user config dir).
pub const PROJECT_CONFIG_TEMPLATE: &str = "\
# daruda project-local config.
#
# Sections specified here override the user-global config
# (~/.config/daruda/config.toml) for this project's daruda windows.
# Sections you don't write keep their user-layer values.
#
# Phase 1 supports the [shell] section only.

# [shell]
# program = \"/usr/local/bin/zsh\"
# close_pane_on_exit = true
";
pub fn settings_title() -> String {
    rust_i18n::t!("settings.title").into_owned()
}
pub fn settings_section_font() -> String {
    rust_i18n::t!("settings.section_font").into_owned()
}
pub fn settings_section_cursor() -> String {
    rust_i18n::t!("settings.section_cursor").into_owned()
}
pub fn settings_section_shell() -> String {
    rust_i18n::t!("settings.section_shell").into_owned()
}
pub fn settings_section_window() -> String {
    rust_i18n::t!("settings.section_window").into_owned()
}
pub fn settings_label_font_family() -> String {
    rust_i18n::t!("settings.label_font_family").into_owned()
}
pub fn settings_label_font_size() -> String {
    rust_i18n::t!("settings.label_font_size").into_owned()
}
pub fn settings_label_editor_font_size() -> String {
    rust_i18n::t!("settings.label_editor_font_size").into_owned()
}
pub fn settings_label_agent_chat_font_size() -> String {
    rust_i18n::t!("settings.label_agent_chat_font_size").into_owned()
}
pub fn settings_label_vertical_spacing() -> String {
    rust_i18n::t!("settings.label_vertical_spacing").into_owned()
}
pub fn settings_label_horizontal_spacing() -> String {
    rust_i18n::t!("settings.label_horizontal_spacing").into_owned()
}
pub fn settings_label_cursor_style() -> String {
    rust_i18n::t!("settings.label_cursor_style").into_owned()
}
pub fn settings_label_cursor_blinking() -> String {
    rust_i18n::t!("settings.label_cursor_blinking").into_owned()
}
pub fn settings_label_max_fps() -> String {
    rust_i18n::t!("settings.label_max_fps").into_owned()
}
pub fn settings_max_fps_option(fps: u32) -> String {
    rust_i18n::t!("settings.max_fps_option", fps => fps).into_owned()
}
pub fn settings_label_close_on_exit() -> String {
    rust_i18n::t!("settings.label_close_on_exit").into_owned()
}
pub fn settings_label_window_opacity() -> String {
    rust_i18n::t!("settings.label_window_opacity").into_owned()
}
pub fn settings_label_window_blur() -> String {
    rust_i18n::t!("settings.label_window_blur").into_owned()
}
pub fn settings_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}
pub fn settings_save() -> String {
    rust_i18n::t!("common.btn_save").into_owned()
}
pub fn settings_err_font_size() -> String {
    rust_i18n::t!("settings.err_font_size").into_owned()
}
pub fn settings_err_editor_font_size() -> String {
    rust_i18n::t!("settings.err_editor_font_size").into_owned()
}
pub fn settings_err_agent_chat_font_size() -> String {
    rust_i18n::t!("settings.err_agent_chat_font_size").into_owned()
}
pub fn settings_err_spacing() -> String {
    rust_i18n::t!("settings.err_spacing").into_owned()
}
pub fn settings_err_opacity() -> String {
    rust_i18n::t!("settings.err_opacity").into_owned()
}
pub fn settings_cursor_block() -> String {
    rust_i18n::t!("settings.cursor_block").into_owned()
}
pub fn settings_cursor_underline() -> String {
    rust_i18n::t!("settings.cursor_underline").into_owned()
}
pub fn settings_cursor_bar() -> String {
    rust_i18n::t!("settings.cursor_bar").into_owned()
}
pub fn settings_section_theme() -> String {
    rust_i18n::t!("settings.section_theme").into_owned()
}
pub fn settings_label_theme() -> String {
    rust_i18n::t!("settings.label_theme").into_owned()
}
pub fn settings_label_language() -> String {
    rust_i18n::t!("settings.label_language").into_owned()
}
pub fn settings_language_auto() -> String {
    rust_i18n::t!("settings.language_auto").into_owned()
}
pub fn settings_language_en() -> String {
    rust_i18n::t!("settings.language_en").into_owned()
}
pub fn settings_language_ko() -> String {
    rust_i18n::t!("settings.language_ko").into_owned()
}
pub fn settings_label_terminal_theme() -> String {
    rust_i18n::t!("settings.label_terminal_theme").into_owned()
}
pub fn settings_label_ui_theme() -> String {
    rust_i18n::t!("settings.label_ui_theme").into_owned()
}
pub fn settings_ui_theme_phase3_tooltip() -> String {
    rust_i18n::t!("settings.ui_theme_phase3_tooltip").into_owned()
}
pub fn settings_section_terminal() -> String {
    rust_i18n::t!("settings.section_terminal").into_owned()
}
pub fn settings_label_scrollback() -> String {
    rust_i18n::t!("settings.label_scrollback").into_owned()
}
pub fn settings_err_scrollback() -> String {
    rust_i18n::t!("settings.err_scrollback").into_owned()
}
pub fn settings_label_inset_x() -> String {
    rust_i18n::t!("settings.label_inset_x").into_owned()
}
pub fn settings_label_inset_y() -> String {
    rust_i18n::t!("settings.label_inset_y").into_owned()
}
pub fn settings_err_inset() -> String {
    rust_i18n::t!("settings.err_inset").into_owned()
}
pub fn settings_section_sidebar() -> String {
    rust_i18n::t!("settings.section_sidebar").into_owned()
}
pub fn settings_label_show_hidden() -> String {
    rust_i18n::t!("settings.label_show_hidden").into_owned()
}
pub fn settings_label_use_gitignore() -> String {
    rust_i18n::t!("settings.label_use_gitignore").into_owned()
}
pub fn settings_label_syntax_theme() -> String {
    rust_i18n::t!("settings.label_syntax_theme").into_owned()
}

pub fn settings_syntax_theme_daruda() -> String {
    rust_i18n::t!("settings.syntax_theme_daruda").into_owned()
}

pub fn settings_syntax_theme_one_dark() -> String {
    rust_i18n::t!("settings.syntax_theme_one_dark").into_owned()
}

pub fn settings_syntax_theme_tokyo_night() -> String {
    rust_i18n::t!("settings.syntax_theme_tokyo_night").into_owned()
}

pub fn settings_syntax_theme_catppuccin_mocha() -> String {
    rust_i18n::t!("settings.syntax_theme_catppuccin_mocha").into_owned()
}

pub fn settings_syntax_theme_dracula() -> String {
    rust_i18n::t!("settings.syntax_theme_dracula").into_owned()
}

pub fn settings_syntax_theme_github_dark() -> String {
    rust_i18n::t!("settings.syntax_theme_github_dark").into_owned()
}

pub fn settings_syntax_theme_material_palenight() -> String {
    rust_i18n::t!("settings.syntax_theme_material_palenight").into_owned()
}

pub fn settings_syntax_theme_monokai() -> String {
    rust_i18n::t!("settings.syntax_theme_monokai").into_owned()
}

pub fn settings_syntax_theme_nord() -> String {
    rust_i18n::t!("settings.syntax_theme_nord").into_owned()
}

pub fn settings_syntax_theme_gruvbox_dark() -> String {
    rust_i18n::t!("settings.syntax_theme_gruvbox_dark").into_owned()
}

pub fn settings_syntax_theme_solarized_dark() -> String {
    rust_i18n::t!("settings.syntax_theme_solarized_dark").into_owned()
}

pub fn settings_syntax_theme_ayu_mirage() -> String {
    rust_i18n::t!("settings.syntax_theme_ayu_mirage").into_owned()
}

pub fn settings_syntax_theme_night_owl() -> String {
    rust_i18n::t!("settings.syntax_theme_night_owl").into_owned()
}

pub fn settings_syntax_theme_darcula() -> String {
    rust_i18n::t!("settings.syntax_theme_darcula").into_owned()
}

// ----------------------------------------------------------------
// Dock nav labels — title-case for the left-rail section list.
// (The uppercase `SETTINGS_SECTION_*` consts above are used as
//  body-area headers inside each rendered section.)
// ----------------------------------------------------------------
pub fn settings_nav_general() -> String {
    rust_i18n::t!("settings.nav_general").into_owned()
}
pub fn settings_nav_font() -> String {
    rust_i18n::t!("settings.nav_font").into_owned()
}
pub fn settings_nav_cursor() -> String {
    rust_i18n::t!("settings.nav_cursor").into_owned()
}
pub fn settings_nav_shell() -> String {
    rust_i18n::t!("settings.nav_shell").into_owned()
}
pub fn settings_nav_window() -> String {
    rust_i18n::t!("settings.nav_window").into_owned()
}
pub fn settings_nav_terminal() -> String {
    rust_i18n::t!("settings.nav_terminal").into_owned()
}
pub fn settings_nav_sidebar() -> String {
    rust_i18n::t!("settings.nav_sidebar").into_owned()
}
pub fn settings_nav_clipboard() -> String {
    rust_i18n::t!("settings.nav_clipboard").into_owned()
}
pub fn settings_nav_panels() -> String {
    rust_i18n::t!("settings.nav_panels").into_owned()
}
pub fn settings_nav_claude_status() -> String {
    rust_i18n::t!("settings.nav_claude_status").into_owned()
}
pub fn settings_nav_notifications() -> String {
    rust_i18n::t!("settings.nav_notifications").into_owned()
}
pub fn settings_nav_keymap() -> String {
    rust_i18n::t!("settings.nav_keymap").into_owned()
}
pub fn settings_nav_plugin() -> String {
    rust_i18n::t!("settings.nav_plugin").into_owned()
}
pub fn settings_nav_agent() -> String {
    rust_i18n::t!("settings.nav_agent").into_owned()
}

pub fn settings_section_general() -> String {
    rust_i18n::t!("settings.section_general").into_owned()
}
pub fn settings_section_clipboard() -> String {
    rust_i18n::t!("settings.section_clipboard").into_owned()
}
pub fn settings_section_panels() -> String {
    rust_i18n::t!("settings.section_panels").into_owned()
}
pub fn settings_section_claude_status() -> String {
    rust_i18n::t!("settings.section_claude_status").into_owned()
}
pub fn settings_section_notifications() -> String {
    rust_i18n::t!("settings.section_notifications").into_owned()
}
pub fn settings_section_keymap() -> String {
    rust_i18n::t!("settings.section_keymap").into_owned()
}
pub fn settings_section_plugin() -> String {
    rust_i18n::t!("settings.section_plugin").into_owned()
}
pub fn settings_section_agent() -> String {
    rust_i18n::t!("settings.section_agent").into_owned()
}

pub fn settings_label_clipboard_streaming() -> String {
    rust_i18n::t!("settings.label_clipboard_streaming").into_owned()
}
pub fn settings_label_grid_columns() -> String {
    rust_i18n::t!("settings.label_grid_columns").into_owned()
}
pub fn settings_label_claude_status_enable() -> String {
    rust_i18n::t!("settings.label_claude_status_enable").into_owned()
}
pub fn settings_err_clipboard() -> String {
    rust_i18n::t!("settings.err_clipboard").into_owned()
}
pub fn settings_err_grid_columns() -> String {
    rust_i18n::t!("settings.err_grid_columns").into_owned()
}
pub fn settings_open_config_file() -> String {
    rust_i18n::t!("settings.open_config_file").into_owned()
}
pub fn settings_placeholder_keymap() -> String {
    rust_i18n::t!("settings.placeholder_keymap").into_owned()
}
pub fn settings_placeholder_notifications() -> String {
    rust_i18n::t!("settings.placeholder_notifications").into_owned()
}

// Notifications section — Telegram bridge subsection.
pub fn settings_telegram_heading() -> String {
    rust_i18n::t!("settings.telegram_heading").into_owned()
}
pub fn settings_telegram_enabled_label() -> String {
    rust_i18n::t!("settings.telegram_enabled_label").into_owned()
}
pub fn settings_telegram_token_label() -> String {
    rust_i18n::t!("settings.telegram_token_label").into_owned()
}
pub fn settings_telegram_token_placeholder() -> String {
    rust_i18n::t!("settings.telegram_token_placeholder").into_owned()
}
pub fn settings_telegram_save_token() -> String {
    rust_i18n::t!("settings.telegram_save_token").into_owned()
}
pub fn settings_telegram_token_configured() -> String {
    rust_i18n::t!("settings.telegram_token_configured").into_owned()
}
pub fn settings_telegram_clear_token() -> String {
    rust_i18n::t!("settings.telegram_clear_token").into_owned()
}
pub fn settings_telegram_not_paired() -> String {
    rust_i18n::t!("settings.telegram_not_paired").into_owned()
}
pub fn settings_telegram_paired(chat_id: i64) -> String {
    rust_i18n::t!("settings.telegram_paired", chat_id = chat_id).into_owned()
}
pub fn settings_telegram_check_pairing() -> String {
    rust_i18n::t!("settings.telegram_check_pairing").into_owned()
}
pub fn settings_telegram_generate_code() -> String {
    rust_i18n::t!("settings.telegram_generate_code").into_owned()
}
pub fn settings_telegram_pair_instructions(code: &str) -> String {
    rust_i18n::t!("settings.telegram_pair_instructions", code = code).into_owned()
}
pub fn settings_telegram_unpair() -> String {
    rust_i18n::t!("settings.telegram_unpair").into_owned()
}

// Agent section — default permission mode dropdown
pub fn settings_label_agent_mode() -> String {
    rust_i18n::t!("settings.label_agent_mode").into_owned()
}
pub fn settings_agent_mode_default() -> String {
    rust_i18n::t!("settings.agent_mode_default").into_owned()
}
pub fn settings_agent_mode_accept_edits() -> String {
    rust_i18n::t!("settings.agent_mode_accept_edits").into_owned()
}
pub fn settings_agent_mode_plan() -> String {
    rust_i18n::t!("settings.agent_mode_plan").into_owned()
}
pub fn settings_agent_mode_bypass() -> String {
    rust_i18n::t!("settings.agent_mode_bypass").into_owned()
}
pub fn settings_label_agent_use_modifier_to_send() -> String {
    rust_i18n::t!("settings.label_agent_use_modifier_to_send").into_owned()
}
pub fn settings_agent_use_modifier_to_send_description() -> String {
    rust_i18n::t!("settings.agent_use_modifier_to_send_description").into_owned()
}
pub fn settings_section_agent_catalog() -> String {
    rust_i18n::t!("settings.section_agent_catalog").into_owned()
}
pub fn settings_agent_catalog_description() -> String {
    rust_i18n::t!("settings.agent_catalog_description").into_owned()
}
pub fn settings_agent_catalog_empty() -> String {
    rust_i18n::t!("settings.agent_catalog_empty").into_owned()
}
pub fn settings_agent_catalog_row_label(index: usize) -> String {
    rust_i18n::t!("settings.agent_catalog_row_label", index = index).into_owned()
}
pub fn settings_agent_field_id() -> String {
    rust_i18n::t!("settings.agent_field_id").into_owned()
}
pub fn settings_agent_field_name() -> String {
    rust_i18n::t!("settings.agent_field_name").into_owned()
}
pub fn settings_agent_field_command() -> String {
    rust_i18n::t!("settings.agent_field_command").into_owned()
}

/// Label for the transport-kind select (`daruda_config::AgentLaunch`'s three
/// variants) in an agent catalog row.
pub fn settings_agent_field_transport() -> String {
    rust_i18n::t!("settings.agent_field_transport").into_owned()
}

/// Transport-select option label for `AgentLaunch::Raw`.
pub fn settings_agent_transport_raw() -> String {
    rust_i18n::t!("settings.agent_transport_raw").into_owned()
}

/// Transport-select option label for `AgentLaunch::Ssh`.
pub fn settings_agent_transport_ssh() -> String {
    rust_i18n::t!("settings.agent_transport_ssh").into_owned()
}

/// Transport-select option label for `AgentLaunch::Docker`.
pub fn settings_agent_transport_docker() -> String {
    rust_i18n::t!("settings.agent_transport_docker").into_owned()
}

/// Label for the SSH host field, shown only when a row's transport is `ssh`.
pub fn settings_agent_field_host() -> String {
    rust_i18n::t!("settings.agent_field_host").into_owned()
}

/// Label for the Docker container field, shown only when a row's transport
/// is `docker`.
pub fn settings_agent_field_container() -> String {
    rust_i18n::t!("settings.agent_field_container").into_owned()
}

/// Hint line shown under the host/container field for an `ssh`/`docker`
/// row, pointing at the Lane's remote-path setting that
/// `AgentLaunch::wrap` substitutes in at connect time.
pub fn settings_agent_remote_path_hint() -> String {
    rust_i18n::t!("settings.agent_remote_path_hint").into_owned()
}

pub fn settings_agent_preset() -> String {
    rust_i18n::t!("settings.agent_preset").into_owned()
}

pub fn settings_agent_add_preset() -> String {
    rust_i18n::t!("settings.agent_add_preset").into_owned()
}

pub fn settings_agent_add_custom() -> String {
    rust_i18n::t!("settings.agent_add_custom").into_owned()
}
pub fn settings_agent_remove() -> String {
    rust_i18n::t!("settings.agent_remove").into_owned()
}
pub fn settings_err_agent_catalog_empty() -> String {
    rust_i18n::t!("settings.err_agent_catalog_empty").into_owned()
}
pub fn settings_err_agent_catalog_field(index: usize) -> String {
    rust_i18n::t!("settings.err_agent_catalog_field", index = index).into_owned()
}
pub fn settings_err_agent_catalog_id(id: &str) -> String {
    rust_i18n::t!("settings.err_agent_catalog_id", id = id).into_owned()
}
pub fn settings_err_agent_catalog_duplicate(id: &str) -> String {
    rust_i18n::t!("settings.err_agent_catalog_duplicate", id = id).into_owned()
}
pub fn settings_err_agent_catalog_host(index: usize) -> String {
    rust_i18n::t!("settings.err_agent_catalog_host", index = index).into_owned()
}
pub fn settings_err_agent_catalog_container(index: usize) -> String {
    rust_i18n::t!("settings.err_agent_catalog_container", index = index).into_owned()
}

// Plugin section — install / uninstall UI labels
pub fn settings_plugin_installed_header() -> String {
    rust_i18n::t!("settings.plugin_installed_header").into_owned()
}
pub fn settings_plugin_available_header() -> String {
    rust_i18n::t!("settings.plugin_available_header").into_owned()
}
pub fn settings_plugin_none_installed() -> String {
    rust_i18n::t!("settings.plugin_none_installed").into_owned()
}
pub fn settings_plugin_none_available() -> String {
    rust_i18n::t!("settings.plugin_none_available").into_owned()
}
pub fn settings_plugin_install() -> String {
    rust_i18n::t!("settings.plugin_install").into_owned()
}
pub fn settings_plugin_uninstall() -> String {
    rust_i18n::t!("settings.plugin_uninstall").into_owned()
}
pub fn settings_plugin_installing() -> String {
    rust_i18n::t!("settings.plugin_installing").into_owned()
}
pub fn settings_plugin_uninstalling() -> String {
    rust_i18n::t!("settings.plugin_uninstalling").into_owned()
}

// Plugin detail pane (Settings → Plugin master-detail layout)
pub fn settings_plugin_detail_empty() -> String {
    rust_i18n::t!("settings.plugin_detail_empty").into_owned()
}
pub fn settings_plugin_detail_marketplace() -> String {
    rust_i18n::t!("settings.plugin_detail_marketplace").into_owned()
}
pub fn settings_plugin_detail_version() -> String {
    rust_i18n::t!("settings.plugin_detail_version").into_owned()
}
pub fn settings_plugin_detail_path() -> String {
    rust_i18n::t!("settings.plugin_detail_path").into_owned()
}
pub fn settings_plugin_detail_scope() -> String {
    rust_i18n::t!("settings.plugin_detail_scope").into_owned()
}
pub fn settings_plugin_detail_availability() -> String {
    rust_i18n::t!("settings.plugin_detail_availability").into_owned()
}
pub fn settings_plugin_detail_status_installed() -> String {
    rust_i18n::t!("settings.plugin_detail_status_installed").into_owned()
}
pub fn settings_plugin_detail_status_available() -> String {
    rust_i18n::t!("settings.plugin_detail_status_available").into_owned()
}
pub fn settings_plugin_detail_skills_header() -> String {
    rust_i18n::t!("settings.plugin_detail_skills_header").into_owned()
}
pub fn settings_plugin_detail_no_skills() -> String {
    rust_i18n::t!("settings.plugin_detail_no_skills").into_owned()
}
pub fn settings_plugin_detail_unknown() -> String {
    rust_i18n::t!("settings.plugin_detail_unknown").into_owned()
}
pub fn settings_plugin_skill_view() -> String {
    rust_i18n::t!("settings.plugin_skill_view").into_owned()
}
pub fn settings_plugin_skill_description() -> String {
    rust_i18n::t!("settings.plugin_skill_description").into_owned()
}
pub fn settings_plugin_skill_invocation() -> String {
    rust_i18n::t!("settings.plugin_skill_invocation").into_owned()
}
pub fn settings_plugin_skill_argument_hint() -> String {
    rust_i18n::t!("settings.plugin_skill_argument_hint").into_owned()
}
pub fn settings_plugin_skill_allowed_tools() -> String {
    rust_i18n::t!("settings.plugin_skill_allowed_tools").into_owned()
}
pub fn settings_plugin_skill_paths() -> String {
    rust_i18n::t!("settings.plugin_skill_paths").into_owned()
}
pub fn settings_plugin_skill_when_to_use() -> String {
    rust_i18n::t!("settings.plugin_skill_when_to_use").into_owned()
}
pub fn settings_plugin_skill_invocation_both() -> String {
    rust_i18n::t!("settings.plugin_skill_invocation_both").into_owned()
}
pub fn settings_plugin_skill_invocation_user_only() -> String {
    rust_i18n::t!("settings.plugin_skill_invocation_user_only").into_owned()
}
pub fn settings_plugin_skill_invocation_model_only() -> String {
    rust_i18n::t!("settings.plugin_skill_invocation_model_only").into_owned()
}
pub fn settings_plugin_skill_invocation_disabled() -> String {
    rust_i18n::t!("settings.plugin_skill_invocation_disabled").into_owned()
}
pub fn settings_plugin_skill_body() -> String {
    rust_i18n::t!("settings.plugin_skill_body").into_owned()
}
pub fn settings_plugin_skill_back() -> String {
    rust_i18n::t!("settings.plugin_skill_back").into_owned()
}
pub fn settings_plugin_skill_body_loading() -> String {
    rust_i18n::t!("settings.plugin_skill_body_loading").into_owned()
}
pub fn settings_plugin_skill_body_error() -> String {
    rust_i18n::t!("settings.plugin_skill_body_error").into_owned()
}

// About section — app version + self-update controls
pub fn settings_nav_about() -> String {
    rust_i18n::t!("settings.nav_about").into_owned()
}
pub fn settings_section_about() -> String {
    rust_i18n::t!("settings.section_about").into_owned()
}
pub fn settings_label_current_version() -> String {
    rust_i18n::t!("settings.label_current_version").into_owned()
}
pub fn settings_button_check_updates() -> String {
    rust_i18n::t!("settings.button_check_updates").into_owned()
}
pub fn settings_button_update() -> String {
    rust_i18n::t!("settings.button_update").into_owned()
}
pub fn settings_button_restart() -> String {
    rust_i18n::t!("settings.button_restart").into_owned()
}
pub fn settings_update_checking() -> String {
    rust_i18n::t!("settings.update_checking").into_owned()
}
pub fn settings_update_up_to_date() -> String {
    rust_i18n::t!("settings.update_up_to_date").into_owned()
}
pub fn settings_update_available(version: &str) -> String {
    rust_i18n::t!("settings.update_available", version => version).into_owned()
}
pub fn settings_update_downloading() -> String {
    rust_i18n::t!("settings.update_downloading").into_owned()
}
pub fn settings_update_installing() -> String {
    rust_i18n::t!("settings.update_installing").into_owned()
}
pub fn settings_update_ready() -> String {
    rust_i18n::t!("settings.update_ready").into_owned()
}
pub fn settings_update_error(msg: &str) -> String {
    rust_i18n::t!("settings.update_error", msg => msg).into_owned()
}
pub fn settings_update_dev_build() -> String {
    rust_i18n::t!("settings.update_dev_build").into_owned()
}
pub fn update_available_toast(version: &str) -> String {
    rust_i18n::t!("settings.update_available_toast", version => version).into_owned()
}

// ============================================================================
// Context menu labels (right-click on tab bar / pane header)
// ============================================================================

pub fn ctx_close_tab() -> String {
    rust_i18n::t!("common.close_tab").into_owned()
}
pub fn ctx_close_other_tabs() -> String {
    rust_i18n::t!("ctx.close_other_tabs").into_owned()
}
pub fn ctx_close_tabs_to_right() -> String {
    rust_i18n::t!("ctx.close_tabs_to_right").into_owned()
}
pub fn ctx_move_tab_left() -> String {
    rust_i18n::t!("common.move_tab_left").into_owned()
}
pub fn ctx_move_tab_right() -> String {
    rust_i18n::t!("common.move_tab_right").into_owned()
}
pub fn ctx_new_tab() -> String {
    rust_i18n::t!("common.new_tab").into_owned()
}
pub fn ctx_new_agent_chat() -> String {
    rust_i18n::t!("common.new_agent_chat").into_owned()
}
pub fn ctx_new_terminal() -> String {
    rust_i18n::t!("common.new_terminal").into_owned()
}
pub fn new_agent_chat_named(name: &str) -> String {
    rust_i18n::t!("common.new_agent_chat_named", name = name).into_owned()
}
/// Suffix appended to a `+`-menu agent entry's label when the entry is
/// disabled because the agent's command needs a remote working directory but
/// the active lane has none set. `PopupMenuItem` has no tooltip API, so the
/// reason has to live in the label text itself.
pub fn agent_needs_remote_cwd_suffix() -> String {
    rust_i18n::t!("common.agent_needs_remote_cwd_suffix").into_owned()
}
pub fn ctx_split_terminal_horizontal() -> String {
    rust_i18n::t!("common.split_terminal_horizontal").into_owned()
}
pub fn ctx_split_terminal_vertical() -> String {
    rust_i18n::t!("common.split_terminal_vertical").into_owned()
}
pub fn ctx_split_agent_chat_horizontal() -> String {
    rust_i18n::t!("common.split_agent_chat_horizontal").into_owned()
}
pub fn ctx_split_agent_chat_vertical() -> String {
    rust_i18n::t!("common.split_agent_chat_vertical").into_owned()
}
pub fn ctx_copy_file_path() -> String {
    rust_i18n::t!("ctx.copy_file_path").into_owned()
}
pub fn ctx_copy_relative_path() -> String {
    rust_i18n::t!("ctx.copy_relative_path").into_owned()
}
pub fn ctx_close_file_viewer() -> String {
    rust_i18n::t!("ctx.close_file_viewer").into_owned()
}
pub fn ctx_close_pane() -> String {
    rust_i18n::t!("common.close_pane").into_owned()
}
pub fn ctx_zoom_pane() -> String {
    rust_i18n::t!("ctx.zoom_pane").into_owned()
}
pub fn ctx_unzoom_pane() -> String {
    rust_i18n::t!("ctx.unzoom_pane").into_owned()
}

// ============================================================================
// Notifications
// ============================================================================

/// Title for "long-running command finished" desktop notifications.
pub fn notification_long_running_title() -> String {
    rust_i18n::t!("notification.long_running_title").into_owned()
}

/// Title for a Claude permission-prompt desktop notification.
pub fn notification_hook_permission_title() -> String {
    rust_i18n::t!("notification.hook_permission_title").into_owned()
}

/// Title for a Claude idle-prompt desktop notification.
pub fn notification_hook_idle_title() -> String {
    rust_i18n::t!("notification.hook_idle_title").into_owned()
}

/// Title for a Claude elicitation-dialog desktop notification.
pub fn notification_hook_elicitation_title() -> String {
    rust_i18n::t!("notification.hook_elicitation_title").into_owned()
}

/// Body for the "agent finished a turn" desktop notification.
pub fn agent_notification_completed() -> String {
    rust_i18n::t!("notification.agent_completed").into_owned()
}

/// Body for the "agent is waiting for input / permission" desktop notification.
pub fn agent_notification_waiting() -> String {
    rust_i18n::t!("notification.agent_waiting").into_owned()
}
pub fn agent_notification_telegram_truncated_marker() -> String {
    rust_i18n::t!("notification.telegram_truncated_marker").into_owned()
}

/// Body for the Telegram ack sent the moment a phone-relayed reply is injected —
/// closes the "did it go through?" gap before the turn produces any output.
pub fn agent_notification_telegram_reply_ack() -> String {
    rust_i18n::t!("notification.telegram_reply_ack").into_owned()
}

/// Prefix for a Telegram follow-up carrying a message the agent produced *after*
/// its turn ended (a background job's completion report).
pub fn agent_notification_telegram_background_update() -> String {
    rust_i18n::t!("notification.telegram_background_update").into_owned()
}

/// Format a `Duration` as a compact, human-friendly span for the
/// "command finished" notification body. Examples: `42s`, `1m 03s`,
/// `2h 15m`. Sub-second resolution is dropped; the user threshold
/// is in whole seconds and any "completed in <1s" command is below
/// the long-running cut-off anyway.
pub fn format_duration_compact(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let hours = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

// ============================================================================
// Right panel — Skills tab
// ============================================================================

/// Section heading for the project-scope group.
pub fn skills_project() -> String {
    rust_i18n::t!("common.section_project").into_owned()
}
/// Section heading for the personal-scope group.
pub fn skills_personal() -> String {
    rust_i18n::t!("common.section_personal").into_owned()
}
/// Section heading for the plugin-scope group. Plugin skills come
/// from marketplace installs (`~/.claude/plugins/cache/...`) and are
/// **read-only** in daruda — Edit / Delete / Rename are disabled.
pub fn skills_plugin() -> String {
    rust_i18n::t!("skills.section_plugin").into_owned()
}
/// Header button — opens the Create modal.
pub fn skills_new_button() -> String {
    rust_i18n::t!("skills.new_button").into_owned()
}
/// Header button on the right-panel Skills tab — opens Settings →
/// Plugin so the user can install / uninstall plugins.
pub fn skills_manage_plugins_button() -> String {
    rust_i18n::t!("skills.manage_plugins_button").into_owned()
}

// ----------------------------------------------------------------
// Skill invocation modal — clicking a row opens this, Submit writes
// `/<skill> <input>\n` into the focused terminal pane.
// ----------------------------------------------------------------
/// Title of the skill-invocation modal (`open_form_modal`).
pub fn skills_invoke_title() -> String {
    rust_i18n::t!("skills.invoke_title").into_owned()
}
pub fn skills_invoke_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}
pub fn skills_invoke_submit() -> String {
    rust_i18n::t!("common.btn_submit").into_owned()
}
pub fn skills_invoke_submitting() -> String {
    rust_i18n::t!("skills.invoke_submitting").into_owned()
}
/// Primary-button label on the create / edit skill modals.
pub fn skills_button_save() -> String {
    rust_i18n::t!("common.btn_save").into_owned()
}
/// Cancel-button label on the create / edit skill modals.
pub fn skills_button_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}
/// Primary-button label while a create / edit skill save is in flight.
pub fn skills_saving_label() -> String {
    rust_i18n::t!("skills.saving_label").into_owned()
}
pub fn skills_invoke_placeholder_default() -> String {
    rust_i18n::t!("skills.invoke_placeholder_default").into_owned()
}
/// Shown when a skill can't be delivered to the captured pane — the
/// pane is gone, is a kind that can't receive text (file / task editor),
/// or the user switched lanes while the modal was open.
pub fn skills_invoke_no_input_target() -> String {
    rust_i18n::t!("skills.invoke_no_input_target").into_owned()
}
/// Empty-state hint in the skill picker when a plugin group exposes no
/// invocable skills (the picker chains into the invocation modal).
pub fn skills_empty_plugin_picker() -> String {
    rust_i18n::t!("skills.empty_plugin_picker").into_owned()
}

// Create / edit skill modal — input placeholders. Shared between the
// two modals so the same field reads identically in both.
pub fn skills_placeholder_name() -> String {
    rust_i18n::t!("skills.placeholder_name").into_owned()
}
pub fn skills_placeholder_description() -> String {
    rust_i18n::t!("skills.placeholder_description").into_owned()
}
pub fn skills_placeholder_when_to_use() -> String {
    rust_i18n::t!("skills.placeholder_when_to_use").into_owned()
}
pub fn skills_placeholder_optional() -> String {
    rust_i18n::t!("skills.placeholder_optional").into_owned()
}
pub fn skills_placeholder_body() -> String {
    rust_i18n::t!("skills.placeholder_body").into_owned()
}
/// Body editor placeholder shown while the existing skill's markdown is
/// still loading from disk in the edit modal.
pub fn skills_placeholder_body_loading() -> String {
    rust_i18n::t!("skills.placeholder_body_loading").into_owned()
}

// ----------------------------------------------------------------
// Skills search bar — substring filter atop the Skills tab.
// ----------------------------------------------------------------
pub fn skills_search_placeholder() -> String {
    rust_i18n::t!("skills.search_placeholder").into_owned()
}
pub fn skills_search_empty_prefix() -> String {
    rust_i18n::t!("skills.search_empty_prefix").into_owned()
}
/// Glyph for the in-field clear button. Rendered on the trailing edge of
/// the search input only while the query is non-empty.
pub const SKILLS_SEARCH_CLEAR_ICON: &str = "✕";

/// Body shown when both scopes are empty.
pub fn skills_empty_project() -> String {
    rust_i18n::t!("skills.empty_project").into_owned()
}
pub fn skills_empty_personal() -> String {
    rust_i18n::t!("skills.empty_personal").into_owned()
}
pub fn skills_empty_plugin() -> String {
    rust_i18n::t!("skills.empty_plugin").into_owned()
}
/// Chip text on plugin rows discovered through a registered
/// marketplace but not yet `/plugin install`-ed. Surfacing them lets
/// the user browse the catalog.
pub fn skills_plugin_available() -> String {
    rust_i18n::t!("skills.plugin_available").into_owned()
}
/// Tooltip / chip text when a project skill shadows a personal one.
pub fn skills_overrides_personal() -> String {
    rust_i18n::t!("skills.overrides_personal").into_owned()
}

/// Title strings for the CRUD modals.
pub fn skills_new_title() -> String {
    rust_i18n::t!("skills.new_title").into_owned()
}
pub fn skills_edit_title() -> String {
    rust_i18n::t!("skills.edit_title").into_owned()
}
pub fn skills_delete_title() -> String {
    rust_i18n::t!("skills.delete_title").into_owned()
}

/// Field labels in the CRUD modals.
pub fn skills_field_name() -> String {
    rust_i18n::t!("common.field_name").into_owned()
}
pub fn skills_field_scope() -> String {
    rust_i18n::t!("common.field_scope").into_owned()
}
pub fn skills_field_description() -> String {
    rust_i18n::t!("skills.field_description").into_owned()
}
pub fn skills_field_when_to_use() -> String {
    rust_i18n::t!("skills.field_when_to_use").into_owned()
}
pub fn skills_field_allowed_tools() -> String {
    rust_i18n::t!("skills.field_allowed_tools").into_owned()
}
pub fn skills_field_arg_hint() -> String {
    rust_i18n::t!("skills.field_arg_hint").into_owned()
}
pub fn skills_field_paths() -> String {
    rust_i18n::t!("skills.field_paths").into_owned()
}
pub fn skills_field_model() -> String {
    rust_i18n::t!("skills.field_model").into_owned()
}
pub fn skills_field_body() -> String {
    rust_i18n::t!("skills.field_body").into_owned()
}
pub fn skills_toggle_user_invocable() -> String {
    rust_i18n::t!("skills.toggle_user_invocable").into_owned()
}
pub fn skills_toggle_disable_model() -> String {
    rust_i18n::t!("skills.toggle_disable_model").into_owned()
}
pub fn skills_button_rename() -> String {
    rust_i18n::t!("common.btn_rename").into_owned()
}
pub fn skills_button_open_finder() -> String {
    rust_i18n::t!("skills.button_open_finder").into_owned()
}
pub fn skills_button_delete() -> String {
    rust_i18n::t!("common.btn_delete").into_owned()
}
/// Hover-only `[View]` action shown on plugin rows in place of Edit —
/// opens SKILL.md in the daruda file viewer.
pub fn skills_button_view() -> String {
    rust_i18n::t!("common.btn_view").into_owned()
}
/// `[Edit]` label on the skill row's hover-only action overlay.
pub fn skills_button_edit() -> String {
    rust_i18n::t!("common.btn_edit").into_owned()
}
pub fn skills_delete_body_prefix() -> String {
    rust_i18n::t!("skills.delete_body_prefix").into_owned()
}

/// Validation messages for the modal banner.
pub fn skills_name_empty() -> String {
    rust_i18n::t!("common.name_required").into_owned()
}
pub fn skills_name_invalid() -> String {
    rust_i18n::t!("skills.name_invalid").into_owned()
}
pub fn skills_name_leading() -> String {
    rust_i18n::t!("skills.name_leading").into_owned()
}
pub fn skills_name_too_long() -> String {
    rust_i18n::t!("skills.name_too_long").into_owned()
}
pub fn skills_name_duplicate() -> String {
    rust_i18n::t!("skills.name_duplicate").into_owned()
}
pub fn skills_description_too_long_hint() -> String {
    rust_i18n::t!("skills.description_too_long_hint").into_owned()
}
pub fn skills_no_project_hint() -> String {
    rust_i18n::t!("skills.no_project_hint").into_owned()
}

// ============================================================================
// Right panel — Tools tab (MCP servers)
// ============================================================================

/// Section heading for project-scope MCP servers (`<wt>/.mcp.json`).
pub fn mcp_project() -> String {
    rust_i18n::t!("common.section_project").into_owned()
}
/// Section heading for user-scope MCP servers (`~/.claude.json`
/// top-level `mcpServers`).
pub fn mcp_user() -> String {
    rust_i18n::t!("common.section_user").into_owned()
}
/// Section heading for local-scope MCP servers (`~/.claude.json`
/// `projects[<lane>].mcpServers`).
pub fn mcp_local() -> String {
    rust_i18n::t!("common.section_local").into_owned()
}
/// Header button — opens AddMcpServerModal.
pub fn mcp_new_button() -> String {
    rust_i18n::t!("mcp.new_button").into_owned()
}
/// Body when project scope has no servers and there is an active lane.
pub fn mcp_empty_project() -> String {
    rust_i18n::t!("mcp.empty_project").into_owned()
}
/// Body when project scope has no active lane (welcome-style window).
pub fn mcp_no_project_hint() -> String {
    rust_i18n::t!("mcp.no_project_hint").into_owned()
}
/// Body when user scope is empty.
pub fn mcp_empty_user() -> String {
    rust_i18n::t!("mcp.empty_user").into_owned()
}
/// Body when local scope is empty (active lane, no `projects[<lane>]`
/// servers in `~/.claude.json`).
pub fn mcp_empty_local() -> String {
    rust_i18n::t!("mcp.empty_local").into_owned()
}
/// Row status label — server has `"disabled": true` in config.
pub fn mcp_status_disabled() -> String {
    rust_i18n::t!("mcp.status_disabled").into_owned()
}
/// Row status label — required fields for the chosen transport are missing.
pub fn mcp_status_malformed() -> String {
    rust_i18n::t!("mcp.status_malformed").into_owned()
}

/// CRUD modal titles.
pub fn mcp_new_title() -> String {
    rust_i18n::t!("mcp.new_title").into_owned()
}
pub fn mcp_edit_title() -> String {
    rust_i18n::t!("mcp.edit_title").into_owned()
}
pub fn mcp_delete_title() -> String {
    rust_i18n::t!("mcp.delete_title").into_owned()
}

/// Field labels in the CRUD modals.
pub fn mcp_field_name() -> String {
    rust_i18n::t!("common.field_name").into_owned()
}
pub fn mcp_field_scope() -> String {
    rust_i18n::t!("common.field_scope").into_owned()
}
pub fn mcp_field_transport() -> String {
    rust_i18n::t!("mcp.field_transport").into_owned()
}
pub fn mcp_field_command() -> String {
    rust_i18n::t!("mcp.field_command").into_owned()
}
pub fn mcp_field_args() -> String {
    rust_i18n::t!("mcp.field_args").into_owned()
}
pub fn mcp_field_url() -> String {
    rust_i18n::t!("mcp.field_url").into_owned()
}
pub fn mcp_field_env() -> String {
    rust_i18n::t!("mcp.field_env").into_owned()
}
pub fn mcp_field_headers() -> String {
    rust_i18n::t!("mcp.field_headers").into_owned()
}

// Add-server modal — input placeholders. Format examples that hint the
// expected shape of each field; not shown on the edit modal (which
// pre-fills the existing values instead).
pub fn mcp_placeholder_name() -> String {
    rust_i18n::t!("mcp.placeholder_name").into_owned()
}
pub fn mcp_placeholder_command() -> String {
    rust_i18n::t!("mcp.placeholder_command").into_owned()
}
pub fn mcp_placeholder_args() -> String {
    rust_i18n::t!("mcp.placeholder_args").into_owned()
}
pub fn mcp_placeholder_url() -> String {
    rust_i18n::t!("mcp.placeholder_url").into_owned()
}
pub fn mcp_placeholder_env() -> String {
    rust_i18n::t!("mcp.placeholder_env").into_owned()
}
pub fn mcp_placeholder_headers() -> String {
    rust_i18n::t!("mcp.placeholder_headers").into_owned()
}

/// Buttons shared across CRUD modals + row hover actions.
pub fn mcp_button_add() -> String {
    rust_i18n::t!("common.btn_add").into_owned()
}
pub fn mcp_button_edit() -> String {
    rust_i18n::t!("common.btn_edit").into_owned()
}
pub fn mcp_button_delete() -> String {
    rust_i18n::t!("common.btn_delete").into_owned()
}
pub fn mcp_button_save() -> String {
    rust_i18n::t!("common.btn_save").into_owned()
}
pub fn mcp_button_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}
/// Displayed on the primary action button while the save is in flight.
pub fn mcp_saving_label() -> String {
    rust_i18n::t!("mcp.saving_label").into_owned()
}
/// Toggle label inside the AddModal / EditModal that maps to the
/// JSON `"disabled": true` key.
pub fn mcp_field_disabled() -> String {
    rust_i18n::t!("mcp.field_disabled").into_owned()
}

/// Confirm-modal body for the Delete flow.
pub fn mcp_delete_body_prefix() -> String {
    rust_i18n::t!("mcp.delete_body_prefix").into_owned()
}

/// Validation messages for the AddModal / EditModal banner.
pub fn mcp_name_empty() -> String {
    rust_i18n::t!("common.name_required").into_owned()
}
pub fn mcp_name_invalid() -> String {
    rust_i18n::t!("mcp.name_invalid").into_owned()
}
pub fn mcp_name_leading() -> String {
    rust_i18n::t!("mcp.name_leading").into_owned()
}
pub fn mcp_name_too_long() -> String {
    rust_i18n::t!("mcp.name_too_long").into_owned()
}
pub fn mcp_name_duplicate() -> String {
    rust_i18n::t!("mcp.name_duplicate").into_owned()
}
pub fn mcp_command_required() -> String {
    rust_i18n::t!("mcp.command_required").into_owned()
}
pub fn mcp_url_required() -> String {
    rust_i18n::t!("mcp.url_required").into_owned()
}
pub fn mcp_url_invalid() -> String {
    rust_i18n::t!("mcp.url_invalid").into_owned()
}
pub fn mcp_env_invalid() -> String {
    rust_i18n::t!("mcp.env_invalid").into_owned()
}

/// Transport option labels for the dropdown.
pub const MCP_TRANSPORT_STDIO: &str = "stdio";
pub const MCP_TRANSPORT_SSE: &str = "sse";
pub const MCP_TRANSPORT_HTTP: &str = "http";

/// Scope option labels for the dropdown.
pub fn mcp_scope_project() -> String {
    rust_i18n::t!("mcp.scope_project").into_owned()
}
pub fn mcp_scope_user() -> String {
    rust_i18n::t!("mcp.scope_user").into_owned()
}
pub fn mcp_scope_local() -> String {
    rust_i18n::t!("mcp.scope_local").into_owned()
}

// ============================================================================
// Error toast (Layer 1 of the error-reporting pipeline)
// ============================================================================

/// Severity glyphs in the toast leading icon slot.
pub const TOAST_ICON_INFO: &str = "ℹ";
pub const TOAST_ICON_WARNING: &str = "⚠";
pub const TOAST_ICON_ERROR: &str = "✕";

/// Action button labels.
pub fn toast_button_copy() -> String {
    rust_i18n::t!("common.btn_copy").into_owned()
}
pub fn toast_button_details() -> String {
    rust_i18n::t!("toast.button_details").into_owned()
}
/// Glyph for the dismiss ✕ button. Same character as the title-bar
/// pane close, separate constant so a future redesign can split them.
pub const TOAST_BUTTON_DISMISS: &str = "×";
/// Transient one-second affordance shown after `[Copy]` is clicked so
/// the user has visual confirmation the clipboard write happened.
pub fn toast_button_copied() -> String {
    rust_i18n::t!("common.btn_copied").into_owned()
}
/// `×N` repeat counter prefix.
pub const TOAST_REPEAT_PREFIX: &str = "×";

// ============================================================================
// Error-report Details modal (Layer 2)
// ============================================================================

/// Title prefix, e.g. `Error: PTY writer thread died`. The trailing
/// title is drawn from the underlying [`ErrorReport`] verbatim.
pub fn error_modal_title_prefix() -> String {
    rust_i18n::t!("error_modal.title_prefix").into_owned()
}

/// Footer button labels. `Copy report` writes the full plain-text
/// rendering (including system-info trailer) to the clipboard;
/// `Open log file` shells out to the system handler for the day's
/// NDJSON log; `Close` dismisses.
pub fn error_modal_button_copy() -> String {
    rust_i18n::t!("common.btn_copy").into_owned()
}
pub fn error_modal_button_copied() -> String {
    rust_i18n::t!("common.btn_copied").into_owned()
}
pub fn error_modal_button_open_log() -> String {
    rust_i18n::t!("error_modal.button_open_log").into_owned()
}
pub fn error_modal_button_close() -> String {
    rust_i18n::t!("common.btn_close").into_owned()
}

// ----------------------------------------------------------------
// Terminal annotations
// ----------------------------------------------------------------

/// Context-menu entry that opens the annotation create dialog.
pub fn terminal_annotation_action_add() -> String {
    rust_i18n::t!("terminal.annotation_action_add").into_owned()
}

/// Tooltip shown on the disabled "Add annotation" entry when the user
/// has no single-line selection yet.
pub fn terminal_annotation_action_add_disabled_tooltip() -> String {
    rust_i18n::t!("terminal.annotation_action_add_disabled_tooltip").into_owned()
}

/// Context-menu entry that removes the annotation under the click.
pub fn terminal_annotation_action_delete() -> String {
    rust_i18n::t!("terminal.annotation_action_delete").into_owned()
}

/// Title of the annotation dialog when adding a new annotation.
pub fn terminal_annotation_dialog_title_create() -> String {
    rust_i18n::t!("terminal.annotation_dialog_title_create").into_owned()
}

/// Title of the annotation dialog when editing an existing annotation.
pub fn terminal_annotation_dialog_title_edit() -> String {
    rust_i18n::t!("terminal.annotation_dialog_title_edit").into_owned()
}

/// Placeholder shown inside the annotation-text input.
pub fn terminal_annotation_placeholder() -> String {
    rust_i18n::t!("terminal.annotation_placeholder").into_owned()
}

/// Toast/modal title when the workspace can no longer find the pane
/// that an annotation operation targeted.
pub fn terminal_annotation_err_pane_missing_title() -> String {
    rust_i18n::t!("terminal.annotation_err_pane_missing_title").into_owned()
}

/// User-facing message body paired with
/// [`terminal_annotation_err_pane_missing_title`].
pub fn terminal_annotation_err_pane_missing_message() -> String {
    rust_i18n::t!("terminal.annotation_err_pane_missing_message").into_owned()
}

/// Toast/modal title when the underlying session rejected an annotation
/// mutation.
pub fn terminal_annotation_err_operation_failed_title() -> String {
    rust_i18n::t!("terminal.annotation_err_operation_failed_title").into_owned()
}

/// User-facing message body paired with
/// [`terminal_annotation_err_operation_failed_title`].
pub fn terminal_annotation_err_operation_failed_message() -> String {
    rust_i18n::t!("terminal.annotation_err_operation_failed_message").into_owned()
}

/// Annotation dialog "Save" button — reuses the shared `common.btn_save`
/// key but lives in the annotation namespace so future localizations can
/// override the label without touching unrelated dialogs.
pub fn annotation_dialog_save() -> String {
    rust_i18n::t!("common.btn_save").into_owned()
}

/// Annotation dialog "Cancel" button — see [`annotation_dialog_save`].
pub fn annotation_dialog_cancel() -> String {
    rust_i18n::t!("common.btn_cancel").into_owned()
}

pub fn create_lane_err_branch_required() -> String {
    rust_i18n::t!("create_lane.err_branch_required").into_owned()
}

pub fn create_lane_err_branch_invalid() -> String {
    rust_i18n::t!("create_lane.err_branch_invalid").into_owned()
}

pub fn create_lane_err_no_active_project() -> String {
    rust_i18n::t!("create_lane.err_no_active_project").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Recursively collect dotted key paths for every scalar leaf in a
    /// YAML mapping tree (e.g. `common.btn_cancel`).
    fn collect_locale_keys(
        value: &serde_yaml::Value,
        prefix: &str,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        if let serde_yaml::Value::Mapping(map) = value {
            for (k, v) in map {
                let key = k.as_str().unwrap_or("<non-string-key>");
                let path = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_locale_keys(v, &path, out);
            }
        } else {
            out.insert(prefix.to_string());
        }
    }

    /// Every i18n key in `en.yml` must have a counterpart in `ko.yml`
    /// and vice versa. A missing translation silently renders the raw
    /// key string at runtime, so key drift must fail the build.
    #[test]
    fn locale_en_ko_key_parity() {
        let en: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../../locales/en.yml")).unwrap();
        let ko: serde_yaml::Value =
            serde_yaml::from_str(include_str!("../../locales/ko.yml")).unwrap();

        let mut en_keys = std::collections::BTreeSet::new();
        let mut ko_keys = std::collections::BTreeSet::new();
        collect_locale_keys(&en, "", &mut en_keys);
        collect_locale_keys(&ko, "", &mut ko_keys);

        let missing_in_ko: Vec<_> = en_keys.difference(&ko_keys).collect();
        let missing_in_en: Vec<_> = ko_keys.difference(&en_keys).collect();
        assert!(
            missing_in_ko.is_empty() && missing_in_en.is_empty(),
            "i18n key drift between en.yml and ko.yml:\n  missing in ko.yml: {missing_in_ko:?}\n  missing in en.yml: {missing_in_en:?}"
        );
    }

    #[test]
    fn telegram_reply_ack_is_non_empty() {
        assert!(!super::agent_notification_telegram_reply_ack().is_empty());
    }

    #[test]
    fn duration_below_minute_renders_seconds() {
        assert_eq!(format_duration_compact(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration_compact(Duration::from_secs(42)), "42s");
        assert_eq!(format_duration_compact(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn duration_under_hour_renders_minutes_and_seconds() {
        assert_eq!(format_duration_compact(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_duration_compact(Duration::from_secs(63)), "1m 03s");
        assert_eq!(
            format_duration_compact(Duration::from_secs(3_599)),
            "59m 59s"
        );
    }

    #[test]
    fn duration_over_hour_renders_hours_and_minutes() {
        assert_eq!(
            format_duration_compact(Duration::from_secs(3_600)),
            "1h 00m"
        );
        assert_eq!(
            format_duration_compact(Duration::from_secs(8_115)),
            "2h 15m"
        );
    }

    #[test]
    fn reset_countdown_zero_renders_now() {
        assert_eq!(format_reset_countdown(Duration::from_secs(0)), "Resets now");
    }

    #[test]
    fn reset_countdown_sub_minute_renders_lt1m() {
        // 1..59 seconds collapses to "<1m" so the user doesn't see
        // "Resets in 0m" linger for a whole minute right before
        // the rolling window resets.
        assert_eq!(
            format_reset_countdown(Duration::from_secs(1)),
            "Resets in <1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(30)),
            "Resets in <1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(59)),
            "Resets in <1m"
        );
    }

    #[test]
    fn reset_countdown_under_hour_renders_minutes() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(60)),
            "Resets in 1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(59 * 60)),
            "Resets in 59m"
        );
    }

    #[test]
    fn reset_countdown_under_day_renders_hours_and_minutes() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(3_600)),
            "Resets in 1h 0m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(2 * 3_600 + 14 * 60)),
            "Resets in 2h 14m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(23 * 3_600 + 59 * 60)),
            "Resets in 23h 59m"
        );
    }

    #[test]
    fn reset_countdown_over_day_renders_days_and_hours() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(24 * 3_600)),
            "Resets in 1d 0h"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(3 * 24 * 3_600 + 7 * 3_600)),
            "Resets in 3d 7h"
        );
    }

    #[test]
    fn service_status_label_operational_ignores_description() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::None,
            description: "stale message".into(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Operational");
    }

    #[test]
    fn service_status_label_uses_description_for_incidents() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Minor,
            description: "Increased 4xx errors on /messages".into(),
            fetched_at: None,
        };
        assert_eq!(
            service_status_label(&s),
            "Increased 4xx errors on /messages"
        );
    }

    #[test]
    fn service_status_label_falls_back_to_default_when_description_empty() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Major,
            description: String::new(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Partial outage");
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Critical,
            description: String::new(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Major outage");
    }

    #[test]
    fn service_status_label_unknown_ignores_description() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Unknown,
            description: "garbage".into(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Status unavailable");
    }

    // ----------------------------------------------------------------
    // bottom_input_placeholder_for_context
    // ----------------------------------------------------------------

    #[test]
    fn placeholder_terminal_pane() {
        // Terminal focus always returns the terminal placeholder regardless
        // of the modifier-to-send flag.
        assert_eq!(
            bottom_input_placeholder_for_context(false, None, false),
            bottom_input_placeholder(),
        );
        assert_eq!(
            bottom_input_placeholder_for_context(false, None, true),
            bottom_input_placeholder(),
        );
        // mode_name is ignored for terminal panes.
        assert_eq!(
            bottom_input_placeholder_for_context(false, Some("Auto"), false),
            bottom_input_placeholder(),
        );
        assert_eq!(
            bottom_input_placeholder_for_context(false, Some("Auto"), true),
            bottom_input_placeholder(),
        );
    }

    #[test]
    fn placeholder_agent_no_modes() {
        // Agent focus without mode info: hint the submit key only.
        assert_eq!(
            bottom_input_placeholder_for_context(true, None, false),
            bottom_input_agent_placeholder(),
        );
        assert_eq!(
            bottom_input_placeholder_for_context(true, None, true),
            bottom_input_agent_modifier_placeholder(),
        );
    }

    #[test]
    fn placeholder_agent_with_mode() {
        // Agent focus with an active mode: hint mode name + submit key.
        assert_eq!(
            bottom_input_placeholder_for_context(true, Some("Auto"), false),
            bottom_input_agent_mode_placeholder("Auto"),
        );
        assert_eq!(
            bottom_input_placeholder_for_context(true, Some("Auto"), true),
            bottom_input_agent_mode_modifier_placeholder("Auto"),
        );
    }
}
