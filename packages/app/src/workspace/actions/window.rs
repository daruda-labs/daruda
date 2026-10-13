//! Settings, command-history, window-menu, project-config and agent-hook
//! action shims.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Window};

use crate::workspace::Workspace;
use crate::workspace::{
    EditWindowTitle, InstallAgentHooks, MinimizeWindow, OpenCommandHistory, OpenProjectConfig,
    OpenSettings, ToggleFullScreen, UninstallAgentHooks, ZoomWindow,
};

impl Workspace {
    // ---- Settings ----

    pub(crate) fn on_open_settings(
        &mut self,
        action: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Refresh how each account signed in before the rows that show it are
        // on screen. Opening Settings is the one moment the user is about to
        // look, and the reading costs a process spawn — so it is taken here
        // rather than polled.
        self.probe_auth_statuses(cx);
        self.open_settings(action.0, window, cx);
    }

    // ---- Command history picker (Cmd+Shift+H) ----

    pub(in crate::workspace) fn on_open_command_history(
        &mut self,
        _: &OpenCommandHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::workspace::command::history::{CommandHistoryItem, CommandHistoryModal};

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
        crate::workspace::dialog_helpers::open_form_modal(
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

        let initial = self
            .window_runtime
            .user_label
            .as_ref()
            .map(|s| s.to_string());
        let workspace_handle = cx.entity().downgrade();
        crate::workspace::dialog_helpers::open_single_field_dialog(
            workspace_handle,
            s::modal::edit_window_title_title(),
            s::modal::edit_window_title_placeholder(),
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
            let report = ErrorReport::new(s::settings::project_config_no_project())
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .dedup("config.no_project")
                .build();
            self.report_error(report, cx);
            return;
        };
        let Some(path) = daruda_config::project_config_path(&project_root) else {
            let report = ErrorReport::new(s::settings::project_config_no_dir())
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
            let report = ErrorReport::new(
                crate::surface::strings::error::project_config_create_dir_failed(),
            )
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
            && let Err(e) = std::fs::write(&path, daruda_config::project::PROJECT_CONFIG_TEMPLATE)
        {
            let report = ErrorReport::new(
                crate::surface::strings::error::project_config_create_file_failed(),
            )
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
            let report =
                ErrorReport::new(crate::surface::strings::error::project_config_open_failed())
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

    pub(in crate::workspace) fn on_install_agent_hooks(
        &mut self,
        _: &InstallAgentHooks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match daruda_agent::hooks::installer::InstallerPaths::from_env() {
            Ok(paths) => daruda_agent::hooks::installer::install(&paths),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                self.claude_sessions.hooks_installed = true;
                // Restart the watcher so it picks up any lane
                // path changes that may have occurred during install.
                self.refresh_jsonl_watcher(cx);
            }
            Err(e) => {
                let report =
                    ErrorReport::new(crate::surface::strings::error::claude_hooks_install_failed())
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

    pub(in crate::workspace) fn on_uninstall_agent_hooks(
        &mut self,
        _: &UninstallAgentHooks,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match daruda_agent::hooks::installer::InstallerPaths::from_env() {
            Ok(paths) => daruda_agent::hooks::installer::uninstall(&paths),
            Err(e) => Err(e),
        };
        match result {
            Ok(()) => {
                self.claude_sessions.hooks_installed = false;
                // Restart the watcher so it picks up any lane
                // path changes that may have occurred during uninstall.
                self.refresh_jsonl_watcher(cx);
            }
            Err(e) => {
                let report = ErrorReport::new(
                    crate::surface::strings::error::claude_hooks_uninstall_failed(),
                )
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
}
