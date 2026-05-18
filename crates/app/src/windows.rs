//! Window lifecycle — open / close / replace workspace and welcome windows.
//!
//! Holds the re-entrancy guard around project-opening flows so the
//! folder picker (async) cannot race with itself or sweep a window
//! that another in-flight open just spawned.

use std::sync::atomic::{AtomicBool, Ordering};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use gpui::{
    App, Bounds, Point, Size, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, prelude::*, px,
};

use crate::settings_window::SettingsWindow;
use crate::welcome;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

fn log_touch_recent_err(path: &std::path::Path, e: std::io::Error) {
    LogWriter::log(
        ErrorReport::new("Failed to update recent projects list")
            .severity(ErrorSeverity::Info)
            .from_error(&e)
            .at(file!(), line!())
            .with_context("path", redact_home(path))
            .dedup("recent.touch")
            .build(),
    );
}

/// Where to land a project opened via File menu / recent list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// Close the currently-open workspace window(s) after the new
    /// one finishes opening. Default for `Open…` and `Open Recent`.
    ReplaceCurrent,
    /// Leave existing windows alone; just add another. Selected via
    /// the `… in New Window` menu siblings.
    NewWindow,
}

/// Re-entrancy guard for project-opening flows. Prevents rapid
/// double-triggers of Open… / Open Recent from overlapping — the
/// folder picker is async, so without this a second call could sweep
/// the window the first call just opened (ReplaceCurrent snapshots
/// ALL prior workspace windows, including any that another in-flight
/// open just spawned). Cleared after the workspace window is
/// installed on the main thread.
static OPEN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub(crate) fn try_enter_open() -> bool {
    OPEN_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(crate) fn leave_open() {
    OPEN_IN_PROGRESS.store(false, Ordering::Release);
}

/// Shared titlebar options for every daruda window: transparent titlebar so
/// GPUI renders the full-bleed header bar, traffic lights at the standard
/// position defined in the theme.
fn build_titlebar_options() -> TitlebarOptions {
    TitlebarOptions {
        title: None,
        appears_transparent: true,
        traffic_light_position: Some(point(
            px(crate::ui::theme::TRAFFIC_LIGHT_X),
            px(crate::ui::theme::TRAFFIC_LIGHT_Y),
        )),
    }
}

pub(crate) fn build_window_options(config: &daruda_config::Config) -> WindowOptions {
    let mut opts = WindowOptions {
        titlebar: Some(build_titlebar_options()),
        ..Default::default()
    };
    if config.window.opacity < 1.0 || config.window.blur {
        opts.window_background = if config.window.blur {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Transparent
        };
    }
    opts
}

pub(crate) fn open_workspace_window(
    config: std::sync::Arc<daruda_config::Config>,
    project: Option<daruda_store::project::Project>,
    saved_state: Option<daruda_store::project::ProjectState>,
    mut window_opts: WindowOptions,
    cx: &mut App,
) {
    // Apply saved window geometry before opening so the new window
    // spawns at its previous position/size instead of the default.
    if let Some(state) = saved_state.as_ref()
        && state.window.is_valid()
    {
        window_opts.window_bounds = Some(WindowBounds::Windowed(Bounds::new(
            Point::new(px(state.window.x), px(state.window.y)),
            Size::new(px(state.window.width), px(state.window.height)),
        )));
    }

    cx.open_window(window_opts, |window, cx| {
        let workspace: gpui::Entity<Workspace> = cx.new(|cx| {
            let data_dir = daruda_store::persistence::default_data_dir();
            let mut ws = Workspace::new_with_project(&config, project, data_dir, window, cx);
            if let Some(state) = saved_state {
                ws.restore_state(&state, window, cx);
            }
            ws
        });
        cx.new(|cx| gpui_component::Root::new(workspace, window, cx))
    })
    .unwrap();
}

/// Open the welcome screen and wire its buttons/recent-list clicks to
/// the workspace launch path. Shared by startup (when there is no
/// recent project to restore) and the `CloseProject` action.
pub(crate) fn open_welcome_window(
    config: std::sync::Arc<daruda_config::Config>,
    opts: WindowOptions,
    cx: &mut App,
) {
    let recent = daruda_store::project::persistence::load_recent();
    let cfg_for_welcome = config.clone();
    let welcome_entity_holder: std::sync::Arc<
        std::sync::Mutex<Option<gpui::Entity<welcome::WelcomeScreen>>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(None));
    let holder_for_window = welcome_entity_holder.clone();

    let welcome_window = cx
        .open_window(opts, |_window, cx| {
            let entity = cx.new(|cx| welcome::WelcomeScreen::new(recent, cx));
            *holder_for_window.lock().unwrap() = Some(entity.clone());
            entity
        })
        .unwrap();

    let Some(welcome_entity) = welcome_entity_holder.lock().unwrap().clone() else {
        return;
    };
    let ww_handle = welcome_window;
    cx.subscribe(&welcome_entity, move |_welcome, event, cx| {
        let cfg = cfg_for_welcome.clone();
        // Close welcome after opening a successor window.
        let close_welcome = move |cx: &mut App| {
            let _ = cx.update_window(ww_handle.into(), |_, window, _cx| {
                window.remove_window();
            });
        };
        match event {
            welcome::WelcomeEvent::OpenFolder => {
                // Folder picker is async; closing welcome before
                // the user picks would quit the app on
                // last-window-closed.
                let cfg2 = cfg.clone();
                let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                    prompt: None,
                });
                cx.spawn(async move |cx| {
                    if let Ok(Ok(Some(selected))) = paths.await
                        && let Some(path) = selected.first()
                    {
                        let project = daruda_store::project::Project::from_path(path);
                        if let Err(e) = daruda_store::project::persistence::touch_recent(path) {
                            log_touch_recent_err(path, e);
                        }
                        let saved = daruda_store::project::persistence::load_state(path);
                        let _ = cx.update(|cx| {
                            let opts = build_window_options(&cfg2);
                            open_workspace_window(cfg2.clone(), Some(project), saved, opts, cx);
                            close_welcome(cx);
                            crate::menus::refresh_recent_menu(cx);
                        });
                    }
                })
                .detach();
            }
            welcome::WelcomeEvent::OpenProject(path) => {
                let project = daruda_store::project::Project::from_path(path);
                if let Err(e) = daruda_store::project::persistence::touch_recent(path) {
                    log_touch_recent_err(path, e);
                }
                let saved = daruda_store::project::persistence::load_state(path);
                let opts = build_window_options(&cfg);
                open_workspace_window(cfg, Some(project), saved, opts, cx);
                close_welcome(cx);
                crate::menus::refresh_recent_menu(cx);
            }
            welcome::WelcomeEvent::NewEmpty => {
                let opts = build_window_options(&cfg);
                open_workspace_window(cfg, None, None, opts, cx);
                close_welcome(cx);
            }
        }
    })
    .detach();
}

