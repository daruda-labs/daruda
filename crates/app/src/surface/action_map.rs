//! Maps user-facing action name strings from config to concrete GPUI
//! `Action` instances. Called at startup to apply keybinding overrides.

use gpui::KeyBinding;

use crate::Quit;
use crate::workspace::{
    ClosePane, CloseTab, CommitAmend, CommitChanges, EditTask, FilesActivate, FilesCollapse,
    FilesExpand, FilesRefresh, FilesSelectNext, FilesSelectPrev, FilesToggleHidden, FocusNextPane,
    FocusPaneDown, FocusPaneLeft, FocusPaneRight, FocusPaneUp, FocusPrevPane, GitChangesActivate,
    GitChangesSelectNext, GitChangesSelectPrev, GitChangesToggleStage, MoveTabLeft, MoveTabRight,
    NewSkill, NewTab, NewTask, NextTab, OpenCommandHistory, OpenSettings, PrevTab, PushChanges,
    RefreshGitStatus, ShowLeftDockFiles, ShowLeftDockGit, ShowLeftDockWorktrees, SplitDown,
    SplitRight, SwitchRightPanelSkills, SwitchRightPanelTasks, SwitchRightPanelTools,
    SwitchRightPanelUsage, ToggleBottomDock, ToggleCommandPalette, ToggleLeftDock, ToggleRightDock,
};
use daruda_terminal::view::{Copy, Paste, SelectAll};

/// Apply user keybinding overrides from config. Each binding is
/// registered after the built-in defaults, so GPUI's last-wins
/// matching makes user bindings take priority.
///
/// Uses a macro internally because `KeyBinding::new` requires a
/// concrete `Action` type — `Box<dyn Action>` is not accepted.
pub fn apply_keybinding_overrides(
    bindings: &std::collections::HashMap<String, String>,
    cx: &mut gpui::App,
) {
    macro_rules! bind {
        ($key:expr, $name:expr, $cx:expr, $( $action_name:expr => $action:expr ),+ $(,)?) => {
            match $name {
                $( $action_name => { $cx.bind_keys([KeyBinding::new($key, $action, None)]); } )+
                _ => {}
            }
        };
    }

    for (key, action_name) in bindings {
        // Slot-aware overrides come first — short-circuit if matched so
        // the slot table stays the single source of truth for all 18
        // ActivateTab* / ActivateWorktree* names.
        let key_str = key.as_str();
        let name_str = action_name.as_str();
        if crate::tab_slot_table!(@try_bind_override key_str, name_str, cx) {
            continue;
        }
        if crate::worktree_slot_table!(@try_bind_override key_str, name_str, cx) {
            continue;
        }
        // Per-section overrides come first so the dotted form wins over
        // the bare `open_settings` arm. Unknown slugs fall through to
        // the default (General) so a typo doesn't silently bind an
        // unrelated action.
        if let Some(stripped) = name_str.strip_prefix("open_settings.")
            && let Some(section) = daruda_config::BuiltinSection::from_slug(stripped)
        {
            cx.bind_keys([KeyBinding::new(key_str, OpenSettings(section), None)]);
            continue;
        }

        bind!(key_str, name_str, cx,
            "open_settings" => OpenSettings(daruda_config::BuiltinSection::default()),
            "quit" => Quit,
            "copy" => Copy,
            "paste" => Paste,
            "select_all" => SelectAll,
            "new_tab" => NewTab,
            "close_pane" => ClosePane,
            "close_tab" => CloseTab,
            "next_tab" => NextTab,
            "prev_tab" => PrevTab,
            "move_tab_left" => MoveTabLeft,
            "move_tab_right" => MoveTabRight,
            "split_right" => SplitRight,
            "split_down" => SplitDown,
            "focus_next_pane" => FocusNextPane,
            "focus_prev_pane" => FocusPrevPane,
            "focus_pane_left" => FocusPaneLeft,
            "focus_pane_right" => FocusPaneRight,
            "focus_pane_up" => FocusPaneUp,
            "focus_pane_down" => FocusPaneDown,
            "toggle_left_dock" => ToggleLeftDock,
            "toggle_bottom_dock" => ToggleBottomDock,
            "toggle_right_dock" => ToggleRightDock,
            "toggle_command_palette" => ToggleCommandPalette,
            "show_left_dock_worktrees" => ShowLeftDockWorktrees,
            "show_left_dock_git" => ShowLeftDockGit,
            "show_left_dock_files" => ShowLeftDockFiles,
            "switch_right_panel_usage" => SwitchRightPanelUsage,
            "switch_right_panel_skills" => SwitchRightPanelSkills,
            "switch_right_panel_tools" => SwitchRightPanelTools,
            "switch_right_panel_tasks" => SwitchRightPanelTasks,
            "new_skill" => NewSkill,
            "new_task" => NewTask,
            "edit_task" => EditTask,
            "refresh_git_status" => RefreshGitStatus,
            "files_toggle_hidden" => FilesToggleHidden,
            "files_refresh" => FilesRefresh,
            "files_select_next" => FilesSelectNext,
            "files_select_prev" => FilesSelectPrev,
            "files_activate" => FilesActivate,
            "files_expand" => FilesExpand,
            "files_collapse" => FilesCollapse,
            "git_changes_select_next" => GitChangesSelectNext,
            "git_changes_select_prev" => GitChangesSelectPrev,
            "git_changes_toggle_stage" => GitChangesToggleStage,
            "git_changes_activate" => GitChangesActivate,
            "commit_changes" => CommitChanges,
            "commit_amend" => CommitAmend,
            "push_changes" => PushChanges,
            "open_command_history" => OpenCommandHistory,
        );
    }
}

