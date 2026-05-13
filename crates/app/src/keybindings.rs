//! Keybinding registration + global action handlers.
//!
//! [`register_static_bindings`] sets the `cx.bind_keys` table for
//! every App / Edit / Tabs / Split / Dock / Files / Git / Window /
//! Help chord. Strings come from `surface::keybindings` so config
//! `[keybindings]` overrides them.
//!
//! [`register_global_actions`] installs the handlers those chords
//! fire (Quit, Help URLs, OpenFolder, NewEmptyWindow, CloseProject).
//! Each calls `cx.stop_propagation()` so the global capture phase
//! wins over a focused Workspace's bubble phase.

use crate::surface::{self, keybindings as k};
use crate::windows::{
    OpenMode, build_window_options, close_all_workspace_windows, prompt_and_open_folder,
    open_workspace_window,
};
use crate::workspace::{
    ClosePane, FileViewerSearchNext, FileViewerSearchOpen, FileViewerSearchPrev, FilesActivate,
    FilesCollapse, FilesExpand, FilesRefresh, FilesSelectNext, FilesSelectPrev, FilesToggleHidden,
    FocusNextPane, FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, FocusPrevPane,
    FocusSkillSearch, GitChangesActivate, GitChangesSelectNext, GitChangesSelectPrev,
    GitChangesToggleStage, InvokeSkillPalette, MinimizeWindow, MoveTabLeft, MoveTabRight, NewTab,
    NextTab, OpenCommandHistory, OpenSettings, PrevTab, SplitDown, SplitRight, ToggleBottomDock,
    ToggleCommandPalette, ToggleFullScreen, ToggleLeftDock, ToggleRightDock,
};
use crate::{
    CloseProject, NewEmptyWindow, OpenDarudaHelp, OpenFolder, OpenFolderInNewWindow, OpenGithubRepo,
    OpenReportIssue, Quit,
};
use daruda_terminal::view::{Copy, Paste, SelectAll};
use gpui::{App, KeyBinding};

