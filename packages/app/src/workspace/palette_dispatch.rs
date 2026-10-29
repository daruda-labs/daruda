//! Command-palette key handling and dispatch: maps the focused palette
//! row to the action it names.

use crate::workspace::{ContinueFlowSession, OpenFlowHistory, ReturnToFlowExecution};

use gpui::{Context, Window};

use super::command::picker::PickerKey;
use super::command::picker_key::picker_keystroke;
use super::{
    ClosePane, CloseTab, EditTask, EditWindowTitle, FocusNextPane, FocusPaneDown, FocusPaneLeft,
    FocusPaneRight, FocusPaneUp, FocusPrevPane, InstallAgentHooks, MoveActiveProjectToGroup,
    MoveTabLeft, MoveTabRight, NewGroup, NewSkill, NewTab, NewTask, NextTab, OpenAgentChat,
    OpenCommandHistory, OpenProjectConfig, OpenSettings, PrevTab, ReloadFlowGraph,
    RenameActiveProject, RunFlow, ShowFlowGraph, ShowLeftDockFiles, ShowLeftDockGit,
    ShowLeftDockLanes, SplitDown, SplitRight, SwitchRightPanelFlows, SwitchRightPanelSkills,
    SwitchRightPanelTasks, SwitchRightPanelTools, SwitchRightPanelUsage, ToggleBottomDock,
    ToggleCommandPalette, ToggleFilesFocus, ToggleGitChangesFocus, ToggleLaneSwitcher,
    ToggleLeftDock, ToggleRightDock, UninstallAgentHooks, ValidateFlow, Workspace, ZoomWindow,
};

impl Workspace {
    /// Map a keystroke onto the open command palette. The
    /// `capture_key_down` attachment stays in the render path; the rules
    /// live here.
    pub(in crate::workspace) fn on_palette_key(
        &mut self,
        ev: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlays.palette.is_open {
            return;
        }
        let Some((key, ch)) = picker_keystroke(ev) else {
            return;
        };
        let visible_len = self.overlays.palette.visible().len();
        match self.overlays.palette.picker.on_key(key, ch, visible_len) {
            PickerKey::Confirm => self.execute_palette_action(window, cx),
            PickerKey::Dismiss => self.close_command_palette(cx),
            PickerKey::Changed => cx.notify(),
            PickerKey::Unchanged => {}
        }
        cx.stop_propagation();
    }

    /// Close the palette without running anything — the backdrop click.
    pub(in crate::workspace) fn close_command_palette(&mut self, cx: &mut Context<Self>) {
        self.overlays.palette.close();
        cx.notify();
    }

