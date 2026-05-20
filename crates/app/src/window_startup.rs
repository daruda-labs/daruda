//! First-window decision tree at app launch.
//!
//! Checks for a recent workspace to restore. If the user's most-
//! recently-opened workspace's state file is on disk, reopen it with
//! the saved multi-project layout; otherwise show the Welcome window.
//!
//! The native menu bar is installed here too — deferred until the
//! recent list is loaded so File > Open Recent shows live entries
//! on first paint. Menu rebuilds afterwards (e.g. after a recent
//! list edit) go through `menus::refresh_recent_menu`.

use crate::menus;
use crate::windows::{open_welcome_window, open_workspace_window};
use gpui::{App, WindowOptions};
use std::sync::Arc;

pub(crate) fn open_first_window(
    config: Arc<daruda_config::Config>,
    window_opts: WindowOptions,
    cx: &mut App,
) {
    let data_dir = daruda_store::persistence::default_data_dir();
    let recent = daruda_store::project::load_recent_in(&data_dir);
    let restored = recent.first().and_then(|entry| {
        daruda_store::project::load_workspace_state_in(&data_dir, entry.workspace_uuid).map(|ws| {
            let projects: Vec<_> = ws
                .project_ids
                .iter()
                .filter_map(|p| daruda_store::project::load_project_state_in(&data_dir, *p))
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
        open_welcome_window(config, window_opts, cx);
    }

    // Install the native menu bar. Deferred until the recent list
    // is loaded so File > Open Recent shows live entries.
    cx.set_menus(menus::build_menu_bar(&recent));
}
