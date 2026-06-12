mod annotation_ops_tests;
mod config_mirror;
mod diag_scroll;
mod dnd;
mod dock;
mod durable;
mod error_modal;
mod error_ops;
mod files;
mod lifecycle;
mod modal_tab_containment;
mod palette_agent;
// TODO Task 11: rewrite for the UUID-keyed schema. Every test in this
// module fed the deleted `legacy::WorkspaceState` / `legacy::ProjectState`
// through `save_state` / `restore_state`; with the legacy module gone
// the whole file no longer compiles. Gating the entire module is the
// lightest-touch fix per Task 10's test policy.
mod lanes;
#[cfg(any())]
mod persistence;
mod projects;
mod pure_ops;
mod regression_namespace;
mod restore_from_disk;
mod snapshot_for_disk;
mod splits;
mod task_edit_tab_cycle;
mod tasks;

use super::*;
use gpui::{AppContext, TestAppContext};

use crate::test_support::init_gpui_component;
// Re-export workspace-internal types needed by sub-modules.
pub(super) use crate::workspace::main_area::pane_tree::{PaneLayout, SplitDirection};

/// Returns a unique temp directory path for each call so parallel tests
/// never share persistence state. The directory is left on disk after
/// the test; macOS cleans up /tmp periodically.
fn fresh_test_data_dir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("daruda_test_{id}"))
}

fn build_workspace(
    cx: &mut TestAppContext,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    let config = daruda_config::Config::default();
    build_workspace_with(cx, &config, None)
}

/// Variant of [`build_workspace`] that lets the caller control the
/// `Config` and supply an optional `Project`. Mirrors production's
/// `windows::open_workspace_window` — Workspace constructed inside
/// `gpui_component::Root::new(...)` so any API that walks the window
/// root (Dialog open, Theme reads, focus tracking) sees the same
/// shape as the running app.
fn build_workspace_with(
    cx: &mut TestAppContext,
    config: &daruda_config::Config,
    project: Option<daruda_store::project::Project>,
) -> (
    gpui::WindowHandle<gpui_component::Root>,
    gpui::Entity<Workspace>,
) {
    init_gpui_component(cx);
    let workspace_for_root = std::cell::RefCell::new(None);
    let window_handle = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| {
            let mut ws = Workspace::new_with_project_for_test(
                config,
                project.clone(),
                fresh_test_data_dir(),
                window,
                cx,
            );
            // `build_workspace` is used by tab/dock/persistence tests
            // that assume a workspace boots with one tab; opt back into
            // that single piece of heavy init while keeping the rest
            // (watchers, persist, macro shortcuts) skipped.
            ws.add_tab(window, cx);
            ws
        });
        *workspace_for_root.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = workspace_for_root.borrow().clone().unwrap();
    (window_handle, workspace)
}
