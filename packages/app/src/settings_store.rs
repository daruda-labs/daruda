//! The user-config store lives in `daruda_settings`; the app keeps it under
//! its old name and starts the file watch that feeds it.

pub use daruda_settings::store::*;

use crate::config_watcher;
use gpui::{App, BorrowAppContext as _};
use std::path::PathBuf;

/// Spawn the `config.toml` watcher and refresh the global on debounced reloads.
/// Must run once after [`SettingsStore::init`]; the moved handle keeps the OS
/// watcher and debounce thread alive for the task lifetime.
pub fn spawn_file_watch(cx: &mut App) {
    let handle = config_watcher::spawn_config_watcher();
    let tick = std::time::Duration::from_millis(250);
    cx.spawn(async move |cx| {
        let handle = handle;
        loop {
            cx.background_executor().timer(tick).await;
            if handle.try_recv().is_err() {
                continue;
            }
            while handle.try_recv().is_ok() {}
            cx.update(|cx| {
                cx.update_global::<SettingsStore, _>(|store, _| store.reload_user());
            });
        }
    })
    .detach();
}

/// Convenience: probe the user config path. Mirrors [`daruda_config::config_path`]
/// but lives here too so consumers that already imported
/// `daruda_config::store::*` don't need a second `use`.
pub fn user_config_path() -> PathBuf {
    daruda_config::config_path()
}
