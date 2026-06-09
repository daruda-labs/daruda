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
            gpui::SharedString::from(s::menu_no_recent()),
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
            name: s::menu_app().into(),
            disabled: false,
            items: vec![
                MenuItem::separator(),
                MenuItem::action(
                    s::menu_settings(),
                    OpenSettings(daruda_config::BuiltinSection::default()),
                ),
                MenuItem::action(s::menu_open_project_config(), OpenProjectConfig),
                MenuItem::separator(),
                MenuItem::os_submenu(s::menu_services(), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(s::menu_quit_app(), Quit),
            ],
        },
        Menu {
            name: s::menu_file().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu_new_window(), NewEmptyWindow),
                MenuItem::action(s::menu_open(), OpenFolder),
                MenuItem::action(s::menu_open_in_new_window(), OpenFolderInNewWindow),
                MenuItem::submenu(Menu {
                    name: s::menu_open_recent().into(),
                    disabled: false,
                    items: build_recent_submenu(recent, OpenMode::ReplaceCurrent),
                }),
                MenuItem::submenu(Menu {
                    name: s::menu_open_recent_in_new_window().into(),
                    disabled: false,
                    items: build_recent_submenu(recent, OpenMode::NewWindow),
                }),
                MenuItem::separator(),
                MenuItem::action(s::menu_close_project(), CloseProject),
                MenuItem::separator(),
                MenuItem::action(s::menu_new_tab(), NewTab),
                MenuItem::action(s::menu_close_pane(), ClosePane),
                MenuItem::action(s::menu_close_tab(), CloseTab),
            ],
        },
        Menu {
            name: s::menu_edit().into(),
            disabled: false,
            items: vec![
                MenuItem::os_action(s::menu_copy(), Copy, OsAction::Copy),
                MenuItem::os_action(s::menu_paste(), Paste, OsAction::Paste),
                MenuItem::os_action(s::menu_select_all(), SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action(s::menu_find(), SearchOpen),
                MenuItem::action(s::menu_find_next(), SearchNext),
                MenuItem::action(s::menu_find_prev(), SearchPrev),
                MenuItem::separator(),
                MenuItem::action(s::menu_clear_buffer(), ClearBuffer),
                MenuItem::action(s::menu_clear_scrollback(), ClearScrollback),
            ],
        },
        Menu {
            name: s::menu_view().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu_split_right(), SplitRight),
                MenuItem::action(s::menu_split_down(), SplitDown),
                MenuItem::separator(),
                MenuItem::action(s::menu_next_pane(), FocusNextPane),
                MenuItem::action(s::menu_prev_pane(), FocusPrevPane),
                MenuItem::separator(),
                MenuItem::action(s::menu_focus_pane_left(), FocusPaneLeft),
                MenuItem::action(s::menu_focus_pane_right(), FocusPaneRight),
                MenuItem::action(s::menu_focus_pane_up(), FocusPaneUp),
                MenuItem::action(s::menu_focus_pane_down(), FocusPaneDown),
                MenuItem::separator(),
                MenuItem::action(s::menu_move_tab_left(), MoveTabLeft),
                MenuItem::action(s::menu_move_tab_right(), MoveTabRight),
                MenuItem::separator(),
                MenuItem::action(s::menu_toggle_full_screen(), ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action(s::menu_toggle_left_dock(), ToggleLeftDock),
                MenuItem::action(s::menu_toggle_bottom_dock(), ToggleBottomDock),
                MenuItem::action(s::menu_toggle_right_dock(), ToggleRightDock),
                MenuItem::separator(),
                MenuItem::action(s::menu_jump_prompt_prev(), PromptJumpPrev),
                MenuItem::action(s::menu_jump_prompt_next(), PromptJumpNext),
            ],
        },
        Menu {
            name: s::menu_worktree().into(),
            disabled: false,
            items: crate::lane_slot_table!(@menu_items),
        },
        Menu {
            name: s::menu_window().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu_minimize(), MinimizeWindow),
                MenuItem::action(s::menu_zoom(), ZoomWindow),
                MenuItem::separator(),
                MenuItem::action(s::menu_edit_window_title(), EditWindowTitle),
            ],
        },
        Menu {
            name: s::menu_help().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu_daruda_help(), OpenDarudaHelp),
                MenuItem::separator(),
                MenuItem::action(s::menu_keyboard_shortcuts(), ToggleCommandPalette),
                MenuItem::action(
                    s::menu_edit_keymap(),
                    OpenSettings(daruda_config::BuiltinSection::Keymap),
                ),
                MenuItem::separator(),
                MenuItem::action(s::menu_report_issue(), OpenReportIssue),
                MenuItem::action(s::menu_github_repo(), OpenGithubRepo),
            ],
        },
    ]
}
