//! daruda — Loop Terminal
//!
//! Dev loop accelerator for Claude Code.
//! Built on GPUI (Metal rendering) + ghostty_vt (Zig SIMD terminal emulation).

pub mod agent;
mod assets;
mod bind_keys;
mod bootstrap;
mod config_watcher;
pub mod files;
mod globals;
mod hooks;
mod menus;
mod panels_watcher;
pub(crate) mod path_ext;
mod platform;
pub mod settings_store;
pub mod settings_window;
pub(crate) mod shell_quote;
mod slot_actions;
pub mod surface;
#[cfg(test)]
mod test_support;
pub mod ui;
mod watcher_pumps;
mod watchers_lifecycle;
pub mod welcome;
pub(crate) mod window_registry;
mod window_startup;
mod windows;
mod workspace;
pub mod worktree;

use gpui::{App, MenuItem, actions};
use windows::OpenMode;

actions!(
    daruda,
    [
        Quit,
        OpenFolder,
        OpenFolderInNewWindow,
        NewEmptyWindow,
        CloseProject,
        OpenRecent0,
        OpenRecent1,
        OpenRecent2,
        OpenRecent3,
        OpenRecent4,
        OpenRecent5,
        OpenRecent6,
        OpenRecent7,
        OpenRecent8,
        OpenRecent9,
        OpenRecentInNewWindow0,
        OpenRecentInNewWindow1,
        OpenRecentInNewWindow2,
        OpenRecentInNewWindow3,
        OpenRecentInNewWindow4,
        OpenRecentInNewWindow5,
        OpenRecentInNewWindow6,
        OpenRecentInNewWindow7,
        OpenRecentInNewWindow8,
        OpenRecentInNewWindow9,
        // Help menu — external URL openers
        OpenDarudaHelp,
        OpenReportIssue,
        OpenGithubRepo,
    ]
);

/// Single source of truth for the recent-project slots. Every other
/// site that needs to map a slot index to an action (menu builder,
/// action registration, slot-count constant) derives from this list
/// via `recent_slot_table!` below.
macro_rules! recent_slot_table {
    ( $( $idx:literal => ($replace:ident, $new_window:ident) ),* $(,)? ) => {
        /// How many recent-project slots the File menu reserves.
        pub(crate) const OPEN_RECENT_SLOTS: usize = [$($idx),*].len();

        /// Map a slot index + mode to the matching `OpenRecent*`
        /// action variant. Falls back to the last-slot action when
        /// `idx >= OPEN_RECENT_SLOTS` (callers guarantee the range,
        /// but keymap overrides could in principle fire out of bounds).
        pub(crate) fn recent_action_for_slot(
            idx: usize,
            label: gpui::SharedString,
            mode: OpenMode,
        ) -> MenuItem {
            match (idx, mode) {
                $(
                    ($idx, OpenMode::ReplaceCurrent) => {
                        MenuItem::action(label, $replace)
                    }
                    ($idx, OpenMode::NewWindow) => {
                        MenuItem::action(label, $new_window)
                    }
                )*
                _ => unreachable!("slot {idx} outside declared recent_slot_table range"),
            }
        }

        /// Register all 2×N recent-project action handlers in one
        /// sweep. Each click reloads the recent list from disk so the
        /// File > Open Recent submenu can stay live (re-built via
        /// `menus::refresh_recent_menu`) without the action handlers
        /// dispatching against a stale snapshot captured at launch.
        fn register_recent_actions(
            cx: &mut App,
            config: std::sync::Arc<daruda_config::Config>,
        ) {
            $(
                {
                    let cfg_replace = config.clone();
                    cx.on_action(move |_: &$replace, cx: &mut App| {
                        let recent = std::sync::Arc::new(
                            daruda_store::project::persistence::load_recent(),
                        );
                        windows::open_recent_idx(
                            $idx,
                            recent,
                            cfg_replace.clone(),
                            OpenMode::ReplaceCurrent,
                            cx,
                        );
                        cx.stop_propagation();
                    });
                }
                {
                    let cfg_new = config.clone();
                    cx.on_action(move |_: &$new_window, cx: &mut App| {
                        let recent = std::sync::Arc::new(
                            daruda_store::project::persistence::load_recent(),
                        );
                        windows::open_recent_idx(
                            $idx,
                            recent,
                            cfg_new.clone(),
                            OpenMode::NewWindow,
                            cx,
                        );
                        cx.stop_propagation();
                    });
                }
            )*
        }
    };
}

recent_slot_table! {
    0 => (OpenRecent0, OpenRecentInNewWindow0),
    1 => (OpenRecent1, OpenRecentInNewWindow1),
    2 => (OpenRecent2, OpenRecentInNewWindow2),
    3 => (OpenRecent3, OpenRecentInNewWindow3),
    4 => (OpenRecent4, OpenRecentInNewWindow4),
    5 => (OpenRecent5, OpenRecentInNewWindow5),
    6 => (OpenRecent6, OpenRecentInNewWindow6),
    7 => (OpenRecent7, OpenRecentInNewWindow7),
    8 => (OpenRecent8, OpenRecentInNewWindow8),
    9 => (OpenRecent9, OpenRecentInNewWindow9),
}

fn main() {
    // `daruda --hook <eventType>` is a non-GUI subcommand invoked by
    // Claude Code's hook system. Handle it before constructing the
    // GPUI App so spawning a hook doesn't open a window or attach to
    // the desktop session.
    if let Some(code) = bootstrap::route_hook_subcommand() {
        std::process::exit(code);
    }

    bootstrap::init_observability();

    let app = bootstrap::new_application();
    app.run(|cx: &mut App| {
        globals::init_all(cx);
        bind_keys::register_static_bindings(cx);

        // SettingsStore is the single source of truth — read the
        // user layer directly instead of re-reading disk.
        let config = crate::settings_store::SettingsStore::global(cx).user_arc();
        surface::action_map::apply_keybinding_overrides(&config.keybindings.bindings, cx);

        let window_opts = windows::build_window_options(&config);

        bind_keys::register_global_actions(cx, config.clone());
        register_recent_actions(cx, config.clone());

        window_startup::open_first_window(config, window_opts, cx);

        watchers_lifecycle::spawn_all(cx);
    });
}
