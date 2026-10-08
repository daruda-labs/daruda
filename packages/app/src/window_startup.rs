//! First-window decision tree at app launch.
//!
//! Checks for a recent workspace to restore. If the user's most-
//! recently-opened workspace's state file is on disk, reopen it with
//! the saved multi-project layout; otherwise open an empty workspace,
//! which paints the Landing view.
//!
//! The native menu bar is installed here too — deferred until the
//! recent list is loaded so File > Open Recent shows live entries
//! on first paint. Menu rebuilds afterwards (e.g. after a recent
//! list edit) go through `menus::refresh_recent_menu`.

use crate::menus;
use crate::windows::open_workspace_window;
use gpui::{App, WindowOptions};
use std::sync::Arc;

pub(crate) fn open_first_window(
    config: Arc<daruda_config::Config>,
    window_opts: WindowOptions,
    cx: &mut App,
) {
    let store = crate::workspace_storage::current(cx);
    let recent = store.load_recent();
    let restored = recent.first().and_then(|entry| {
        store.load_workspace(entry.workspace_uuid).map(|ws| {
            let projects: Vec<_> = ws
                .project_ids
                .iter()
                .filter_map(|p| store.load_project(*p))
                .collect();
            (ws, projects)
        })
    });

    if let Some((ws_state, project_states)) = restored {
        open_workspace_window(
            config.clone(),
            Some((ws_state, project_states)),
            None,
            window_opts,
            cx,
        );
    } else {
        // Nothing to restore — an empty workspace, which paints Landing.
        open_workspace_window(config, None, None, window_opts, cx);
    }

    // Install the native menu bar. Deferred until the recent list
    // is loaded so File > Open Recent shows live entries.
    menus::set_menu_bar(&recent, cx);
}
