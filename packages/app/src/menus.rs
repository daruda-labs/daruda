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
            gpui::SharedString::from(s::menu::no_recent()),
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

/// The menu table as the app-drawn menu reads it back.
///
/// `App::get_menus` deep-clones the whole table — every label and a
/// `boxed_clone` per action — and the button that pops it is rebuilt inside
/// an uncached root render, so reading it there would pay that on every
/// frame off macOS. Snapshotting at the one place the table is written keeps
/// the per-frame cost at an `Rc::clone` and keeps `build_menu_bar` the single
/// definition.
pub(crate) struct MenuSnapshot(pub(crate) std::rc::Rc<Vec<gpui::OwnedMenu>>);

impl gpui::Global for MenuSnapshot {}

/// The recent-projects list as the Landing view reads it back.
///
/// `load_recent_in` is a disk read, which `render` must not do. The menu
/// bar already loads the list at every point it changes, so the same write
/// serves both readers and no second cache has to be kept in sync.
pub(crate) struct RecentSnapshot(pub(crate) std::rc::Rc<Vec<daruda_store::project::RecentEntry>>);

impl gpui::Global for RecentSnapshot {}

/// Install the application menu built from `recent` and refresh both
/// snapshots that read it back. The one entry point for all three.
pub(crate) fn set_menu_bar(recent: &[daruda_store::project::RecentEntry], cx: &mut App) {
    cx.set_menus(build_menu_bar(recent));
    let snapshot = cx.get_menus().unwrap_or_default();
    cx.set_global(MenuSnapshot(std::rc::Rc::new(snapshot)));
    cx.set_global(RecentSnapshot(std::rc::Rc::new(recent.to_vec())));
}

/// Re-load the recent-projects list from disk and refresh the entire
/// menu bar. Call after every successful `touch_recent_in` so File >
/// Open Recent stays current without requiring a relaunch.
pub(crate) fn refresh_recent_menu(cx: &mut App) {
    let recent = crate::workspace_storage::current(cx).load_recent();
    set_menu_bar(&recent, cx);
}

/// Build the entire native menu bar. Kept in one helper so the File
/// menu's Recent submenu can be rebuilt with fresh data on launch and
/// after each `touch_recent_in` via [`refresh_recent_menu`].
fn build_menu_bar(recent: &[daruda_store::project::RecentEntry]) -> Vec<Menu> {
    vec![
        Menu {
            name: s::menu::app().into(),
            disabled: false,
            items: vec![
                MenuItem::separator(),
                MenuItem::action(
                    s::menu::settings(),
                    OpenSettings(daruda_config::BuiltinSection::default()),
                ),
                MenuItem::action(s::menu::open_project_config(), OpenProjectConfig),
                MenuItem::separator(),
                MenuItem::os_submenu(s::menu::services(), SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action(s::menu::quit_app(), Quit),
            ],
        },
        Menu {
            name: s::menu::file().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu::new_window(), NewEmptyWindow),
                MenuItem::action(s::menu::open(), OpenFolder),
                MenuItem::action(s::menu::open_in_new_window(), OpenFolderInNewWindow),
                MenuItem::submenu(Menu {
                    name: s::menu::open_recent().into(),
                    disabled: false,
                    items: build_recent_submenu(recent, OpenMode::ReplaceCurrent),
                }),
                MenuItem::submenu(Menu {
                    name: s::menu::open_recent_in_new_window().into(),
                    disabled: false,
                    items: build_recent_submenu(recent, OpenMode::NewWindow),
                }),
                MenuItem::separator(),
                MenuItem::action(s::menu::close_project(), CloseProject),
                MenuItem::separator(),
                MenuItem::action(s::common::new_tab(), NewTab),
                MenuItem::action(s::common::close_pane(), ClosePane),
                MenuItem::action(s::common::close_tab(), CloseTab),
            ],
        },
        Menu {
            name: s::menu::edit().into(),
            disabled: false,
            items: vec![
                MenuItem::os_action(s::menu::copy(), Copy, OsAction::Copy),
                MenuItem::os_action(s::menu::paste(), Paste, OsAction::Paste),
                MenuItem::os_action(s::menu::select_all(), SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action(s::menu::find(), SearchOpen),
                MenuItem::action(s::menu::find_next(), SearchNext),
                MenuItem::action(s::menu::find_prev(), SearchPrev),
                MenuItem::separator(),
                MenuItem::action(s::menu::clear_buffer(), ClearBuffer),
                MenuItem::action(s::menu::clear_scrollback(), ClearScrollback),
            ],
        },
        Menu {
            name: s::menu::view().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::common::split_right(), SplitRight),
                MenuItem::action(s::common::split_down(), SplitDown),
                MenuItem::separator(),
                MenuItem::action(s::menu::next_pane(), FocusNextPane),
                MenuItem::action(s::menu::prev_pane(), FocusPrevPane),
                MenuItem::separator(),
                MenuItem::action(s::menu::focus_pane_left(), FocusPaneLeft),
                MenuItem::action(s::menu::focus_pane_right(), FocusPaneRight),
                MenuItem::action(s::menu::focus_pane_up(), FocusPaneUp),
                MenuItem::action(s::menu::focus_pane_down(), FocusPaneDown),
                MenuItem::separator(),
                MenuItem::action(s::common::move_tab_left(), MoveTabLeft),
                MenuItem::action(s::common::move_tab_right(), MoveTabRight),
                MenuItem::separator(),
                MenuItem::action(s::menu::toggle_full_screen(), ToggleFullScreen),
                MenuItem::separator(),
                MenuItem::action(s::menu::toggle_left_dock(), ToggleLeftDock),
                MenuItem::action(s::menu::toggle_bottom_dock(), ToggleBottomDock),
                MenuItem::action(s::menu::toggle_right_dock(), ToggleRightDock),
                MenuItem::separator(),
                MenuItem::action(s::menu::jump_prompt_prev(), PromptJumpPrev),
                MenuItem::action(s::menu::jump_prompt_next(), PromptJumpNext),
            ],
        },
        Menu {
            name: s::menu::worktree().into(),
            disabled: false,
            items: crate::lane_slot_table!(@menu_items),
        },
        Menu {
            name: s::menu::window().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu::minimize(), MinimizeWindow),
                MenuItem::action(s::menu::zoom(), ZoomWindow),
                MenuItem::separator(),
                MenuItem::action(s::menu::edit_window_title(), EditWindowTitle),
            ],
        },
        Menu {
            name: s::menu::help().into(),
            disabled: false,
            items: vec![
                MenuItem::action(s::menu::daruda_help(), OpenDarudaHelp),
                MenuItem::separator(),
                MenuItem::action(s::menu::keyboard_shortcuts(), ToggleCommandPalette),
                MenuItem::action(
                    s::menu::edit_keymap(),
                    OpenSettings(daruda_config::BuiltinSection::Keymap),
                ),
                MenuItem::separator(),
                MenuItem::action(s::menu::report_issue(), OpenReportIssue),
                MenuItem::action(s::menu::export_diagnostics(), crate::ExportDiagnostics),
                MenuItem::action(s::menu::github_repo(), OpenGithubRepo),
            ],
        },
    ]
}