pub(crate) fn register_static_bindings(cx: &mut App) {
    cx.bind_keys([
        // App
        KeyBinding::new(k::SHORTCUT_QUIT, Quit, None),
        KeyBinding::new(
            k::SHORTCUT_SETTINGS,
            OpenSettings(daruda_config::BuiltinSection::default()),
            None,
        ),
        // Edit
        KeyBinding::new(k::SHORTCUT_SELECT_ALL, SelectAll, None),
        KeyBinding::new(k::SHORTCUT_COPY, Copy, None),
        KeyBinding::new(k::SHORTCUT_PASTE, Paste, None),
        // Tabs
        KeyBinding::new(k::SHORTCUT_NEW_TAB, NewTab, None),
        KeyBinding::new(k::SHORTCUT_CLOSE_PANE, ClosePane, None),
        // Split
        KeyBinding::new(k::SHORTCUT_SPLIT_RIGHT, SplitRight, None),
        KeyBinding::new(k::SHORTCUT_SPLIT_DOWN, SplitDown, None),
        KeyBinding::new(k::SHORTCUT_FOCUS_NEXT_PANE, FocusNextPane, None),
        KeyBinding::new(k::SHORTCUT_FOCUS_PREV_PANE, FocusPrevPane, None),
        // Directional pane focus (iTerm2: cmd-alt-arrow).
        KeyBinding::new(k::SHORTCUT_FOCUS_PANE_LEFT, FocusPaneLeft, None),
        KeyBinding::new(k::SHORTCUT_FOCUS_PANE_RIGHT, FocusPaneRight, None),
        KeyBinding::new(k::SHORTCUT_FOCUS_PANE_UP, FocusPaneUp, None),
        KeyBinding::new(k::SHORTCUT_FOCUS_PANE_DOWN, FocusPaneDown, None),
        // Tab move (iTerm2: cmd-shift-ctrl-arrow).
        KeyBinding::new(k::SHORTCUT_MOVE_TAB_LEFT, MoveTabLeft, None),
        KeyBinding::new(k::SHORTCUT_MOVE_TAB_RIGHT, MoveTabRight, None),
        KeyBinding::new(k::SHORTCUT_NEXT_TAB, NextTab, None),
        KeyBinding::new(k::SHORTCUT_PREV_TAB, PrevTab, None),
        // Dock toggles
        KeyBinding::new(k::SHORTCUT_TOGGLE_LEFT_DOCK, ToggleLeftDock, None),
        KeyBinding::new(k::SHORTCUT_TOGGLE_BOTTOM_DOCK, ToggleBottomDock, None),
        KeyBinding::new(k::SHORTCUT_TOGGLE_RIGHT_DOCK, ToggleRightDock, None),
        // Command palette
        KeyBinding::new(k::SHORTCUT_COMMAND_PALETTE, ToggleCommandPalette, None),
        // Skills shortcuts
        KeyBinding::new(k::SHORTCUT_FOCUS_SKILL_SEARCH, FocusSkillSearch, None),
        KeyBinding::new(k::SHORTCUT_INVOKE_SKILL_PALETTE, InvokeSkillPalette, None),
        // File viewer search
        KeyBinding::new(
            k::SHORTCUT_FILE_VIEWER_SEARCH_OPEN,
            FileViewerSearchOpen,
            Some("FileViewer"),
        ),
        KeyBinding::new(
            k::SHORTCUT_FILE_VIEWER_SEARCH_NEXT,
            FileViewerSearchNext,
            Some("FileViewerSearch"),
        ),
        KeyBinding::new(
            k::SHORTCUT_FILE_VIEWER_SEARCH_PREV,
            FileViewerSearchPrev,
            Some("FileViewerSearch"),
        ),
        // Project
        KeyBinding::new(k::SHORTCUT_OPEN_FOLDER, OpenFolder, None),
        KeyBinding::new(
            k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW,
            OpenFolderInNewWindow,
            None,
        ),
        KeyBinding::new(k::SHORTCUT_NEW_WINDOW, NewEmptyWindow, None),
        KeyBinding::new(k::SHORTCUT_CLOSE_PROJECT, CloseProject, None),
        // Files view (global toggle)
        KeyBinding::new(k::SHORTCUT_FILES_TOGGLE_HIDDEN, FilesToggleHidden, None),
        // Files panel keyboard navigation — only fires when the
        // sidebar Files body holds focus, so arrow keys still
        // reach terminal panes by default.
        KeyBinding::new(
            k::SHORTCUT_FILES_SELECT_NEXT,
            FilesSelectNext,
            Some("FilesPanel"),
        ),
        KeyBinding::new(
            k::SHORTCUT_FILES_SELECT_PREV,
            FilesSelectPrev,
            Some("FilesPanel"),
        ),
        KeyBinding::new(k::SHORTCUT_FILES_EXPAND, FilesExpand, Some("FilesPanel")),
        KeyBinding::new(
            k::SHORTCUT_FILES_COLLAPSE,
            FilesCollapse,
            Some("FilesPanel"),
        ),
        KeyBinding::new(
            k::SHORTCUT_FILES_ACTIVATE,
            FilesActivate,
            Some("FilesPanel"),
        ),
        KeyBinding::new(k::SHORTCUT_FILES_REFRESH, FilesRefresh, Some("FilesPanel")),
        // Git Changes view — only fires when the sidebar Git body
        // holds focus (key_context "GitChanges"), so arrow keys still
        // reach terminal panes by default.
        KeyBinding::new(
            k::SHORTCUT_GIT_CHANGES_SELECT_NEXT,
            GitChangesSelectNext,
            Some("GitChanges"),
        ),
        KeyBinding::new(
            k::SHORTCUT_GIT_CHANGES_SELECT_PREV,
            GitChangesSelectPrev,
            Some("GitChanges"),
        ),
        KeyBinding::new(
            k::SHORTCUT_GIT_CHANGES_TOGGLE_STAGE,
            GitChangesToggleStage,
            Some("GitChanges"),
        ),
        KeyBinding::new(
            k::SHORTCUT_GIT_CHANGES_ACTIVATE,
            GitChangesActivate,
            Some("GitChanges"),
        ),
        // Window menu — Minimize / Toggle Full Screen are global so
        // they work whether terminal, welcome, or settings is the
        // focused responder. Zoom has no standard macOS keystroke.
        KeyBinding::new(k::SHORTCUT_MINIMIZE, MinimizeWindow, None),
        KeyBinding::new(k::SHORTCUT_TOGGLE_FULL_SCREEN, ToggleFullScreen, None),
        // Help menu — ⌘/ surfaces the command palette as the
        // canonical "what shortcuts are there" UI (superset pattern);
        // ⌘⇧/ (= ⌘?) is the macOS HIG Help-menu alias.
        KeyBinding::new(k::SHORTCUT_KEYBOARD_SHORTCUTS, ToggleCommandPalette, None),
        KeyBinding::new(
            k::SHORTCUT_KEYBOARD_SHORTCUTS_ALT,
            ToggleCommandPalette,
            None,
        ),
        // ⌘⇧H — command-history picker (iTerm2 Toolbelt > Commands).
        KeyBinding::new(k::SHORTCUT_OPEN_COMMAND_HISTORY, OpenCommandHistory, None),
    ]);

    // Cmd+1..9 tab quick-switch and Cmd+Ctrl+1..9 worktree quick-switch
    // come from a shared slot table so adding a tenth slot is one line
    // in `slot_actions.rs` instead of nine across four files.
    cx.bind_keys(crate::tab_slot_table!(@bindings));
    cx.bind_keys(crate::worktree_slot_table!(@bindings));
}

