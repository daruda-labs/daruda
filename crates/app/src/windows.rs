//! Window lifecycle — open / close / replace workspace and settings windows.
//!
//! Holds the re-entrancy guard around project-opening flows so the
//! folder picker (async) cannot race with itself or sweep a window
//! that another in-flight open just spawned.

use std::sync::atomic::{AtomicBool, Ordering};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::project::{ProjectState, WorkspaceState, WorkspaceUuid};
use gpui::{
    App, Bounds, Point, Size, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, prelude::*, px,
};

use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;

/// Enter `handle`'s window context to run `f`. Failures route through
/// the on-disk log (Layer 3) with `site` as the dedup tag, so the
/// May-2026 silent-failure class — `let _ = cx.update_window(...)`
/// swallowing a "window not found" inside a modal callback — can no
/// longer hide. Every new caller that needs to re-enter a window from
/// outside its event loop must go through this helper or a
/// `match`/`?` of its own; `scripts/lint-no-silent-update.sh` enforces.
pub(crate) fn try_update_workspace_window<F>(
    handle: gpui::AnyWindowHandle,
    cx: &mut App,
    site: &'static str,
    f: F,
) where
    F: FnOnce(&mut gpui::Window, &mut App),
{
    match cx.update_window(handle, move |_, window, cx_w| f(window, cx_w)) {
        Ok(()) => {}
        Err(e) => {
            LogWriter::log(
                ErrorReport::new("Failed to enter window context")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("site", site)
                    .with_context("error", format!("{e}"))
                    .dedup(format!("window.update_failed.{site}"))
                    .build(),
            );
        }
    }
}

/// Where to land a project opened via File menu / recent list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// Close the currently-open workspace window(s) after the new
    /// one finishes opening. Used by the recent-list menu items
    /// (File > Open Recent) which conceptually replace the active
    /// project. The first-class `Open…` action now routes through
    /// [`prompt_and_open_folder_with_policy`] instead.
    ReplaceCurrent,
    /// Leave existing windows alone; just add another. Selected via
    /// the `… in New Window` menu siblings and the
    /// `OpenFolderInNewWindow` action.
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

/// Open a workspace window for the new UUID-keyed schema.
///
/// - `saved = Some((workspace, projects))` — restore an existing
///   workspace (recent list, startup, OpenRecent menu); `project` is
///   ignored.
/// - `saved = None, project = Some(_)` — fresh workspace seeded with
///   one project (folder picker, "New Window" path).
/// - `saved = None, project = None` — fully empty workspace
///   (`NewEmptyWindow`).
pub(crate) fn open_workspace_window(
    config: std::sync::Arc<daruda_config::Config>,
    saved: Option<(WorkspaceState, Vec<ProjectState>)>,
    project: Option<daruda_store::project::Project>,
    window_opts: WindowOptions,
    cx: &mut App,
) {
    try_open_workspace_window(config, saved, project, window_opts, cx).unwrap();
}

/// Fallible counterpart for callers that must report an unavailable host.
pub(crate) fn try_open_workspace_window(
    config: std::sync::Arc<daruda_config::Config>,
    saved: Option<(WorkspaceState, Vec<ProjectState>)>,
    project: Option<daruda_store::project::Project>,
    mut window_opts: WindowOptions,
    cx: &mut App,
) -> anyhow::Result<gpui::AnyWindowHandle> {
    // Apply saved window geometry before opening so the new window
    // spawns at its previous position/size instead of the default.
    if let Some((ws, _)) = saved.as_ref()
        && ws.window.is_valid()
    {
        window_opts.window_bounds = Some(WindowBounds::Windowed(Bounds::new(
            Point::new(px(ws.window.x), px(ws.window.y)),
            Size::new(px(ws.window.width), px(ws.window.height)),
        )));
    }

    cx.open_window(window_opts, |window, cx| {
        let workspace: gpui::Entity<Workspace> = cx.new(|cx| {
            #[cfg(not(test))]
            let data_dir = daruda_store::persistence::default_data_dir();
            #[cfg(test)]
            let data_dir =
                std::env::temp_dir().join(format!("daruda-host-{}", uuid::Uuid::new_v4()));
            // Mirror the legacy split: when restoring, build the
            // workspace without an initial project (restore will
            // populate `self.projects`); otherwise seed with the
            // caller-supplied project (or None for an empty window).
            let seed = if saved.is_some() { None } else { project };
            #[cfg(not(test))]
            let mut ws = Workspace::new_with_project(&config, seed, data_dir, window, cx);
            #[cfg(test)]
            let mut ws = Workspace::new_with_project_for_test(&config, seed, data_dir, window, cx);
            if let Some((ws_state, project_states)) = saved {
                ws.restore_from_disk(&ws_state, &project_states, window, cx);
            }
            ws
        });
        #[cfg(test)]
        WindowRegistry::register(window.window_handle(), workspace.downgrade(), cx);
        cx.new(|cx| gpui_component::Root::new(workspace, window, cx))
    })
    .map(Into::into)
}

