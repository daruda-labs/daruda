//! The palette's command table — `PaletteEntry` and the
//! `PALETTE_ENTRIES` array. Pure const data, split out from the
//! state machine and overlay in [`super`] so each half stays one
//! responsibility.

use crate::surface::keybindings as k;
use crate::surface::strings as s;

/// A single entry in the command palette.
#[derive(Clone)]
pub(in crate::workspace) struct PaletteEntry {
    /// Action identifier (snake_case, matches action_map).
    pub id: &'static str,
    /// Human-readable label shown in the palette.
    pub label: fn() -> String,
    /// Default chord (`surface::keybindings`), rendered through
    /// `surface::shortcut_display` so each platform reads its own modifiers.
    pub shortcut: &'static str,
}

/// All available palette entries, grouped by domain so a new entry lands next
/// to its siblings. The order carries no ranking — `label` is an i18n function,
/// so what the reader sees is resolved at match time and ordered there.
///
/// Per-section settings entries use the dotted form
/// `open_settings.<slug>` matching the keybinding-override syntax in
/// `surface::action_map`. The bare `open_settings` id resolves to the
/// default page (General) so an empty-arg keybinding still works.
pub(in crate::workspace) const PALETTE_ENTRIES: &[PaletteEntry] = &[
    PaletteEntry {
        id: "toggle_lane_switcher",
        label: s::command_switch_lane,
        shortcut: k::SHORTCUT_LANE_SWITCHER,
    },
    PaletteEntry {
        id: "run_flow",
        label: s::command_run_flow,
        shortcut: "",
    },
    PaletteEntry {
        id: "validate_flow",
        label: s::command_check_flow,
        shortcut: "",
    },
    PaletteEntry {
        id: "show_flow_graph",
        label: s::command_show_flow_graph,
        shortcut: "",
    },
    PaletteEntry {
        id: "reload_flow_graph",
        label: s::command_reload_flow_graph,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings",
        label: s::command_settings,
        shortcut: k::SHORTCUT_SETTINGS,
    },
    // These three are otherwise menu-only, which puts them out of reach
    // wherever gpui does not draw a menu bar.
    PaletteEntry {
        id: "open_project_config",
        label: s::command_open_project_config,
        shortcut: "",
    },
    PaletteEntry {
        id: "edit_window_title",
        label: s::command_edit_window_title,
        shortcut: "",
    },
    PaletteEntry {
        id: "zoom_window",
        label: s::command_zoom_window,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.general",
        label: s::command_settings_general,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.appearance",
        label: s::command_settings_appearance,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.font",
        label: s::command_settings_font,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.terminal",
        label: s::command_settings_terminal,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.workspace",
        label: s::command_settings_workspace,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.keymap",
        label: s::command_settings_keymap,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.agent",
        label: s::command_settings_agent,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.orchestrator",
        label: s::command_settings_orchestrator,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.session_hosts",
        label: s::command_settings_session_hosts,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.accounts",
        label: s::command_settings_accounts,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.notifications",
        label: s::command_settings_notifications,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.remote_control",
        label: s::command_settings_remote_control,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.plugin",
        label: s::command_settings_plugin,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.about",
        label: s::command_settings_about,
        shortcut: "",
    },
    PaletteEntry {
        id: "new_tab",
        label: s::command_new_tab,
        shortcut: k::SHORTCUT_NEW_TAB,
    },
    PaletteEntry {
        id: "new_task",
        label: s::command_new_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "new_agent_chat",
        label: s::command_new_agent_chat,
        shortcut: "",
    },
    PaletteEntry {
        id: "edit_task",
        label: s::command_edit_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "start_task",
        label: s::command_start_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "cancel_task",
        label: s::command_cancel_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "reopen_task",
        label: s::command_reopen_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "retry_task",
        label: s::command_retry_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "delete_task",
        label: s::command_delete_task,
        shortcut: "",
    },
    PaletteEntry {
        id: "close_pane",
        label: s::command_close_pane,
        shortcut: k::SHORTCUT_CLOSE_PANE,
    },
    PaletteEntry {
        id: "close_tab",
        label: s::command_close_tab,
        shortcut: "",
    },
    PaletteEntry {
        id: "split_right",
        label: s::command_split_right,
        shortcut: k::SHORTCUT_SPLIT_RIGHT,
    },
    PaletteEntry {
        id: "split_down",
        label: s::command_split_down,
        shortcut: k::SHORTCUT_SPLIT_DOWN,
    },
    PaletteEntry {
        id: "next_tab",
        label: s::command_next_tab,
        shortcut: k::SHORTCUT_NEXT_TAB,
    },
    PaletteEntry {
        id: "prev_tab",
        label: s::command_previous_tab,
        shortcut: k::SHORTCUT_PREV_TAB,
    },
    PaletteEntry {
        id: "toggle_left_dock",
        label: s::command_toggle_left_dock,
        shortcut: k::SHORTCUT_TOGGLE_LEFT_DOCK,
    },
    PaletteEntry {
        id: "toggle_git_changes_focus",
        label: s::command_toggle_git_changes_focus,
        shortcut: k::SHORTCUT_TOGGLE_GIT_CHANGES_FOCUS,
    },
    PaletteEntry {
        id: "toggle_files_focus",
        label: s::command_toggle_files_focus,
        shortcut: k::SHORTCUT_TOGGLE_FILES_FOCUS,
    },
    PaletteEntry {
        id: "toggle_bottom_dock",
        label: s::command_toggle_bottom_panel,
        shortcut: k::SHORTCUT_TOGGLE_BOTTOM_DOCK,
    },
    PaletteEntry {
        id: "toggle_right_dock",
        label: s::command_toggle_right_dock,
        shortcut: k::SHORTCUT_TOGGLE_RIGHT_DOCK,
    },
    PaletteEntry {
        id: "focus_next_pane",
        label: s::command_focus_next_pane,
        shortcut: k::SHORTCUT_FOCUS_NEXT_PANE,
    },
    PaletteEntry {
        id: "focus_prev_pane",
        label: s::command_focus_previous_pane,
        shortcut: k::SHORTCUT_FOCUS_PREV_PANE,
    },
    PaletteEntry {
        id: "focus_pane_left",
        label: s::command_focus_pane_left,
        shortcut: k::SHORTCUT_FOCUS_PANE_LEFT,
    },
    PaletteEntry {
        id: "focus_pane_right",
        label: s::command_focus_pane_right,
        shortcut: k::SHORTCUT_FOCUS_PANE_RIGHT,
    },
    PaletteEntry {
        id: "focus_pane_up",
        label: s::command_focus_pane_up,
        shortcut: k::SHORTCUT_FOCUS_PANE_UP,
    },
    PaletteEntry {
        id: "focus_pane_down",
        label: s::command_focus_pane_down,
        shortcut: k::SHORTCUT_FOCUS_PANE_DOWN,
    },
    PaletteEntry {
        id: "move_tab_left",
        label: s::command_move_tab_left,
        shortcut: "",
    },
    PaletteEntry {
        id: "move_tab_right",
        label: s::command_move_tab_right,
        shortcut: "",
    },
    PaletteEntry {
        id: "copy",
        label: s::command_copy,
        shortcut: k::SHORTCUT_COPY,
    },
    PaletteEntry {
        id: "paste",
        label: s::command_paste,
        shortcut: k::SHORTCUT_PASTE,
    },
    PaletteEntry {
        id: "select_all",
        label: s::command_select_all,
        shortcut: k::SHORTCUT_SELECT_ALL,
    },
    PaletteEntry {
        id: "activate_lane_1",
        label: s::command_activate_lane_1,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_1,
    },
    PaletteEntry {
        id: "activate_lane_2",
        label: s::command_activate_lane_2,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_2,
    },
    PaletteEntry {
        id: "activate_lane_3",
        label: s::command_activate_lane_3,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_3,
    },
    PaletteEntry {
        id: "activate_lane_4",
        label: s::command_activate_lane_4,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_4,
    },
    PaletteEntry {
        id: "activate_lane_5",
        label: s::command_activate_lane_5,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_5,
    },
    PaletteEntry {
        id: "activate_lane_6",
        label: s::command_activate_lane_6,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_6,
    },
    PaletteEntry {
        id: "activate_lane_7",
        label: s::command_activate_lane_7,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_7,
    },
    PaletteEntry {
        id: "activate_lane_8",
        label: s::command_activate_lane_8,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_8,
    },
    PaletteEntry {
        id: "activate_lane_9",
        label: s::command_activate_lane_9,
        shortcut: k::SHORTCUT_ACTIVATE_LANE_9,
    },
    PaletteEntry {
        id: "open_folder",
        label: s::command_open_project,
        shortcut: k::SHORTCUT_OPEN_FOLDER,
    },
    PaletteEntry {
        id: "new_group",
        label: s::command_new_group,
        shortcut: k::SHORTCUT_NEW_GROUP,
    },
    PaletteEntry {
        id: "rename_project",
        label: s::command_rename_project,
        shortcut: k::SHORTCUT_RENAME_PROJECT,
    },
    PaletteEntry {
        id: "move_project_to_group",
        label: s::command_move_project_to_group,
        shortcut: k::SHORTCUT_MOVE_PROJECT_TO_GROUP,
    },
    PaletteEntry {
        id: "close_project",
        label: s::command_close_project,
        shortcut: k::SHORTCUT_CLOSE_PROJECT,
    },
    PaletteEntry {
        id: "show_left_dock_lanes",
        label: s::command_show_lanes,
        shortcut: "",
    },
    PaletteEntry {
        id: "show_left_dock_git",
        label: s::command_show_git_changes,
        shortcut: "",
    },
    PaletteEntry {
        id: "show_left_dock_files",
        label: s::command_show_files,
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_usage",
        label: s::command_right_panel_usage,
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_skills",
        label: s::command_right_panel_skills,
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_tools",
        label: s::command_right_panel_tools,
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_tasks",
        label: s::command_right_panel_tasks,
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_flows",
        label: s::command_right_panel_flows,
        shortcut: "",
    },
    PaletteEntry {
        id: "new_skill",
        label: s::command_skills_new_skill,
        shortcut: "",
    },
    PaletteEntry {
        id: "refresh_git_status",
        label: s::command_refresh_git_status,
        shortcut: "",
    },
    PaletteEntry {
        id: "files_toggle_hidden",
        label: s::command_files_toggle_hidden,
        shortcut: k::SHORTCUT_FILES_TOGGLE_HIDDEN,
    },
    PaletteEntry {
        id: "files_refresh",
        label: s::command_files_refresh,
        shortcut: "",
    },
    PaletteEntry {
        id: "commit_changes",
        label: s::command_commit_changes,
        shortcut: "",
    },
    PaletteEntry {
        id: "push_changes",
        label: s::command_push_changes,
        shortcut: "",
    },
    PaletteEntry {
        id: "install_claude_hooks",
        label: s::command_claude_install_hooks,
        shortcut: "",
    },
    PaletteEntry {
        id: "uninstall_claude_hooks",
        label: s::command_claude_uninstall_hooks,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_command_history",
        label: s::command_open_command_history,
        shortcut: k::SHORTCUT_OPEN_COMMAND_HISTORY,
    },
    PaletteEntry {
        id: "quit",
        label: s::command_quit,
        shortcut: k::SHORTCUT_QUIT,
    },
];