    /// Run the row the mouse named. Focusing it first is what makes a click
    /// the same gesture as arrowing there and pressing Enter.
    pub(in crate::workspace) fn pick_palette_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlays.palette.picker.focus(ix);
        self.execute_palette_action(window, cx);
    }

    /// Execute the currently focused palette action and close.
    pub(crate) fn execute_palette_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action_id = self.overlays.palette.focused_action_id();
        self.close_command_palette(cx);

        if let Some(id) = action_id {
            match id {
                "new_tab" => self.on_new_tab(&NewTab, window, cx),
                "new_task" => self.on_new_task(&NewTask, window, cx),
                "new_agent_chat" => self.on_open_agent_chat(&OpenAgentChat, window, cx),
                "edit_task" => self.on_edit_task(&EditTask, window, cx),
                "start_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Start,
                    window,
                    cx,
                ),
                "cancel_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Cancel,
                    window,
                    cx,
                ),
                "reopen_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Reopen,
                    window,
                    cx,
                ),
                "retry_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Retry,
                    window,
                    cx,
                ),
                "delete_task" => self.open_task_picker_modal(
                    crate::workspace::right_dock::task_picker_modal::TaskPickAction::Delete,
                    window,
                    cx,
                ),
                "close_pane" => self.on_close_pane(&ClosePane, window, cx),
                "close_tab" => self.on_close_tab(&CloseTab, window, cx),
                "split_right" => self.on_split_right(&SplitRight, window, cx),
                "split_down" => self.on_split_down(&SplitDown, window, cx),
                "next_tab" => self.on_next_tab(&NextTab, window, cx),
                "prev_tab" => self.on_prev_tab(&PrevTab, window, cx),
                "reset_left_dock_width" => {
                    self.on_reset_left_dock_width(&super::ResetLeftDockWidth, window, cx)
                }
                "toggle_left_dock" => self.on_toggle_left_dock(&ToggleLeftDock, window, cx),
                "toggle_git_changes_focus" => {
                    self.on_toggle_git_changes_focus(&ToggleGitChangesFocus, window, cx);
                }
                "toggle_files_focus" => {
                    self.on_toggle_files_focus(&ToggleFilesFocus, window, cx);
                }
                "toggle_bottom_dock" => self.on_toggle_bottom_dock(&ToggleBottomDock, window, cx),
                "toggle_right_dock" => self.on_toggle_right_dock(&ToggleRightDock, window, cx),
                "toggle_command_palette" => {
                    self.on_toggle_command_palette(&ToggleCommandPalette, window, cx);
                }
                "toggle_lane_switcher" => {
                    self.on_toggle_lane_switcher(&ToggleLaneSwitcher, window, cx);
                }
                "open_flow_history" => self.on_open_flow_history(&OpenFlowHistory, window, cx),
                "continue_flow_session" => {
                    self.on_continue_flow_session(&ContinueFlowSession, window, cx)
                }
                "return_to_flow_execution" => {
                    self.on_return_to_flow_execution(&ReturnToFlowExecution, window, cx)
                }
                "run_flow" => self.on_run_flow(&RunFlow, window, cx),
                "validate_flow" => self.on_validate_flow(&ValidateFlow, window, cx),
                "show_flow_graph" => self.on_show_flow_graph(&ShowFlowGraph, window, cx),
                "reload_flow_graph" => self.on_reload_flow_graph(&ReloadFlowGraph, window, cx),
                "focus_next_pane" => self.on_focus_next_pane(&FocusNextPane, window, cx),
                "focus_prev_pane" => self.on_focus_prev_pane(&FocusPrevPane, window, cx),
                "focus_pane_left" => self.on_focus_pane_left(&FocusPaneLeft, window, cx),
                "focus_pane_right" => self.on_focus_pane_right(&FocusPaneRight, window, cx),
                "focus_pane_up" => self.on_focus_pane_up(&FocusPaneUp, window, cx),
                "focus_pane_down" => self.on_focus_pane_down(&FocusPaneDown, window, cx),
                "move_tab_left" => self.on_move_tab_left(&MoveTabLeft, window, cx),
                "move_tab_right" => self.on_move_tab_right(&MoveTabRight, window, cx),
                "show_left_dock_lanes" => {
                    self.on_show_left_dock_worktrees(&ShowLeftDockLanes, window, cx);
                }
                "show_left_dock_git" => {
                    self.on_show_left_dock_git(&ShowLeftDockGit, window, cx);
                }
                "show_left_dock_files" => {
                    self.on_show_left_dock_files(&ShowLeftDockFiles, window, cx);
                }
                "switch_right_panel_usage" => {
                    self.on_switch_right_panel_usage(&SwitchRightPanelUsage, window, cx);
                }
                "switch_right_panel_skills" => {
                    self.on_switch_right_panel_skills(&SwitchRightPanelSkills, window, cx);
                }
                "switch_right_panel_tools" => {
                    self.on_switch_right_panel_tools(&SwitchRightPanelTools, window, cx);
                }
                "switch_right_panel_tasks" => {
                    self.on_switch_right_panel_tasks(&SwitchRightPanelTasks, window, cx);
                }
                "switch_right_panel_flows" => {
                    self.on_switch_right_panel_flows(&SwitchRightPanelFlows, window, cx);
                }
                "new_skill" => {
                    self.on_new_skill(&NewSkill, window, cx);
                }
                "copy" | "paste" | "select_all" => {
                    // These are terminal-level actions dispatched via the
                    // focused pane's TerminalView, not workspace actions.
                    // The GPUI action dispatch already handles them when the
                    // terminal has focus, so no-op here is correct.
                }
                "install_claude_hooks" => {
                    self.on_install_agent_hooks(&InstallAgentHooks, window, cx);
                }
                "uninstall_claude_hooks" => {
                    self.on_uninstall_agent_hooks(&UninstallAgentHooks, window, cx);
                }
                "open_command_history" => {
                    self.on_open_command_history(&OpenCommandHistory, window, cx);
                }
                "open_folder" => {
                    window.dispatch_action(Box::new(crate::OpenFolder), cx);
                }
                "close_project" => {
                    window.dispatch_action(Box::new(crate::CloseProject), cx);
                }
                "new_group" => self.on_new_group(&NewGroup, window, cx),
                "rename_project" => {
                    self.on_rename_active_project(&RenameActiveProject, window, cx);
                }
                "move_project_to_group" => {
                    self.on_move_active_project_to_group(&MoveActiveProjectToGroup, window, cx);
                }
                "open_project_config" => {
                    self.on_open_project_config(&OpenProjectConfig, window, cx);
                }
                "edit_window_title" => {
                    self.on_edit_window_title(&EditWindowTitle, window, cx);
                }
                "zoom_window" => self.on_zoom_window(&ZoomWindow, window, cx),
                "quit" => Self::request_quit(cx),
                _ => {
                    // Per-section settings entries follow the
                    // `open_settings.<slug>` pattern used by the
                    // keybinding-override path. Bare "open_settings"
                    // resolves to the default page.
                    if id == "open_settings" {
                        self.on_open_settings(
                            &OpenSettings(daruda_config::BuiltinSection::default()),
                            window,
                            cx,
                        );
                    } else if let Some(slug) = id.strip_prefix("open_settings.")
                        && let Some(section) = daruda_config::BuiltinSection::from_slug(slug)
                    {
                        self.on_open_settings(&OpenSettings(section), window, cx);
                    }
                }
            }
        }
    }
}
