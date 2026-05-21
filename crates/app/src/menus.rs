//! Native macOS menu bar construction.
//!
//! All user-visible labels live in [`crate::surface::strings`]; this
//! module only wires actions to menu rows. The Open Recent submenu
//! takes the recent-projects snapshot at build time so each launch
//! shows the live list.

use daruda_terminal::view::{
    ClearBuffer, ClearScrollback, Copy, Paste, PromptJumpNext, PromptJumpPrev, SearchNext,
    SearchOpen, SearchPrev, SelectAll,
};
use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};

use crate::surface::strings as s;
use crate::windows::OpenMode;
use crate::workspace::{
    ClosePane, CloseTab, EditWindowTitle, FocusNextPane, FocusPaneDown, FocusPaneLeft,
    FocusPaneRight, FocusPaneUp, FocusPrevPane, MinimizeWindow, MoveTabLeft, MoveTabRight, NewTab,
    OpenProjectConfig, OpenSettings, SplitDown, SplitRight, ToggleBottomDock, ToggleCommandPalette,
    ToggleFullScreen, ToggleLeftDock, ToggleRightDock, ZoomWindow,
};
use crate::{
    CloseProject, NewEmptyWindow, OPEN_RECENT_SLOTS, OpenDarudaHelp, OpenFolder,
    OpenFolderInNewWindow, OpenGithubRepo, OpenReportIssue, Quit, recent_action_for_slot,
};

/// Build the File > Open Recent (or "… in New Window") submenu items.
/// Empty recent list produces a single placeholder row so the submenu
/// doesn't collapse to nothing.
pub(crate) fn build_recent_submenu(
    recent: &[daruda_store::project::RecentEntry],
    mode: OpenMode,
) -> Vec<MenuItem> {
    if recent.is_empty() {
        // Placeholder bound to the slot-0 action; handler no-ops
        // because `recent.get(0)` returns None.
        return vec![recent_action_for_slot(
            0,
            gpui::SharedString::from(s::MENU_NO_RECENT),
            mode,
        )];
    }
    recent
        .iter()
        .take(OPEN_RECENT_SLOTS)
        .enumerate()
        .map(|(idx, entry)| {
            recent_action_for_slot(
                idx,
                gpui::SharedString::from(entry.display_name.clone()),
                mode,
            )
        })
        .collect()
}

/// Re-load the recent-projects list from disk and refresh the entire
/// menu bar. Call after every successful `touch_recent_in` so File >
/// Open Recent stays current without requiring a relaunch.
pub(crate) fn refresh_recent_menu(cx: &mut App) {
    let recent =
        daruda_store::project::load_recent_in(&daruda_store::persistence::default_data_dir());
    cx.set_menus(build_menu_bar(&recent));
}

