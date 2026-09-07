//! The palette's command table — `PaletteEntry` and the
//! `PALETTE_ENTRIES` array. Pure const data, split out from the
//! state machine and overlay in [`super`] so each half stays one
//! responsibility.

use crate::surface::strings as s;

/// A single entry in the command palette.
#[derive(Clone)]
pub(in crate::workspace) struct PaletteEntry {
    /// Action identifier (snake_case, matches action_map).
    pub id: &'static str,
    /// Human-readable label shown in the palette.
    pub label: fn() -> String,
    /// Keyboard shortcut hint (displayed right-aligned).
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
        shortcut: "Cmd+P",
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
        shortcut: "Cmd+,",
    },
    PaletteEntry {
        id: "open_settings.general",
        label: s::command_settings_general,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.font",
        label: s::command_settings_font,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.cursor",
        label: s::command_settings_cursor,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.shell",
        label: s::command_settings_shell,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.window",
        label: s::command_settings_window,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.terminal",
        label: s::command_settings_terminal,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.dock",
        label: s::command_settings_dock,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.clipboard",
        label: s::command_settings_clipboard,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.external_editor",
        label: s::command_settings_external_editor,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.notifications",
        label: s::command_settings_notifications,
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.keymap",
        label: s::command_settings_keymap,
        shortcut: "",
    },
    PaletteEntry {
        id: "new_tab",
        label: s::command_new_tab,
        shortcut: "Cmd+T",
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
        shortcut: "Cmd+W",
    },
    PaletteEntry {
        id: "close_tab",
        label: s::command_close_tab,
        shortcut: "",
    },
    PaletteEntry {
        id: "split_right",
        label: s::command_split_right,
        shortcut: "Cmd+D",
    },
    PaletteEntry {
        id: "split_down",
        label: s::command_split_down,
        shortcut: "Cmd+Shift+D",
    },
    PaletteEntry {
        id: "next_tab",
        label: s::command_next_tab,
        shortcut: "Ctrl+Tab",
    },
    PaletteEntry {
        id: "prev_tab",
        label: s::command_previous_tab,
        shortcut: "Ctrl+Shift+Tab",
    },
    PaletteEntry {
        id: "toggle_left_dock",
        label: s::command_toggle_left_dock,
        shortcut: "Cmd+B",
    },
    PaletteEntry {
        id: "toggle_bottom_dock",
        label: s::command_toggle_bottom_panel,
        shortcut: "Cmd+J",
    },
    PaletteEntry {
        id: "toggle_right_dock",
        label: s::command_toggle_right_dock,
        shortcut: "Cmd+Shift+B",
    },
    PaletteEntry {
        id: "focus_next_pane",
        label: s::command_focus_next_pane,
        shortcut: "Cmd+]",
    },
    PaletteEntry {
        id: "focus_prev_pane",
        label: s::command_focus_previous_pane,
        shortcut: "Cmd+[",
    },
    PaletteEntry {
        id: "focus_pane_left",
        label: s::command_focus_pane_left,
        shortcut: "Cmd+Alt+Left",
    },
    PaletteEntry {
        id: "focus_pane_right",
        label: s::command_focus_pane_right,
        shortcut: "Cmd+Alt+Right",
    },
    PaletteEntry {
        id: "focus_pane_up",
        label: s::command_focus_pane_up,
        shortcut: "Cmd+Alt+Up",
    },
    PaletteEntry {
        id: "focus_pane_down",
        label: s::command_focus_pane_down,
        shortcut: "Cmd+Alt+Down",
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
        shortcut: "Cmd+C",
    },
    PaletteEntry {
        id: "paste",
        label: s::command_paste,
        shortcut: "Cmd+V",
    },
    PaletteEntry {
        id: "select_all",
        label: s::command_select_all,
        shortcut: "Cmd+A",
    },
    PaletteEntry {
        id: "activate_lane_1",
        label: s::command_activate_lane_1,
        shortcut: "Cmd+Ctrl+1",
    },
    PaletteEntry {
        id: "activate_lane_2",
        label: s::command_activate_lane_2,
        shortcut: "Cmd+Ctrl+2",
    },
    PaletteEntry {
        id: "activate_lane_3",
        label: s::command_activate_lane_3,
        shortcut: "Cmd+Ctrl+3",
    },
    PaletteEntry {
        id: "activate_lane_4",
        label: s::command_activate_lane_4,
        shortcut: "Cmd+Ctrl+4",
    },
    PaletteEntry {
        id: "activate_lane_5",
        label: s::command_activate_lane_5,
        shortcut: "Cmd+Ctrl+5",
    },
    PaletteEntry {
        id: "activate_lane_6",
        label: s::command_activate_lane_6,
        shortcut: "Cmd+Ctrl+6",
    },
    PaletteEntry {
        id: "activate_lane_7",
        label: s::command_activate_lane_7,
        shortcut: "Cmd+Ctrl+7",
    },
    PaletteEntry {
        id: "activate_lane_8",
        label: s::command_activate_lane_8,
        shortcut: "Cmd+Ctrl+8",
    },
    PaletteEntry {
        id: "activate_lane_9",
        label: s::command_activate_lane_9,
        shortcut: "Cmd+Ctrl+9",
    },
    PaletteEntry {
        id: "open_folder",
        label: s::command_open_project,
        shortcut: "Cmd+O",
    },
    PaletteEntry {
        id: "new_group",
        label: s::command_new_group,
        shortcut: "Cmd+Shift+N",
    },
    PaletteEntry {
        id: "rename_project",
        label: s::command_rename_project,
        shortcut: "Cmd+Shift+R",
    },
    PaletteEntry {
        id: "move_project_to_group",
        label: s::command_move_project_to_group,
        shortcut: "Cmd+Shift+M",
    },
    PaletteEntry {
        id: "close_project",
        label: s::command_close_project,
        shortcut: "Cmd+Shift+W",
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
        shortcut: "Cmd+Shift+.",
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
        shortcut: "Cmd+Shift+H",
    },
    PaletteEntry {
        id: "quit",
        label: s::command_quit,
        shortcut: "Cmd+Q",
    },
];
