//! Trivial `on_*` action-handler shims that translate a GPUI action
//! into a one-liner business call.
//!
//! Each handler is the receiving end of a `KeyBinding::new(SHORTCUT_*,
//! Action, ctx)` registered in `main.rs` (or in the slot tables) and a
//! matching `.on_action(cx.listener(Self::on_xxx))` registration in
//! `workspace/render.rs`. All real logic lives in dedicated ops files
//! (`split_focused_pane` in `mod.rs`, `move_files_selection` in
//! `file_tree_ops.rs`, etc.); this file only forwards.
//!
//! New handlers go here unless they carry meaningful logic of their
//! own — at which point they belong with the rest of that domain
//! (lane / dock / git / files / etc.).

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Focusable as _, Window};

use super::Workspace;
use super::{
    CloseOtherTabs, ClosePane, CloseTab, CloseTabsToRight, CommitAmend, EditTask, EditWindowTitle,
    FetchChanges, FilesActivate, FilesCollapse, FilesExpand, FilesRefresh, FilesSelectNext,
    FilesSelectPrev, FilesToggleHidden, FocusNextPane, FocusPaneDown, FocusPaneLeft,
    FocusPaneRight, FocusPaneUp, FocusPrevPane, FocusSkillSearch, GitChangesActivate,
    GitChangesSelectNext, GitChangesSelectPrev, GitChangesToggleStage, InstallClaudeHooks,
    InvokeSkillPalette, MinimizeWindow, MoveTabLeft, MoveTabRight, NewSkill, NewTab, NewTask,
    NextTab, OpenAgentChat, OpenCommandHistory, OpenProjectConfig, OpenSettings, PrevTab,
    PullChanges, RefreshGitStatus, ShowLeftDockFiles, ShowLeftDockGit, ShowLeftDockLanes,
    SplitDown, SplitRight, SwitchRightPanelSkills, SwitchRightPanelTasks, SwitchRightPanelTools,
    SwitchRightPanelUsage, ToggleCommandPalette, ToggleFullScreen, ToggleZoomPane,
    UninstallClaudeHooks, ZoomWindow,
};
use crate::workspace::main_area::nav::NavDirection;
use crate::workspace::main_area::pane_tree::SplitDirection;

impl Workspace {
    // ---- Tabs ----

