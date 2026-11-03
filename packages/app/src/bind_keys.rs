//! Keybinding registration + global action handlers.
//!
//! [`register_static_bindings`] sets the `cx.bind_keys` table for
//! every App / Edit / Tabs / Split / Dock / Files / Git / Window /
//! Help chord. Strings come from `surface::keybindings` so config
//! `[keybindings]` overrides them.
//!
//! [`register_global_actions`] installs the handlers those chords
//! fire (Quit, Help URLs, OpenFolder, NewEmptyWindow, CloseProject,
//! OpenSettings). `cx.on_action` registers a *bubble*-phase global
//! listener, which GPUI runs only after the focused window's own
//! listeners declined — so these are fallbacks, not overrides.

use crate::surface::{self, keybindings as k};
use crate::window_registry::WindowRegistry;
use crate::windows::{
    OpenMode, build_window_options, close_all_workspace_windows, open_workspace_window,
    prompt_and_open_folder, prompt_and_open_folder_with_policy,
};
use crate::workspace::{
    ClosePane, FileViewerSearchNext, FileViewerSearchOpen, FileViewerSearchPrev, FilesActivate,
    FilesCollapse, FilesExpand, FilesRefresh, FilesSelectNext, FilesSelectPrev, FilesToggleHidden,
    FocusNextPane, FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, FocusPrevPane,
    FocusSkillSearch, GitChangesActivate, GitChangesSelectNext, GitChangesSelectPrev,
    GitChangesToggleStage, InvokeSkillPalette, MinimizeWindow, MoveActiveProjectToGroup,
    MoveTabLeft, MoveTabRight, NewGroup, NewTab, NextTab, OpenCommandHistory, OpenSettings,
    PrevTab, QuestionNextField, QuestionPick1, QuestionPick2, QuestionPick3, QuestionPick4,
    QuestionPick5, QuestionPick6, QuestionPick7, QuestionPick8, QuestionPick9, QuestionPrevField,
    QuestionToggle, RenameActiveProject, SaveFilePane, SplitDown, SplitRight, ToggleBottomDock,
    ToggleCommandPalette, ToggleFilesFocus, ToggleFullScreen, ToggleGitChangesFocus,
    ToggleLaneSwitcher, ToggleLeftDock, ToggleRightDock,
};
use crate::{
    CloseProject, NewEmptyWindow, OpenDarudaHelp, OpenFolder, OpenFolderInNewWindow,
    OpenGithubRepo, OpenRecentWorkspace, OpenReportIssue, Quit,
};
use daruda_terminal::view::{Copy, Paste, SelectAll};
use gpui::{App, AppContext as _, KeyBinding};

/// Resolve host modifier aliases for both built-in and configured bindings.
pub(crate) fn binding<A: gpui::Action>(
    keys: &str,
    action: A,
    context: Option<&str>,
    cx: &App,
) -> KeyBinding {
    KeyBinding::load(
        keys,
        Box::new(action),
        context.map(|value| {
            gpui::KeyBindingContextPredicate::parse(value)
                .expect("valid key context")
                .into()
        }),
        false,
        None,
        cx,
    )
    .expect("valid keybinding")
}

