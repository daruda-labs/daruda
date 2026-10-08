//! daruda — Loop Terminal
//!
//! Dev loop accelerator for Claude Code.
//! Built on GPUI (Metal rendering) + ghostty_vt (Zig SIMD terminal emulation).

#![cfg_attr(windows, windows_subsystem = "windows")]

rust_i18n::i18n!("locales", fallback = "en");

pub mod agent;
mod app_presence;
mod assets;
mod bind_keys;
mod bootstrap;
mod config_watcher;
mod control;
mod diagnostics;
mod dir_watch;
mod env_strip;
pub(crate) mod file_name;
pub mod files;
mod fuzzy;
mod globals;
mod hooks;
pub use daruda_project::lane;
pub(crate) mod menus;
mod orchestrator;
mod panels_watcher;
pub(crate) mod path_ext;
mod platform;
pub use daruda_project::project;
mod remote_channel;
#[cfg(feature = "replay")]
mod replay;
#[cfg(feature = "screenshot")]
mod screenshot;
pub mod settings;
pub mod settings_store;
mod shell_env;
mod slot_actions;
mod smoke;
mod startup;
pub mod surface;
mod telegram;
#[cfg(test)]
mod test_support;
mod title_bar;
mod transcript;
pub mod ui;
mod update;
mod watcher_pumps;
mod watchers_lifecycle;
mod window_placement;
pub(crate) mod window_registry;
mod window_startup;
mod windows;
mod workspace;
mod workspace_storage;

use gpui::{App, MenuItem, actions};
use windows::OpenMode;

/// Open a recent workspace by identity. The File menu's `OpenRecent*` slots
/// address the list by index, which is fine for a menu gpui rebuilds from the
/// same load. The Landing view renders from a cached snapshot, so an index
/// there can drift from the list the handler reloads — a row would then open
/// a workspace other than the one it names. Carrying the
/// [`daruda_store::project::WorkspaceUuid`] removes that gap, and the
/// `OPEN_RECENT_SLOTS` cap with it.
#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = daruda, no_json)]
pub struct OpenRecentWorkspace(pub daruda_store::project::WorkspaceUuid);

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
        ExportDiagnostics,
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
                            crate::workspace_storage::current(cx).load_recent(),
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
                            crate::workspace_storage::current(cx).load_recent(),
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
    startup::run();
}