    pub(in crate::workspace) fn on_new_tab(
        &mut self,
        _: &NewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.add_tab(window, cx);
        });
    }

    pub(in crate::workspace) fn on_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            let idx = ws.active_runtime().active_tab_index;
            ws.request_close_tab(idx, window, cx);
        });
    }

    pub(in crate::workspace) fn on_next_tab(
        &mut self,
        _: &NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_runtime().tabs.len() > 1 {
            self.activate_tab(
                (self.active_runtime().active_tab_index + 1) % self.active_runtime().tabs.len(),
                window,
                cx,
            );
        }
    }

    pub(in crate::workspace) fn on_prev_tab(
        &mut self,
        _: &PrevTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_runtime().tabs.len() > 1 {
            let idx = if self.active_runtime().active_tab_index == 0 {
                self.active_runtime().tabs.len() - 1
            } else {
                self.active_runtime().active_tab_index - 1
            };
            self.activate_tab(idx, window, cx);
        }
    }

    pub(in crate::workspace) fn on_activate_tab_n(
        &mut self,
        n: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = if n == 8 {
            self.active_runtime().tabs.len().saturating_sub(1)
        } else {
            n
        };
        if index < self.active_runtime().tabs.len() {
            self.activate_tab(index, window, cx);
        }
    }

    // ---- Splits + close pane ----

    pub(in crate::workspace) fn on_split_right(
        &mut self,
        _: &SplitRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.split_focused_pane(SplitDirection::Horizontal, window, cx);
        });
    }

    pub(in crate::workspace) fn on_split_down(
        &mut self,
        _: &SplitDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.split_focused_pane(SplitDirection::Vertical, window, cx);
        });
    }

    pub(in crate::workspace) fn on_close_pane(
        &mut self,
        _: &ClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.close_focused_pane(window, cx);
        });
    }

    // ---- Left-dock view switches ----

    pub(in crate::workspace) fn on_show_left_dock_worktrees(
        &mut self,
        _: &ShowLeftDockLanes,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_left_dock_view(daruda_store::project::LeftDockView::Lanes, cx);
    }

    pub(in crate::workspace) fn on_show_left_dock_git(
        &mut self,
        _: &ShowLeftDockGit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_left_dock_view(daruda_store::project::LeftDockView::GitChanges, cx);
    }

    pub(in crate::workspace) fn on_show_left_dock_files(
        &mut self,
        _: &ShowLeftDockFiles,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_left_dock_view(daruda_store::project::LeftDockView::Files, cx);
    }

    // ---- Right panel view switches ----

    pub(in crate::workspace) fn on_switch_right_panel_usage(
        &mut self,
        _: &SwitchRightPanelUsage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Usage, cx);
    }

    pub(in crate::workspace) fn on_switch_right_panel_skills(
        &mut self,
        _: &SwitchRightPanelSkills,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Skills, cx);
    }

    pub(in crate::workspace) fn on_switch_right_panel_tools(
        &mut self,
        _: &SwitchRightPanelTools,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Tools, cx);
    }

    pub(in crate::workspace) fn on_switch_right_panel_tasks(
        &mut self,
        _: &SwitchRightPanelTasks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Tasks, cx);
    }

    pub(in crate::workspace) fn on_new_skill(
        &mut self,
        _: &NewSkill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Skills, cx);
        crate::workspace::right_dock::skills::open_create_skill_modal(self, None, window, cx);
    }

    /// Bound to the [`NewTask`] action — opens a fresh TaskEdit pane
    /// in draft mode. Keyboard-equivalent of the right-panel
    /// `[+ New]` button.
    pub(in crate::workspace) fn on_new_task(
        &mut self,
        _: &NewTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_task_edit_pane(None, window, cx);
    }

    /// Bound to the [`OpenAgentChat`] action — opens a fresh Agent chat
    /// (ACP) pane in a new tab, anchored at the active lane.
    pub(in crate::workspace) fn on_open_agent_chat(
        &mut self,
        _: &OpenAgentChat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            ws.open_agent_chat_pane(window, cx);
        });
    }

    /// Bound to the [`EditTask`] action — opens the task picker so the
    /// user can pick which existing task to edit. The picker's confirm
    /// handler routes through `TaskPickAction::Edit` →
    /// `open_task_edit_pane(Some(id), ...)`.
    pub(in crate::workspace) fn on_edit_task(
        &mut self,
        _: &EditTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_task_picker_modal(
            crate::workspace::right_dock::task_picker_modal::TaskPickAction::Edit,
            window,
            cx,
        );
    }

    /// Switch to the Skills tab and focus the search input. Bound to
    /// `Cmd+/` so any keyboard-driven flow lands directly on the
    /// query box without traversing the right-panel tab bar first.
    pub(in crate::workspace) fn on_focus_skill_search(
        &mut self,
        _: &FocusSkillSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_right_dock_view(daruda_store::project::RightDockView::Skills, cx);
        let handle = self.skill_search_input.read(cx).focus_handle(cx);
        handle.focus(window, cx);
    }

    /// Open the global skill picker — every project / personal /
    /// plugin skill, searchable. Bound to `Cmd+Shift+S`. The picker
    /// confirms into the standard invocation modal.
    pub(in crate::workspace) fn on_invoke_skill_palette(
        &mut self,
        _: &InvokeSkillPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Collect every skill across the three scopes from the live
        // `SkillsState` Global. Plugin scope is filtered to Installed
        // (Available rows show in Settings → Plugin only) so the
        // palette never prompts the user to invoke something Claude
        // Code can't actually run.
        let lane = self.active_lane_root();
        let snap = cx
            .global::<crate::agent::skills::SkillsState>()
            .snapshot_for(lane.as_deref());
        let mut skills: Vec<crate::agent::skills::Skill> = snap.project.clone();
        skills.extend(snap.personal.iter().cloned());
        skills.extend(
            snap.plugin
                .iter()
                .filter(|s| {
                    matches!(
                        s.plugin_availability,
                        Some(crate::agent::skills::plugins::PluginAvailability::Installed)
                    )
                })
                .cloned(),
        );
        if skills.is_empty() {
            return;
        }
        self.open_skill_picker_modal(
            &skills,
            gpui::SharedString::from("Invoke skill"),
            window,
            cx,
        );
    }

    // ---- Git ops shims (delegate to `git_ops/` workers) ----

    pub(in crate::workspace) fn on_refresh_git_status_action(
        &mut self,
        _: &RefreshGitStatus,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.active;
        self.refresh_git_status(target, cx);
    }

    pub(in crate::workspace) fn on_commit_amend_action(
        &mut self,
        _: &CommitAmend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_commit_amend(window, cx);
    }

    pub(in crate::workspace) fn on_fetch_action(
        &mut self,
        _: &FetchChanges,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_fetch(cx);
    }

    pub(in crate::workspace) fn on_pull_action(
        &mut self,
        _: &PullChanges,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.on_pull(cx);
    }

    // ---- Files panel ----

    pub(in crate::workspace) fn on_files_toggle_hidden(
        &mut self,
        _: &FilesToggleHidden,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_files_show_hidden(cx);
    }

    pub(in crate::workspace) fn on_files_select_next(
        &mut self,
        _: &FilesSelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_files_selection(1, cx);
    }

    pub(in crate::workspace) fn on_files_select_prev(
        &mut self,
        _: &FilesSelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_files_selection(-1, cx);
    }

    pub(in crate::workspace) fn on_files_activate(
        &mut self,
        _: &FilesActivate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_files_selection(window, cx);
    }

    pub(in crate::workspace) fn on_files_expand(
        &mut self,
        _: &FilesExpand,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.expand_at_files_selection(cx);
    }

    pub(in crate::workspace) fn on_files_collapse(
        &mut self,
        _: &FilesCollapse,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.collapse_at_files_selection(cx);
    }

    pub(in crate::workspace) fn on_files_refresh(
        &mut self,
        _: &FilesRefresh,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_files_root(cx);
    }

    // ---- Settings ----

    pub(in crate::workspace) fn on_open_settings(
        &mut self,
        action: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::windows::open_settings_window(action.0, window, cx);
    }

    // ---- Command history picker (Cmd+Shift+H) ----

    pub(in crate::workspace) fn on_open_command_history(
        &mut self,
        _: &OpenCommandHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::command::history::{CommandHistoryItem, CommandHistoryModal};

        // Snapshot the focused pane's command history at open time;
        // a user filtering the picker is acting on what they were
        // shown, not on a moving target.
        let Some(view) = self.focused_terminal_view() else {
            return;
        };
        let entries = view.read(cx).session().command_history();
        if entries.is_empty() {
            return;
        }
        // Reverse so the most-recent command is at the top of the
        // list — matches `Cmd+R` shell history convention.
        let items: Vec<CommandHistoryItem> = entries
            .iter()
            .rev()
            .map(CommandHistoryItem::from_entry)
            .collect();
        let view_weak = view.downgrade();
        super::dialog_helpers::open_form_modal(
            "",
            None,
            move |window, cx| CommandHistoryModal::new(view_weak.clone(), items, window, cx),
            window,
            cx,
        );
    }

    // ---- Window menu (Minimize / Zoom / Toggle Full Screen) ----

    pub(in crate::workspace) fn on_minimize_window(
        &mut self,
        _: &MinimizeWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.minimize_window();
    }

    pub(in crate::workspace) fn on_zoom_window(
        &mut self,
        _: &ZoomWindow,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.zoom_window();
    }

    pub(in crate::workspace) fn on_toggle_full_screen(
        &mut self,
        _: &ToggleFullScreen,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    /// Open Window > Edit Window Title… modal. Routes the result to
    /// `Workspace::set_window_label`. The window has no index — there
    /// is exactly one per Workspace — so the modal needs no slot id.
    pub(in crate::workspace) fn on_edit_window_title(
        &mut self,
        _: &EditWindowTitle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::surface::strings as s;

        let initial = self.window_user_label.as_ref().map(|s| s.to_string());
        let workspace_handle = cx.entity().downgrade();
        super::dialog_helpers::open_single_field_dialog(
            workspace_handle,
            s::edit_window_title_modal_title(),
            s::edit_window_title_placeholder(),
            initial.as_deref(),
            |ws, value, _window, cx| {
                ws.set_window_label(value, cx);
            },
            window,
            cx,
        );
    }

    /// Open the project-local `config.toml` in the system default
    /// editor, creating the directory and a commented template if the
    /// file doesn't yet exist. The recursive config watcher picks up
    /// the file as soon as it lands so the next save triggers a
    /// reload — no app restart needed.
    pub(in crate::workspace) fn on_open_project_config(
        &mut self,
        _: &OpenProjectConfig,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::surface::strings as s;

        let Some(project_root) = self.active_project().map(|p| p.root.clone()) else {
            let report = ErrorReport::new(s::project_config_no_project())
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .dedup("config.no_project")
                .build();
            self.report_error(report, cx);
            return;
        };
        let Some(path) = daruda_config::project_config_path(&project_root) else {
            let report = ErrorReport::new(s::project_config_no_dir())
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .dedup("config.no_dir")
                .build();
            self.report_error(report, cx);
            return;
        };

        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            let report = ErrorReport::new("Failed to create project config directory")
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(parent))
                .dedup("config.mkdir")
                .build();
            self.report_error(report, cx);
            return;
        }
        if !path.exists()
            && let Err(e) = std::fs::write(&path, s::PROJECT_CONFIG_TEMPLATE)
        {
            let report = ErrorReport::new("Failed to create project config file")
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("config.create")
                .build();
            self.report_error(report, cx);
            return;
        }

        // macOS-only — daruda is macOS-only per project README. The
        // `open` crate launches the system default handler and detaches
        // without waiting for it to exit.
        if let Err(e) = open::that_detached(&path) {
            let report = ErrorReport::new("Failed to open project config")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .dedup("config.open")
                .build();
            self.report_error(report, cx);
        }
    }

    // ---- Claude Code integration ----

    pub(in crate::workspace) fn on_install_claude_hooks(
        &mut self,
        _: &InstallClaudeHooks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match crate::hooks::installer::InstallerPaths::from_env() {
            Ok(paths) => crate::hooks::installer::install(&paths),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                self.claude.claude_hooks_installed = true;
                // Restart the watcher so it picks up any lane
                // path changes that may have occurred during install.
                self.refresh_jsonl_watcher(cx);
            }
            Err(e) => {
                let report = ErrorReport::new("Claude hooks install failed")
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("hooks.install")
                    .build();
                self.report_error(report, cx);
            }
        }
        cx.notify();
    }

    pub(in crate::workspace) fn on_uninstall_claude_hooks(
        &mut self,
        _: &UninstallClaudeHooks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match crate::hooks::installer::InstallerPaths::from_env() {
            Ok(paths) => crate::hooks::installer::uninstall(&paths),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                self.claude.claude_hooks_installed = false;
                // Restart the watcher so it picks up any lane
                // path changes that may have occurred during uninstall.
                self.refresh_jsonl_watcher(cx);
            }
            Err(e) => {
                let report = ErrorReport::new("Claude hooks uninstall failed")
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("hooks.uninstall")
                    .build();
                self.report_error(report, cx);
            }
        }
        cx.notify();
    }

    // ---- Tab context-menu operations ----
    // These actions have no keyboard binding; they are dispatched exclusively
    // from the tab bar right-click context menu. The handlers operate on the
    // active tab, not the right-clicked tab — context-menu callers invoke
    // close_other_tabs / close_tabs_to_right / toggle_zoom_pane directly
    // with the clicked index / pane id.

    pub(in crate::workspace) fn on_close_other_tabs(
        &mut self,
        _: &CloseOtherTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            let idx = ws.active_runtime().active_tab_index;
            let indices: Vec<usize> = (0..ws.active_runtime().tabs.len())
                .rev()
                .filter(|&i| i != idx)
                .collect();
            ws.request_close_tabs_bulk(indices, window, cx);
        });
    }

    pub(in crate::workspace) fn on_close_tabs_to_right(
        &mut self,
        _: &CloseTabsToRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mutate_durable_in(window, cx, |ws, window, cx| {
            let idx = ws.active_runtime().active_tab_index;
            let indices: Vec<usize> = (idx + 1..ws.active_runtime().tabs.len()).rev().collect();
            ws.request_close_tabs_bulk(indices, window, cx);
        });
    }

    pub(in crate::workspace) fn on_toggle_zoom_pane(
        &mut self,
        _: &ToggleZoomPane,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.active_runtime().focused_pane_id;
        self.toggle_zoom_pane(pane_id, cx);
    }

    // ---- Command palette ----

    pub(in crate::workspace) fn on_toggle_command_palette(
        &mut self,
        _: &ToggleCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command_palette.is_open {
            self.command_palette.close();
        } else {
            self.command_palette.open();
        }
        cx.notify();
    }

    // ---- Pane focus ----

    pub(in crate::workspace) fn on_focus_next_pane(
        &mut self,
        _: &FocusNextPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_next_pane(window, cx);
    }

    pub(in crate::workspace) fn on_focus_prev_pane(
        &mut self,
        _: &FocusPrevPane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_prev_pane(window, cx);
    }

    pub(in crate::workspace) fn on_focus_pane_left(
        &mut self,
        _: &FocusPaneLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane_in_direction(NavDirection::Left, window, cx);
    }

    pub(in crate::workspace) fn on_focus_pane_right(
        &mut self,
        _: &FocusPaneRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane_in_direction(NavDirection::Right, window, cx);
    }

    pub(in crate::workspace) fn on_focus_pane_up(
        &mut self,
        _: &FocusPaneUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane_in_direction(NavDirection::Up, window, cx);
    }

    pub(in crate::workspace) fn on_focus_pane_down(
        &mut self,
        _: &FocusPaneDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane_in_direction(NavDirection::Down, window, cx);
    }

    // ---- Tab move ----

    pub(in crate::workspace) fn on_move_tab_left(
        &mut self,
        _: &MoveTabLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.active_runtime().active_tab_index;
        if from > 0 {
            self.mutate_durable(cx, |ws, cx| {
                ws.move_tab(from, from - 1, cx);
            });
        }
    }

    pub(in crate::workspace) fn on_move_tab_right(
        &mut self,
        _: &MoveTabRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.active_runtime().active_tab_index;
        if from + 1 < self.active_runtime().tabs.len() {
            self.mutate_durable(cx, |ws, cx| {
                ws.move_tab(from, from + 1, cx);
            });
        }
    }

    // ---- Git Changes keyboard navigation ----

    pub(in crate::workspace) fn on_git_changes_select_next(
        &mut self,
        _: &GitChangesSelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_git_changes_cursor(1, cx);
    }

    pub(in crate::workspace) fn on_git_changes_select_prev(
        &mut self,
        _: &GitChangesSelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_git_changes_cursor(-1, cx);
    }

    pub(in crate::workspace) fn on_git_changes_toggle_stage(
        &mut self,
        _: &GitChangesToggleStage,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_git_changes_cursor_stage(cx);
    }

    pub(in crate::workspace) fn on_git_changes_activate(
        &mut self,
        _: &GitChangesActivate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_git_changes_cursor(window, cx);
    }
}
