//! First-window decision tree at app launch.
//!
//! Checks for a recent project to restore. If the user's most
//! recent project has a saved state file, reopen the workspace
//! with that layout; otherwise show the Welcome window.
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
    let recent = daruda_store::project::persistence::load_recent();
    let restored_state = recent
        .first()
        .and_then(|entry| daruda_store::project::persistence::load_state(&entry.root));

    if let Some(state) = restored_state {
        let project = daruda_store::project::Project::from_path(&state.root);
        open_workspace_window(
            config.clone(),
            Some(project),
            Some(state),
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