/// Build the entire native menu bar. Kept in one helper so the File
/// menu's Recent submenu can be rebuilt with fresh data on launch and
/// after each `touch_recent_in` via [`refresh_recent_menu`].
pub(crate) fn build_menu_bar(recent: &[daruda_store::project::RecentEntry]) -> Vec<Menu> {
    vec![
        Menu {
            name: s::MENU_APP.into(),
            items: vec![
                MenuItem::separator(),
                MenuItem::action(
                    s::MENU_SETTINGS,
                    OpenSettings(daruda_config::BuiltinSection::default()),
                ),
                MenuItem::action(s::MENU_OPEN_PROJECT_CONFIG, OpenProjectConfig),
                MenuItem::separator(),
                MenuItem::os_submenu(s::MENU_SERVICES, SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(s::menu_quit_app(), Quit),
            ],
        },
        Menu {
            name: s::MENU_FILE.into(),
            items: vec![
                MenuItem::action(s::menu_new_window(), NewEmptyWindow),
                MenuItem::action(s::menu_open(), OpenFolder),
                MenuItem::action(s::MENU_OPEN_IN_NEW_WINDOW, OpenFolderInNewWindow),
                MenuItem::submenu(Menu {
                    name: s::MENU_OPEN_RECENT.into(),
                    items: build_recent_submenu(recent, OpenMode::ReplaceCurrent),
                }),
                MenuItem::submenu(Menu {
                    name: s::MENU_OPEN_RECENT_IN_NEW_WINDOW.into(),
                    items: build_recent_submenu(recent, OpenMode::NewWindow),
                }),
                MenuItem::separator(),
                MenuItem::action(s::menu_close_project(), CloseProject),
                MenuItem::separator(),
                MenuItem::action(s::menu_new_tab(), NewTab),
                MenuItem::action(s::MENU_CLOSE_PANE, ClosePane),
                MenuItem::action(s::menu_close_tab(), CloseTab),
            ],
        },
        Menu {
            name: s::MENU_EDIT.into(),
            items: vec![
                MenuItem::os_action(s::MENU_COPY, Copy, OsAction::Copy),
                MenuItem::os_action(s::MENU_PASTE, Paste, OsAction::Paste),
                MenuItem::os_action(s::MENU_SELECT_ALL, SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action(s::MENU_FIND, SearchOpen),
                MenuItem::action(s::MENU_FIND_NEXT, SearchNext),
                MenuItem::action(s::MENU_FIND_PREV, SearchPrev),
                MenuItem::separator(),
                MenuItem::action(s::MENU_CLEAR_BUFFER, ClearBuffer),
                MenuItem::action(s::MENU_CLEAR_SCROLLBACK, ClearScrollback),
            ],
        },
        Menu {
            name: s::MENU_VIEW.into(),
            items: vec![
                MenuItem::action(s::menu_split_right(), SplitRight),
                MenuItem::action(s::menu_split_down(), SplitDown),
                MenuItem::separator(),
                MenuItem::action(s::MENU_NEXT_PANE, FocusNextPane),
                MenuItem::action(s::MENU_PREV_PANE, FocusPrevPane),
                MenuItem::separator(),
                MenuItem::action(s::MENU_FOCUS_PANE_LEFT, FocusPaneLeft),
                MenuItem::action(s::MENU_FOCUS_PANE_RIGHT, FocusPaneRight),
                MenuItem::action(s::MENU_FOCUS_PANE_UP, FocusPaneUp),
                MenuItem::action(s::MENU_FOCUS_PANE_DOWN, FocusPaneDown),
                MenuItem::separator(),
                MenuItem::action(s::MENU_MOVE_TAB_LEFT, MoveTabLeft),
                MenuItem::action(s::MENU_MOVE_TAB_RIGHT, MoveTabRight),
                MenuItem::separator(),
                MenuItem::action(s::menu_toggle_full_screen(), ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action(s::MENU_TOGGLE_LEFT_DOCK, ToggleLeftDock),
                MenuItem::action(s::MENU_TOGGLE_BOTTOM_DOCK, ToggleBottomDock),
                MenuItem::action(s::MENU_TOGGLE_RIGHT_DOCK, ToggleRightDock),
                MenuItem::separator(),
                MenuItem::action(s::MENU_JUMP_PROMPT_PREV, PromptJumpPrev),
                MenuItem::action(s::MENU_JUMP_PROMPT_NEXT, PromptJumpNext),
            ],
        },
        Menu {
            name: s::MENU_WORKTREE.into(),
            items: crate::worktree_slot_table!(@menu_items),
        },
        Menu {
            name: s::MENU_WINDOW.into(),
            items: vec![
                MenuItem::action(s::MENU_MINIMIZE, MinimizeWindow),
                MenuItem::action(s::MENU_ZOOM, ZoomWindow),
                MenuItem::separator(),
                MenuItem::action(s::MENU_EDIT_WINDOW_TITLE, EditWindowTitle),
            ],
        },
        Menu {
            name: s::MENU_HELP.into(),
            items: vec![
                MenuItem::action(s::MENU_DARUDA_HELP, OpenDarudaHelp),
                MenuItem::separator(),
                MenuItem::action(s::MENU_KEYBOARD_SHORTCUTS, ToggleCommandPalette),
                MenuItem::action(
                    s::MENU_EDIT_KEYMAP,
                    OpenSettings(daruda_config::BuiltinSection::Keymap),
                ),
                MenuItem::separator(),
                MenuItem::action(s::MENU_REPORT_ISSUE, OpenReportIssue),
                MenuItem::action(s::MENU_GITHUB_REPO, OpenGithubRepo),
            ],
        },
    ]
}