/// Open the recent project at `idx`. Missing index / stale workspace
/// is a silent no-op (matches macOS conventions for stale Open
/// Recent). `mode` controls whether the active workspace window is
/// closed after the new one opens.
pub(crate) fn open_recent_idx(
    idx: usize,
    recent: std::sync::Arc<Vec<daruda_store::project::RecentEntry>>,
    config: std::sync::Arc<daruda_config::Config>,
    mode: OpenMode,
    cx: &mut App,
) {
    let Some(entry) = recent.get(idx) else {
        return;
    };
    open_recent_uuid(entry.workspace_uuid, config, mode, cx);
}

/// Open the workspace `uuid` names. The identity-keyed entry point: the
/// Landing view dispatches this so a row's label and the workspace it opens
/// cannot disagree, which indexing a separately-loaded list can.
///
/// Missing / stale workspace is a silent no-op that prunes the recent row
/// (matches macOS conventions for stale Open Recent).
pub(crate) fn open_recent_uuid(
    uuid: WorkspaceUuid,
    config: std::sync::Arc<daruda_config::Config>,
    mode: OpenMode,
    cx: &mut App,
) {
    if !try_enter_open() {
        return;
    }
    // A workspace is one on-disk record, so opening a second window onto it
    // would give two live `Workspace`s one uuid and let the last save win —
    // an emptied one can erase the projects the other still holds. Focus the
    // window that already has it instead. Clicking a Landing row for the
    // window you are in lands here too, and correctly does nothing.
    if let Some(open) = WindowRegistry::workspace_window_for_uuid(uuid, cx) {
        activate_existing(open, cx);
        leave_open();
        return;
    }
    let initiating_window = active_window_to_close(cx);
    let data_dir = daruda_store::persistence::default_data_dir();
    let Some(ws_state) = daruda_store::project::load_workspace_state_in(&data_dir, uuid) else {
        // Stale recent entry — prune and bail. The user perceives
        // this as the menu row vanishing on next refresh.
        let mut entries = daruda_store::project::load_recent_in(&data_dir);
        entries.retain(|e| e.workspace_uuid != uuid);
        if let Err(e) = daruda_store::project::save_recent_in(&data_dir, &entries) {
            LogWriter::log(
                ErrorReport::new("Failed to prune stale recent entry")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("workspace_uuid", uuid.as_inner().to_string())
                    .dedup("recent.prune_missing")
                    .build(),
            );
        }
        crate::menus::refresh_recent_menu(cx);
        leave_open();
        return;
    };
    let project_states: Vec<_> = ws_state
        .project_ids
        .iter()
        .filter_map(|p| daruda_store::project::load_project_state_in(&data_dir, *p))
        .collect();
    let opts = build_window_options(&config);
    open_project_with_mode(
        config.clone(),
        Some((ws_state, project_states)),
        None,
        opts,
        mode,
        initiating_window,
        cx,
    );
    crate::menus::refresh_recent_menu(cx);
    leave_open();
}