pub(crate) fn register_static_bindings(cx: &mut App) {
    cx.bind_keys([
        // App
        binding(k::SHORTCUT_QUIT, Quit, None, cx),
        binding(
            k::SHORTCUT_SETTINGS,
            OpenSettings(daruda_config::BuiltinSection::default()),
            None,
            cx,
        ),
        // Edit
        binding(k::SHORTCUT_SELECT_ALL, SelectAll, None, cx),
        binding(k::SHORTCUT_COPY, Copy, None, cx),
        binding(k::SHORTCUT_PASTE, Paste, None, cx),
        // Tabs
        binding(k::SHORTCUT_NEW_TAB, NewTab, None, cx),
        binding(k::SHORTCUT_CLOSE_PANE, ClosePane, None, cx),
        // Split
        binding(k::SHORTCUT_SPLIT_RIGHT, SplitRight, None, cx),
        binding(k::SHORTCUT_SPLIT_DOWN, SplitDown, None, cx),
        binding(k::SHORTCUT_FOCUS_NEXT_PANE, FocusNextPane, None, cx),
        binding(k::SHORTCUT_FOCUS_PREV_PANE, FocusPrevPane, None, cx),
        // Directional pane focus (iTerm2: cmd-alt-arrow).
        binding(k::SHORTCUT_FOCUS_PANE_LEFT, FocusPaneLeft, None, cx),
        binding(k::SHORTCUT_FOCUS_PANE_RIGHT, FocusPaneRight, None, cx),
        binding(k::SHORTCUT_FOCUS_PANE_UP, FocusPaneUp, None, cx),
        binding(k::SHORTCUT_FOCUS_PANE_DOWN, FocusPaneDown, None, cx),
        // Tab move (iTerm2: cmd-shift-ctrl-arrow).
        binding(k::SHORTCUT_MOVE_TAB_LEFT, MoveTabLeft, None, cx),
        binding(k::SHORTCUT_MOVE_TAB_RIGHT, MoveTabRight, None, cx),
        binding(k::SHORTCUT_NEXT_TAB, NextTab, None, cx),
        binding(k::SHORTCUT_PREV_TAB, PrevTab, None, cx),
        // Dock toggles
        binding(k::SHORTCUT_TOGGLE_LEFT_DOCK, ToggleLeftDock, None, cx),
        // Global on purpose: these are the only keyboard way *into* a
        // left-dock panel, so they must fire from wherever focus is.
        binding(
            k::SHORTCUT_TOGGLE_GIT_CHANGES_FOCUS,
            ToggleGitChangesFocus,
            None,
            cx,
        ),
        binding(k::SHORTCUT_TOGGLE_FILES_FOCUS, ToggleFilesFocus, None, cx),
        binding(k::SHORTCUT_TOGGLE_BOTTOM_DOCK, ToggleBottomDock, None, cx),
        binding(k::SHORTCUT_TOGGLE_RIGHT_DOCK, ToggleRightDock, None, cx),
        // Command palette
        binding(k::SHORTCUT_COMMAND_PALETTE, ToggleCommandPalette, None, cx),
        // Lane switcher (Cmd+P) — fuzzy quick-switch across lanes.
        binding(k::SHORTCUT_LANE_SWITCHER, ToggleLaneSwitcher, None, cx),
        // Skills shortcuts
        binding(k::SHORTCUT_FOCUS_SKILL_SEARCH, FocusSkillSearch, None, cx),
        binding(
            k::SHORTCUT_INVOKE_SKILL_PALETTE,
            InvokeSkillPalette,
            None,
            cx,
        ),
        // File viewer
        binding(
            k::SHORTCUT_SAVE_FILE_PANE,
            SaveFilePane,
            Some("FileViewer"),
            cx,
        ),
        binding(
            k::SHORTCUT_SAVE_FILE_PANE,
            SaveFilePane,
            Some("FlowGraphPane"),
            cx,
        ),
        binding(
            k::SHORTCUT_SAVE_FILE_PANE,
            SaveFilePane,
            Some("TaskEditPane"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILE_VIEWER_SEARCH_OPEN,
            FileViewerSearchOpen,
            Some("FileViewer"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILE_VIEWER_SEARCH_NEXT,
            FileViewerSearchNext,
            Some("FileViewerSearch"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILE_VIEWER_SEARCH_PREV,
            FileViewerSearchPrev,
            Some("FileViewerSearch"),
            cx,
        ),
        // Project
        binding(k::SHORTCUT_OPEN_FOLDER, OpenFolder, None, cx),
        binding(
            k::SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW,
            OpenFolderInNewWindow,
            None,
            cx,
        ),
        binding(k::SHORTCUT_NEW_WINDOW, NewEmptyWindow, None, cx),
        binding(k::SHORTCUT_CLOSE_PROJECT, CloseProject, None, cx),
        binding(k::SHORTCUT_NEW_GROUP, NewGroup, None, cx),
        binding(k::SHORTCUT_RENAME_PROJECT, RenameActiveProject, None, cx),
        binding(
            k::SHORTCUT_MOVE_PROJECT_TO_GROUP,
            MoveActiveProjectToGroup,
            None,
            cx,
        ),
        // Files view (global toggle)
        binding(k::SHORTCUT_FILES_TOGGLE_HIDDEN, FilesToggleHidden, None, cx),
        // Files panel keyboard navigation — only fires when the
        // dock/left-dock Files body holds focus, so arrow keys still
        // reach terminal panes by default.
        binding(
            k::SHORTCUT_FILES_SELECT_NEXT,
            FilesSelectNext,
            Some("FilesPanel"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILES_SELECT_PREV,
            FilesSelectPrev,
            Some("FilesPanel"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILES_EXPAND,
            FilesExpand,
            Some("FilesPanel"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILES_COLLAPSE,
            FilesCollapse,
            Some("FilesPanel"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILES_ACTIVATE,
            FilesActivate,
            Some("FilesPanel"),
            cx,
        ),
        binding(
            k::SHORTCUT_FILES_REFRESH,
            FilesRefresh,
            Some("FilesPanel"),
            cx,
        ),
        // Git Changes view — only fires when the left-dock Git body
        // holds focus (key_context "GitChanges"), so arrow keys still
        // reach terminal panes by default.
        binding(
            k::SHORTCUT_GIT_CHANGES_SELECT_NEXT,
            GitChangesSelectNext,
            Some("GitChanges"),
            cx,
        ),
        binding(
            k::SHORTCUT_GIT_CHANGES_SELECT_PREV,
            GitChangesSelectPrev,
            Some("GitChanges"),
            cx,
        ),
        binding(
            k::SHORTCUT_GIT_CHANGES_TOGGLE_STAGE,
            GitChangesToggleStage,
            Some("GitChanges"),
            cx,
        ),
        binding(
            k::SHORTCUT_GIT_CHANGES_ACTIVATE,
            GitChangesActivate,
            Some("GitChanges"),
            cx,
        ),
        // Window menu — Minimize / Toggle Full Screen are global so
        // they work whether terminal or settings is the
        // focused responder. Zoom has no standard macOS keystroke.
        binding(k::SHORTCUT_MINIMIZE, MinimizeWindow, None, cx),
        binding(k::SHORTCUT_TOGGLE_FULL_SCREEN, ToggleFullScreen, None, cx),
        // Help menu — ⌘/ surfaces the command palette as the
        // canonical "what shortcuts are there" UI (superset pattern);
        // ⌘⇧/ (= ⌘?) is the macOS HIG Help-menu alias.
        binding(
            k::SHORTCUT_KEYBOARD_SHORTCUTS,
            ToggleCommandPalette,
            None,
            cx,
        ),
        binding(
            k::SHORTCUT_KEYBOARD_SHORTCUTS_ALT,
            ToggleCommandPalette,
            None,
            cx,
        ),
        // ⌘⇧H — command-history picker (iTerm2 Toolbelt > Commands).
        binding(
            k::SHORTCUT_OPEN_COMMAND_HISTORY,
            OpenCommandHistory,
            None,
            cx,
        ),
    ]);

    // Cmd+1..9 tab quick-switch and Cmd+Ctrl+1..9 lane quick-switch
    // come from a shared slot table so adding a tenth slot is one line
    // in `slot_actions.rs` instead of nine across four files.
    cx.bind_keys(crate::tab_slot_table!(@bindings cx));
    cx.bind_keys(crate::lane_slot_table!(@bindings cx));
    let question_bindings = question_answer_bindings(cx);
    cx.bind_keys(question_bindings);
}

/// The composer's keys while it answers an agent's question. Each names
/// `> Input` so it matches at the input's own depth, and registering after
/// `gpui_component::init` puts it ahead of the input's Tab indent there.
/// Digits and Space need an empty composer, which the IME's marked text
/// also fills, so they never act mid-composition.
fn question_answer_bindings(cx: &App) -> Vec<KeyBinding> {
    const EMPTY: Option<&str> = Some("QuestionAnswerEmpty > Input");
    const BOUND: Option<&str> = Some("QuestionAnswer > Input");
    let pick = k::SHORTCUT_QUESTION_PICK;
    vec![
        binding(pick[0], QuestionPick1, EMPTY, cx),
        binding(pick[1], QuestionPick2, EMPTY, cx),
        binding(pick[2], QuestionPick3, EMPTY, cx),
        binding(pick[3], QuestionPick4, EMPTY, cx),
        binding(pick[4], QuestionPick5, EMPTY, cx),
        binding(pick[5], QuestionPick6, EMPTY, cx),
        binding(pick[6], QuestionPick7, EMPTY, cx),
        binding(pick[7], QuestionPick8, EMPTY, cx),
        binding(pick[8], QuestionPick9, EMPTY, cx),
        binding(k::SHORTCUT_QUESTION_TOGGLE, QuestionToggle, EMPTY, cx),
        binding(
            k::SHORTCUT_QUESTION_NEXT_FIELD,
            QuestionNextField,
            BOUND,
            cx,
        ),
        binding(
            k::SHORTCUT_QUESTION_PREV_FIELD,
            QuestionPrevField,
            BOUND,
            cx,
        ),
    ]
}

pub(crate) fn register_global_actions(cx: &mut App, config: std::sync::Arc<daruda_config::Config>) {
    cx.on_action(|_: &crate::ExportDiagnostics, cx: &mut App| crate::diagnostics::export(cx));
    // Through each window's close gate: a bare `cx.quit()` never asks
    // about running work or unsaved edits.
    cx.on_action(|_: &Quit, cx: &mut App| {
        crate::workspace::Workspace::request_quit(cx);
    });

    // Help menu — open URLs in the user's default browser.
    cx.on_action(|_: &OpenDarudaHelp, cx: &mut App| {
        cx.open_url(surface::constants::URL_HELP);
    });
    cx.on_action(|_: &OpenReportIssue, cx: &mut App| {
        cx.open_url(surface::constants::URL_REPORT_ISSUE);
    });
    cx.on_action(|_: &OpenGithubRepo, cx: &mut App| {
        cx.open_url(surface::constants::URL_GITHUB_REPO);
    });

    // Global OpenFolder handler — routes through the policy-aware chooser:
    // AddHere adds to the current window, NewWindow opens a fresh one, Ask
    // surfaces the chooser modal. A root already open somewhere short-
    // circuits to focusing that window. No Workspace handles this action,
    // so the global listener is what answers it.
    let cfg_for_open = config.clone();
    cx.on_action(move |_: &OpenFolder, cx: &mut App| {
        prompt_and_open_folder_with_policy(cfg_for_open.clone(), cx);
        cx.stop_propagation();
    });
    // `cmd-shift-o` forces a new window regardless of policy — the
    // duplicate-root focus shortcut still applies, so opening an
    // already-open root in a new window is silently coalesced into
    // focusing the existing window.
    let cfg_for_open_new = config.clone();
    cx.on_action(move |_: &OpenFolderInNewWindow, cx: &mut App| {
        prompt_and_open_folder(cfg_for_open_new.clone(), OpenMode::NewWindow, cx);
        cx.stop_propagation();
    });

    // Fallback for an `OpenSettings` no workspace window answered — the
    // menu bar's, with no window left to dispatch it through.
    cx.on_action(|action: &OpenSettings, cx: &mut App| {
        crate::windows::open_settings_in_some_workspace(action.0, cx);
        cx.stop_propagation();
    });

    // Global NewEmptyWindow handler — opens a project-less workspace,
    // which paints the Landing view.
    let cfg_for_new = config.clone();
    cx.on_action(move |_: &NewEmptyWindow, cx: &mut App| {
        let opts = build_window_options(&cfg_for_new);
        open_workspace_window(cfg_for_new.clone(), None, None, opts, cx);
        cx.stop_propagation();
    });
    // Identity-keyed recent open, dispatched by the Landing view's rows.
    // `ReplaceCurrent` is what keeps an emptied window from lingering
    // behind the workspace it just opened.
    let cfg_for_recent = config;
    cx.on_action(move |action: &OpenRecentWorkspace, cx: &mut App| {
        crate::windows::open_recent_uuid(
            action.0,
            cfg_for_recent.clone(),
            OpenMode::ReplaceCurrent,
            cx,
        );
        cx.stop_propagation();
    });
    // Global CloseProject handler — surfaces the DeleteProjectModal
    // so the user picks between "Remove from Daruda only" (safe) and
    // "Remove lanes and delete on disk" (destructive). The
    // workspace mutation runs on the modal's submit callback.
    //
    // Fallback for "no active workspace" (e.g. only Settings is open)
    // is to close every Workspace window, so the user is never stuck
    // with an unresponsive shortcut. `QuitMode::Default` (macOS
    // Explicit) keeps the app running past the last closed window.
    cx.on_action(move |_: &CloseProject, cx: &mut App| {
        let Some((handle, weak)) = WindowRegistry::active_workspace(cx) else {
            close_all_workspace_windows(cx);
            cx.stop_propagation();
            return;
        };
        let project_name = match weak.upgrade() {
            Some(ws) => {
                let read_res = cx.update_window(handle, |_, _, cx_w| {
                    ws.read(cx_w).active_project_name().unwrap_or_default()
                });
                read_res.unwrap_or_default()
            }
            None => String::new(),
        };
        if project_name.is_empty() {
            cx.stop_propagation();
            return;
        }
        let weak_for_modal = weak.clone();
        crate::windows::try_update_workspace_window(
            handle,
            cx,
            "close_project.open_modal",
            |window, cx_w| {
                crate::workspace::delete_project_modal::open_delete_project_modal(
                    project_name,
                    move |choice, _window, app_cx| {
                        let Some(ws) = weak_for_modal.upgrade() else {
                            return;
                        };
                        crate::windows::try_update_workspace_window(
                            handle,
                            app_cx,
                            "close_project.modal_callback",
                            |window, cx_w| match choice {
                                crate::workspace::delete_project_modal::DeleteProjectChoice::KeepOnDisk => {
                                    // Closing the last project empties the
                                    // workspace; the window stays and paints
                                    // Landing rather than being destroyed.
                                    ws.update(cx_w, |ws, cx| {
                                        ws.close_active_project(window, cx);
                                    });
                                }
                                crate::workspace::delete_project_modal::DeleteProjectChoice::DeleteOnDisk => {
                                    ws.update(cx_w, |ws, cx| {
                                        ws.delete_active_project_on_disk(window, cx);
                                    });
                                }
                            },
                        );
                    },
                    window,
                    cx_w,
                );
            },
        );
        cx.stop_propagation();
    });
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;

    /// The catalogue, read rather than listed.
    ///
    /// A hand-written list covers the constants someone remembered to add to
    /// it, which is the wrong set — the one that matters is the constant
    /// added without it. Reading the source covers a new chord the moment it
    /// exists, and picks up *both* arms of `per_platform`, including the
    /// spelling this host will never resolve and so could never typo-check.
    const CATALOGUE: &str = include_str!("surface/keybindings.rs");

    /// Every chord literal in the catalogue, in source order.
    fn chords() -> Vec<String> {
        let mut out = Vec::new();
        let mut pending = String::new();
        for line in CATALOGUE.lines() {
            if pending.is_empty() && !line.starts_with("pub const SHORTCUT_") {
                continue;
            }
            pending.push_str(line);
            // A `per_platform(...)` initializer may wrap onto a second line.
            if !line.trim_end().ends_with(';') {
                continue;
            }
            let mut rest = pending.as_str();
            while let Some(open) = rest.find('"') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else { break };
                out.push(after[..close].to_owned());
                rest = &after[close + 1..];
            }
            pending.clear();
        }
        out
    }

    /// A shortcut is a plain string until startup hands it to gpui, so a
    /// typo is silent: the binding never fires and nothing says why.
    #[test]
    fn every_chord_in_the_catalogue_parses() {
        let chords = chords();

        assert!(
            chords.len() >= 80,
            "found only {} chords — has the catalogue moved?",
            chords.len()
        );
        for chord in &chords {
            assert!(
                Keystroke::parse_with_os(chord, gpui::OperatingSystem::Unknown).is_ok(),
                "`{chord}` is not a keystroke gpui can parse"
            );
        }
    }

    /// `secondary` *is* ctrl off macOS, so a chord naming both resolves to
    /// `ctrl-ctrl-…`. Those carry two spellings through `per_platform`; this
    /// is what stops one being written by hand instead.
    #[test]
    fn no_chord_names_both_secondary_and_ctrl() {
        for chord in chords() {
            assert!(
                !(chord.contains("secondary") && chord.contains("ctrl")),
                "`{chord}` would resolve to ctrl-ctrl off macOS"
            );
        }
    }

    #[test]
    fn secondary_follows_the_host_operating_system() {
        for (os, expected) in [
            (gpui::OperatingSystem::Mac, "cmd-c"),
            (gpui::OperatingSystem::Linux, "ctrl-c"),
            (gpui::OperatingSystem::Windows, "ctrl-c"),
        ] {
            let secondary = Keystroke::parse_with_os("secondary-c", os).expect("parses");
            assert_eq!(
                secondary.modifiers,
                Keystroke::parse(expected).expect("parses").modifiers
            );
        }
    }
}