/// Open the recent project at `idx`. Missing index / stale path is a
/// silent no-op (matches macOS conventions for stale Open Recent).
/// `mode` controls whether the active workspace window is closed
/// after the new one opens.
pub(crate) fn open_recent_idx(
    idx: usize,
    recent: std::sync::Arc<Vec<daruda_store::project::RecentEntry>>,
    config: std::sync::Arc<daruda_config::Config>,
    mode: OpenMode,
    cx: &mut App,
) {
    if !try_enter_open() {
        return;
    }
    let Some(entry) = recent.get(idx) else {
        leave_open();
        return;
    };
    let initiating_window = active_window_to_close(cx);
    let project = daruda_store::project::Project::from_path(&entry.root);
    if let Err(e) = daruda_store::project::persistence::touch_recent(&entry.root) {
        log_touch_recent_err(&entry.root, e);
    }
    let saved = daruda_store::project::persistence::load_state(&entry.root);
    let opts = build_window_options(&config);
    open_project_with_mode(
        config.clone(),
        Some(project),
        saved,
        opts,
        mode,
        initiating_window,
        cx,
    );
    crate::menus::refresh_recent_menu(cx);
    leave_open();
}

/// Folder-picker entry point shared by `OpenFolder` and
/// `OpenFolderInNewWindow`. Opens the native picker asynchronously;
/// if the user picks a directory, loads any saved session and hands
/// off to `open_project_with_mode`. Re-entrancy is guarded by
/// `OPEN_IN_PROGRESS` so a rapid second trigger cannot sweep the
/// window the first one just created.
pub(crate) fn prompt_and_open_folder(
    config: std::sync::Arc<daruda_config::Config>,
    mode: OpenMode,
    cx: &mut App,
) {
    if !try_enter_open() {
        return;
    }
    // Capture the initiating window before the async picker so that
    // ReplaceCurrent closes only this window, not every open workspace.
    let initiating_window = active_window_to_close(cx);
    let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let selected = paths.await;
        let _ = cx.update(|cx| {
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.first()
            {
                let project = daruda_store::project::Project::from_path(path);
                if let Err(e) = daruda_store::project::persistence::touch_recent(path) {
                    log_touch_recent_err(path, e);
                }
                let saved = daruda_store::project::persistence::load_state(path);
                let opts = build_window_options(&config);
                open_project_with_mode(
                    config.clone(),
                    Some(project),
                    saved,
                    opts,
                    mode,
                    initiating_window,
                    cx,
                );
                crate::menus::refresh_recent_menu(cx);
            }
            leave_open();
        });
    })
    .detach();
}