pub(crate) fn register_global_actions(cx: &mut App, config: std::sync::Arc<daruda_config::Config>) {
    cx.on_action(|_: &Quit, cx: &mut App| {
        cx.quit();
    });

    // Help menu — open URLs in the user's default browser.
    cx.on_action(|_: &OpenDarudaHelp, cx: &mut App| {
        cx.open_url(surface::strings::URL_HELP);
    });
    cx.on_action(|_: &OpenReportIssue, cx: &mut App| {
        cx.open_url(surface::strings::URL_REPORT_ISSUE);
    });
    cx.on_action(|_: &OpenGithubRepo, cx: &mut App| {
        cx.open_url(surface::strings::URL_GITHUB_REPO);
    });

    // Global OpenFolder handler — picks a folder and either
    // replaces the current workspace or opens a new window,
    // depending on the action variant. `cx.stop_propagation()`
    // blocks the action from re-firing in the focused Workspace's
    // element tree (bubble phase runs after the global capture
    // phase in GPUI's dispatch pipeline).
    let cfg_for_open = config.clone();
    cx.on_action(move |_: &OpenFolder, cx: &mut App| {
        prompt_and_open_folder(cfg_for_open.clone(), OpenMode::ReplaceCurrent, cx);
        cx.stop_propagation();
    });
    let cfg_for_open_new = config.clone();
    cx.on_action(move |_: &OpenFolderInNewWindow, cx: &mut App| {
        prompt_and_open_folder(cfg_for_open_new.clone(), OpenMode::NewWindow, cx);
        cx.stop_propagation();
    });

    // Global NewEmptyWindow handler — opens a project-less
    // workspace. Shell starts in the user's home directory.
    let cfg_for_new = config;
    cx.on_action(move |_: &NewEmptyWindow, cx: &mut App| {
        let opts = build_window_options(&cfg_for_new);
        open_workspace_window(cfg_for_new.clone(), None, None, opts, cx);
        cx.stop_propagation();
    });

    // Global CloseProject handler — iTerm2 convention: close
    // all Workspace windows and let the app stay alive in the
    // background. User reopens via File > New Window / Open…
    // / Open Recent / Dock-click. `QuitMode::Default` (macOS
    // Explicit) keeps the app running past the last closed
    // window so there's no need to auto-spawn welcome.
    cx.on_action(move |_: &CloseProject, cx: &mut App| {
        close_all_workspace_windows(cx);
        cx.stop_propagation();
    });
}