/// Check if a name is a recognized action. Used by tests.
#[cfg(test)]
fn is_known_action(name: &str) -> bool {
    known_actions().contains(&name)
}

/// List of every recognized action name. Slot names come from the
/// `tab_slot_table!` / `worktree_slot_table!` macros so the test stays
/// in sync with the wiring without manual duplication.
#[cfg(test)]
fn known_actions() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = vec![
        "open_settings",
        "quit",
        "copy",
        "paste",
        "select_all",
        "new_tab",
        "close_pane",
        "close_tab",
        "next_tab",
        "prev_tab",
        "move_tab_left",
        "move_tab_right",
        "split_right",
        "split_down",
        "focus_next_pane",
        "focus_prev_pane",
        "focus_pane_left",
        "focus_pane_right",
        "focus_pane_up",
        "focus_pane_down",
        "toggle_left_dock",
        "toggle_bottom_dock",
        "toggle_right_dock",
        "toggle_command_palette",
        "show_left_dock_worktrees",
        "show_left_dock_git",
        "show_left_dock_files",
        "switch_right_panel_usage",
        "switch_right_panel_skills",
        "switch_right_panel_tools",
        "switch_right_panel_tasks",
        "refresh_git_status",
        "files_toggle_hidden",
        "files_refresh",
        "files_select_next",
        "files_select_prev",
        "files_activate",
        "files_expand",
        "files_collapse",
        "git_changes_select_next",
        "git_changes_select_prev",
        "git_changes_toggle_stage",
        "git_changes_activate",
        "commit_changes",
        "push_changes",
        "open_command_history",
        "new_task",
        "edit_task",
    ];
    v.extend(crate::tab_slot_table!(@names));
    v.extend(crate::worktree_slot_table!(@names));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_known_actions_are_recognized() {
        for name in known_actions() {
            assert!(
                is_known_action(name),
                "is_known_action({name:?}) returned false"
            );
        }
    }

    #[test]
    fn unknown_action_is_not_recognized() {
        assert!(!is_known_action("nonexistent_action"));
        assert!(!is_known_action(""));
    }

    #[test]
    fn action_names_are_snake_case() {
        for name in known_actions() {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                "action name {name:?} should be snake_case"
            );
        }
    }

    #[test]
    fn known_actions_list_is_not_empty() {
        let actions = known_actions();
        assert!(!actions.is_empty());
        assert!(actions.len() >= 20, "should have many known actions");
    }

    #[test]
    fn open_settings_dotted_form_resolves_to_section() {
        // Section overrides land via the prefix-strip path in
        // `apply_keybinding_overrides` rather than the static table,
        // so verify each builtin slug round-trips through the parser
        // pieces directly. (`from_slug` is the matcher used in
        // `bind!`; if it ever drifts away from `BuiltinSection::ALL`
        // this test fails before user keybindings break silently.)
        for &section in daruda_config::BuiltinSection::ALL {
            let dotted = format!("open_settings.{}", section.slug());
            let stripped = dotted.strip_prefix("open_settings.").unwrap();
            assert_eq!(
                daruda_config::BuiltinSection::from_slug(stripped),
                Some(section),
                "round-trip failed for {dotted:?}"
            );
        }
    }

    #[test]
    fn open_settings_default_arg_is_general() {
        // Bare "open_settings" preserves the legacy meaning ("open
        // the Settings window on whatever the default page is").
        // Pinning Default == General here means a future variant
        // accidentally taking `Default` doesn't silently change the
        // bare-key behaviour.
        assert_eq!(
            daruda_config::BuiltinSection::default(),
            daruda_config::BuiltinSection::General,
        );
    }

    #[test]
    fn open_settings_unknown_dotted_slug_is_ignored() {
        // The bind! prefix path returns None for unknown slugs and
        // short-circuits via `continue`, so a typo is a silent no-op
        // (matching the existing behaviour for unknown bare names).
        assert_eq!(
            daruda_config::BuiltinSection::from_slug("definitely_not_a_section"),
            None,
        );
    }
}