/// Return the handle of the window that should be closed when a
/// `ReplaceCurrent` open fires. Checks the `WindowRegistry` first
/// (covers Workspace windows), then falls back to checking whether the
/// active window is a `WelcomeScreen` (which is not tracked by the
/// registry).
fn active_window_to_close(cx: &App) -> Option<gpui::AnyWindowHandle> {
    WindowRegistry::active_workspace_handle(cx).or_else(|| {
        cx.active_window()
            .filter(|h| h.downcast::<welcome::WelcomeScreen>().is_some())
    })
}

/// Open a workspace window and, if `mode == ReplaceCurrent`, close
/// `window_to_close` on the next tick. Passing `None` skips the
/// close step (used when there is no initiating window to replace).
/// Welcome is included as a valid target because the menu-bar
/// `Open…` path does not route through Welcome's own event handler.
pub(crate) fn open_project_with_mode(
    config: std::sync::Arc<daruda_config::Config>,
    project: Option<daruda_store::project::Project>,
    saved_state: Option<daruda_store::project::ProjectState>,
    opts: WindowOptions,
    mode: OpenMode,
    window_to_close: Option<gpui::AnyWindowHandle>,
    cx: &mut App,
) {
    // Identify the window to replace before opening the new one so
    // the handle definitely refers to the initiating window, not the
    // newcomer. Only act in ReplaceCurrent mode.
    let target = if mode == OpenMode::ReplaceCurrent {
        window_to_close
    } else {
        None
    };
    open_workspace_window(config, project, saved_state, opts, cx);
    let Some(handle) = target else {
        return;
    };
    // Sequencing assumption: `open_workspace_window` runs synchronously
    // and has already queued the new window before we spawn here, so
    // `remove_window` on the next tick targets only the captured handle,
    // never the newcomer.
    cx.spawn(async move |cx| {
        let _ = cx.update(|cx| {
            let _ = cx.update_window(handle, |_, window, _| {
                window.remove_window();
            });
        });
    })
    .detach();
}

/// Open the Settings window on `section`. If a Settings window is
/// already open, bring it to the front and route through
/// `SettingsWindow::focus_section` to switch the active page instead
/// of opening a second one.
pub(crate) fn open_settings_window(
    section: daruda_config::BuiltinSection,
    _window: &mut gpui::Window,
    cx: &mut App,
) {
    if let Some(sh) = WindowRegistry::settings(cx) {
        sh.update(cx, move |this, window, cx| {
            this.focus_section(section, window, cx);
            window.activate_window();
        });
        return;
    }

    let opts = WindowOptions {
        titlebar: Some(build_titlebar_options()),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Point::new(
                px(crate::ui::theme::SETTINGS_WINDOW_ORIGIN_X),
                px(crate::ui::theme::SETTINGS_WINDOW_ORIGIN_Y),
            ),
            Size::new(
                px(crate::ui::theme::SETTINGS_WINDOW_W),
                px(crate::ui::theme::SETTINGS_WINDOW_H),
            ),
        ))),
        ..Default::default()
    };

    // The Settings window root is `gpui_component::Root` because the form
    // renders `gpui_component::Input` text fields, whose `TextElement::paint`
    // calls `Root::read` and panics if the root view is not a `Root`. The
    // inner `SettingsWindow` entity registers itself in `WindowRegistry` via
    // its constructor so the singleton-focus path above can reach it.
    cx.open_window(opts, |window, cx| {
        let settings = cx.new(|cx| SettingsWindow::new_with_section(section, window, cx));
        cx.new(|cx| gpui_component::Root::new(settings, window, cx))
    })
    .unwrap();
}

/// Close every currently-open Workspace window. Runs on the next
/// tick so callers can trigger this from a menu dispatch without
/// re-entering the current update cycle.
///
/// `drain_handles` atomically clears the registry before the async
/// close fires, so a double-trigger (e.g. two rapid CloseProject
/// dispatches) returns an empty list on the second call and exits
/// immediately without sweeping windows the first call already queued.
pub(crate) fn close_all_workspace_windows(cx: &mut App) {
    let targets = WindowRegistry::drain_handles(cx);
    if targets.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        let _ = cx.update(|cx| {
            for handle in targets {
                let _ = cx.update_window(handle, |_, window, _| {
                    window.remove_window();
                });
            }
        });
    })
    .detach();
}