/// Folder-picker entry point used by `OpenFolderInNewWindow` and the
/// recent-list menu items — both want a fresh window regardless of
/// any workspace-level policy. Re-entrancy is guarded by
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
        // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
        cx.update(|cx| {
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.first()
            {
                // Policy B: the same root may legitimately appear in
                // multiple windows (each share the on-disk
                // `ProjectState` via UUID reuse). This entry point —
                // `OpenFolderInNewWindow` / recent-list menu — wants a
                // fresh window regardless of any workspace-level
                // policy, so no cross-window dedup branch fires here.
                let project = daruda_store::project::Project::from_path(path);
                let opts = build_window_options(&config);
                open_project_with_mode(
                    config.clone(),
                    None,
                    Some(project),
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

/// Policy-driven entry point for the `cmd-o` Open Project action.
///
/// Prompts the user for a folder, then dispatches based on the active
/// workspace's `WindowOpenPolicy`:
///   - `AddHere` → call `Workspace::add_project` on the active window
///     (same-window dedup is enforced inside the `AddHere` path);
///   - `NewWindow` → open a fresh window;
///   - `Ask` → surface the [`OpenProjectModal`] chooser.
///
/// **No active workspace** — open a fresh window (the `NewWindow`
/// branch).
///
/// Policy B: the same root may legitimately appear in multiple windows
/// (each share the on-disk `ProjectState` via UUID reuse), so no
/// cross-window dedup branch fires here.
pub(crate) fn prompt_and_open_folder_with_policy(
    config: std::sync::Arc<daruda_config::Config>,
    cx: &mut App,
) {
    if !try_enter_open() {
        return;
    }
    let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let selected = paths.await;
        // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
        cx.update(|cx| {
            if let Ok(Ok(Some(paths))) = selected
                && let Some(path) = paths.first()
            {
                handle_picked_folder(config.clone(), path.clone(), cx);
            }
            leave_open();
        });
    })
    .detach();
}

/// Synchronous dispatcher for a picked folder. Splits the policy
/// decision tree from the async picker so the same logic can be
/// re-used by tests / the chooser modal's submit callback.
fn handle_picked_folder(
    config: std::sync::Arc<daruda_config::Config>,
    path: std::path::PathBuf,
    cx: &mut App,
) {
    let Some((handle, weak)) = WindowRegistry::active_workspace(cx) else {
        open_new_workspace_for_path(config, &path, cx);
        return;
    };
    // An empty workspace is the Landing state, where the policy question
    // ("add here or open a new window?") has nothing to weigh — there is
    // no occupant to displace. Answer it here so `Ask` doesn't pop a
    // chooser over a window holding nothing.
    if workspace_is_empty(handle, &weak, cx) {
        add_path_to_workspace(handle, &weak, path, cx);
        return;
    }
    let policy = workspace_policy(handle, &weak, cx);
    match policy {
        daruda_store::project::WindowOpenPolicy::AddHere => {
            // Policy B allows the same root in multiple windows, but
            // adding it twice to the *same* window would render the
            // folder twice. Focus the active window if its workspace
            // already owns this root; otherwise add.
            if workspace_has_root(handle, &weak, &path, cx) {
                activate_existing(handle, cx);
                return;
            }
            add_path_to_workspace(handle, &weak, path, cx);
        }
        daruda_store::project::WindowOpenPolicy::NewWindow => {
            open_new_workspace_for_path(config, &path, cx);
        }
        daruda_store::project::WindowOpenPolicy::Ask => {
            open_chooser_modal(config, handle, weak, path, cx);
        }
    }
}

/// Activate (focus) a previously-registered workspace window. Used by
/// the duplicate-root check so the user sees their existing project
/// instead of getting a second copy in a new window.
fn activate_existing(handle: gpui::AnyWindowHandle, cx: &mut App) {
    // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
    let _ = cx.update_window(handle, |_, window, _| {
        window.activate_window();
    });
}

/// Read the active workspace's [`WindowOpenPolicy`] through its
/// `WeakEntity`. Falls back to [`WindowOpenPolicy::default()`] (`Ask`)
/// when the entity is gone or the read fails.
fn workspace_policy(
    handle: gpui::AnyWindowHandle,
    weak: &gpui::WeakEntity<crate::workspace::Workspace>,
    cx: &mut App,
) -> daruda_store::project::WindowOpenPolicy {
    let weak = weak.clone();
    cx.update_window(handle, |_, _, cx_w| {
        weak.upgrade()
            .map(|ws| ws.read(cx_w).window_open_policy())
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// True when the workspace referenced by `weak` holds no projects — the
/// Landing state. Returns `false` if the entity is gone or the read
/// fails, which routes the caller back through the normal policy path
/// rather than silently adding to a window that may not be there.
fn workspace_is_empty(
    handle: gpui::AnyWindowHandle,
    weak: &gpui::WeakEntity<crate::workspace::Workspace>,
    cx: &mut App,
) -> bool {
    let weak = weak.clone();
    cx.update_window(handle, move |_, _, cx_w| {
        weak.upgrade()
            .map(|ws| ws.read(cx_w).has_no_projects())
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// True when the workspace referenced by `weak` already hosts a project
/// at `root`. Used by the `AddHere` policy path to keep the same-window
/// dedup intact under policy B (the cross-window dedup is intentionally
/// gone). Returns `false` if the entity is gone or the read fails — the
/// add path will run and the worst case is the workspace gracefully
/// rejects the redundant entry on its own.
fn workspace_has_root(
    handle: gpui::AnyWindowHandle,
    weak: &gpui::WeakEntity<crate::workspace::Workspace>,
    root: &std::path::Path,
    cx: &mut App,
) -> bool {
    let weak = weak.clone();
    let root = root.to_path_buf();
    cx.update_window(handle, move |_, _, cx_w| {
        weak.upgrade()
            .map(|ws| ws.read(cx_w).has_project_root(&root))
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// AddHere path — drive [`Workspace::add_project`] inside the active
/// window's render cycle. `Workspace::add_project` triggers
/// `persist_state` via `mutate_durable`, which writes the new project
/// file and updates the recent list in one shot, so no explicit
/// `touch_recent_in` is needed here.
fn add_path_to_workspace(
    handle: gpui::AnyWindowHandle,
    weak: &gpui::WeakEntity<crate::workspace::Workspace>,
    path: std::path::PathBuf,
    cx: &mut App,
) {
    // The handle captured by `handle_picked_folder` can go stale by the
    // time the chooser modal's submit callback fires — GPUI considers
    // the window "not found" inside `update_window` even though the
    // workspace itself is still alive (the modal entity's lifecycle
    // appears to invalidate the cached handle as part of close_dialog).
    // Probe first, then fall back to whichever workspace window the
    // registry currently considers active. The weak entity stays the
    // authoritative pointer; only the handle is re-resolved.
    let target_handle = match cx.update_window(handle, |_, _, _| {}) {
        Ok(()) => handle,
        Err(_) => match WindowRegistry::active_workspace(cx) {
            Some((fresh, _)) => {
                LogWriter::log(
                    ErrorReport::new(
                        "trace.add_project_flow: handle re-resolved via active_workspace",
                    )
                    .severity(ErrorSeverity::Info)
                    .at(file!(), line!())
                    .dedup("trace.add_project_flow.handle_refresh")
                    .build(),
                );
                fresh
            }
            None => {
                LogWriter::log(
                    ErrorReport::new("trace.add_project_flow: no active workspace to receive add")
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .dedup("trace.add_project_flow.no_target")
                        .build(),
                );
                return;
            }
        },
    };
    let weak = weak.clone();
    let update_result = cx.update_window(target_handle, move |_, window, cx_w| {
        if let Some(ws) = weak.upgrade() {
            ws.update(cx_w, |ws, cx| {
                ws.add_project(path, window, cx);
            });
        } else {
            LogWriter::log(
                ErrorReport::new("trace.add_project_flow: weak.upgrade=None (workspace dropped)")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("trace.add_project_flow.weak_dropped")
                    .build(),
            );
        }
        window.activate_window();
    });
    if let Err(e) = &update_result {
        LogWriter::log(
            ErrorReport::new("trace.add_project_flow: update_window failed")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("error", format!("{e:?}"))
                .dedup("trace.add_project_flow.update_window_err")
                .build(),
        );
    }
    // SILENT-OK: failure was logged above; this discards the Result so the
    // outer fn doesn't have to thread one through the menus-refresh tail.
    let _ = update_result;
    crate::menus::refresh_recent_menu(cx);
}

/// NewWindow path — open a fresh workspace window seeded with `path`.
/// The new workspace's `persist_state` is responsible for writing the
/// initial workspace file and refreshing the recent list (via
/// `touch_recent_in`); we don't pre-write either here.
fn open_new_workspace_for_path(
    config: std::sync::Arc<daruda_config::Config>,
    path: &std::path::Path,
    cx: &mut App,
) {
    let project = daruda_store::project::Project::from_path(path);
    let opts = build_window_options(&config);
    open_workspace_window(config.clone(), None, Some(project), opts, cx);
    crate::menus::refresh_recent_menu(cx);
}

/// Ask path — open the [`OpenProjectModal`] in the active window and
/// route the user's choice through `add_path_to_workspace` or
/// `open_new_workspace_for_path`. "Don't ask again" persists the picked
/// choice into the workspace's [`WindowOpenPolicy`].
fn open_chooser_modal(
    config: std::sync::Arc<daruda_config::Config>,
    handle: gpui::AnyWindowHandle,
    weak: gpui::WeakEntity<crate::workspace::Workspace>,
    path: std::path::PathBuf,
    cx: &mut App,
) {
    let weak_for_modal = weak.clone();
    // `handle` is consumed by `try_update_workspace_window` below; the inner
    // modal callback re-enters the workspace via its own live `&mut Window`
    // instead (matches zed's `update_in` pattern — see `app/CLAUDE.md` G9).
    try_update_workspace_window(handle, cx, "open_chooser_modal", move |window, cx_w| {
        let config = config.clone();
        crate::workspace::open_project_modal::open_choose_window_modal(
            path,
            crate::workspace::open_project_modal::OpenProjectChoice::AddHere,
            move |choice, dont_ask, picked_path, window, app_cx| {
                if dont_ask && let Some(ws) = weak_for_modal.upgrade() {
                    ws.update(app_cx, |ws, cx| {
                        let policy = match choice {
                            crate::workspace::open_project_modal::OpenProjectChoice::AddHere => {
                                daruda_store::project::WindowOpenPolicy::AddHere
                            }
                            crate::workspace::open_project_modal::OpenProjectChoice::NewWindow => {
                                daruda_store::project::WindowOpenPolicy::NewWindow
                            }
                        };
                        ws.set_window_open_policy(policy, cx);
                    });
                }
                match choice {
                    crate::workspace::open_project_modal::OpenProjectChoice::AddHere => {
                        // The modal callback already runs inside the
                        // workspace window's context (OpenProjectModal::submit
                        // re-enters via `cx.defer` + `update_window` on the
                        // workspace handle). Calling `add_path_to_workspace`
                        // here would re-enter `update_window` a second time
                        // on the same window — GPUI flags that as "window
                        // not found". Use the live `window` + `weak entity`
                        // directly (matches zed's `update_in` pattern).
                        if let Some(ws) = weak_for_modal.upgrade() {
                            ws.update(app_cx, |ws, cx| {
                                ws.add_project(picked_path.clone(), window, cx);
                            });
                            window.activate_window();
                        }
                        crate::menus::refresh_recent_menu(app_cx);
                    }
                    crate::workspace::open_project_modal::OpenProjectChoice::NewWindow => {
                        open_new_workspace_for_path(config.clone(), &picked_path, app_cx);
                    }
                }
            },
            window,
            cx_w,
        );
    });
}

/// Return the handle of the window that should be closed when a
/// `ReplaceCurrent` open fires. Every window that can initiate one is a
/// registered Workspace, including the empty one showing Landing, so the
/// registry is the whole answer.
fn active_window_to_close(cx: &App) -> Option<gpui::AnyWindowHandle> {
    WindowRegistry::active_workspace_handle(cx)
}

/// Open a workspace window and, if `mode == ReplaceCurrent`, close
/// `window_to_close` on the next tick. Passing `None` skips the
/// close step (used when there is no initiating window to replace).
/// An empty workspace showing Landing is a valid target like any other:
/// replacing it is what keeps a recent-row click from leaving it behind.
pub(crate) fn open_project_with_mode(
    config: std::sync::Arc<daruda_config::Config>,
    saved: Option<(WorkspaceState, Vec<ProjectState>)>,
    project: Option<daruda_store::project::Project>,
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
    open_workspace_window(config, saved, project, opts, cx);
    let Some(handle) = target else {
        return;
    };
    // Sequencing assumption: `open_workspace_window` runs synchronously
    // and has already queued the new window before we spawn here, so
    // `remove_window` on the next tick targets only the captured handle,
    // never the newcomer.
    cx.spawn(async move |cx| {
        // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
        cx.update(|cx| {
            // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
            let _ = cx.update_window(handle, |_, window, _| {
                window.remove_window();
            });
        });
    })
    .detach();
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
        // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
        cx.update(|cx| {
            for handle in targets {
                // SILENT-OK: window or process may exit during async picker / close-loop / registry iteration
                let _ = cx.update_window(handle, |_, window, _| {
                    window.remove_window();
                });
            }
        });
    })
    .detach();
}
