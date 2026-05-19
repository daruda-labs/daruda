//! GPUI `KeyBinding` shortcut strings. Remap = edit one line here.
//!
//! Keep this file strictly declarative: `pub const` strings only, no
//! `KeyBinding::new` calls. `main.rs` composes them with the action
//! types so this module stays free of `gpui` imports and can be reused
//! by future docs / cheat-sheet tooling.

// ============================================================================
// App
// ============================================================================
pub const SHORTCUT_QUIT: &str = "cmd-q";
pub const SHORTCUT_SETTINGS: &str = "cmd-,";

// ============================================================================
// Edit
// ============================================================================
pub const SHORTCUT_SELECT_ALL: &str = "cmd-a";
pub const SHORTCUT_COPY: &str = "cmd-c";
pub const SHORTCUT_PASTE: &str = "cmd-v";
pub const SHORTCUT_FIND: &str = "cmd-f";
pub const SHORTCUT_FIND_NEXT: &str = "cmd-g";
pub const SHORTCUT_FIND_PREV: &str = "cmd-shift-g";
pub const SHORTCUT_CLEAR_BUFFER: &str = "cmd-k";
pub const SHORTCUT_CLEAR_SCROLLBACK: &str = "cmd-shift-k";

// ============================================================================
// Window
// ============================================================================
pub const SHORTCUT_MINIMIZE: &str = "cmd-m";
pub const SHORTCUT_TOGGLE_FULL_SCREEN: &str = "ctrl-cmd-f";

// ============================================================================
// Help
// ============================================================================
pub const SHORTCUT_KEYBOARD_SHORTCUTS: &str = "cmd-/";
/// macOS HIG Help menu standard (`⌘?`). Aliased to the same action as
/// [`SHORTCUT_KEYBOARD_SHORTCUTS`] so users coming from native Mac apps
/// hit the command palette via the conventional Help shortcut.
pub const SHORTCUT_KEYBOARD_SHORTCUTS_ALT: &str = "cmd-shift-/";

// ============================================================================
// View — prompt navigation
// ============================================================================
pub const SHORTCUT_JUMP_PROMPT_PREV: &str = "cmd-shift-up";
pub const SHORTCUT_JUMP_PROMPT_NEXT: &str = "cmd-shift-down";

// ============================================================================
// Tabs
// ============================================================================
pub const SHORTCUT_NEW_TAB: &str = "cmd-t";
pub const SHORTCUT_CLOSE_PANE: &str = "cmd-w";
pub const SHORTCUT_NEXT_TAB: &str = "ctrl-tab";
pub const SHORTCUT_PREV_TAB: &str = "ctrl-shift-tab";

pub const SHORTCUT_ACTIVATE_TAB_1: &str = "cmd-1";
pub const SHORTCUT_ACTIVATE_TAB_2: &str = "cmd-2";
pub const SHORTCUT_ACTIVATE_TAB_3: &str = "cmd-3";
pub const SHORTCUT_ACTIVATE_TAB_4: &str = "cmd-4";
pub const SHORTCUT_ACTIVATE_TAB_5: &str = "cmd-5";
pub const SHORTCUT_ACTIVATE_TAB_6: &str = "cmd-6";
pub const SHORTCUT_ACTIVATE_TAB_7: &str = "cmd-7";
pub const SHORTCUT_ACTIVATE_TAB_8: &str = "cmd-8";
pub const SHORTCUT_ACTIVATE_TAB_9: &str = "cmd-9";

pub const SHORTCUT_MOVE_TAB_LEFT: &str = "cmd-shift-ctrl-left";
pub const SHORTCUT_MOVE_TAB_RIGHT: &str = "cmd-shift-ctrl-right";

// ============================================================================
// Splits
// ============================================================================
pub const SHORTCUT_SPLIT_RIGHT: &str = "cmd-d";
pub const SHORTCUT_SPLIT_DOWN: &str = "cmd-shift-d";

pub const SHORTCUT_FOCUS_NEXT_PANE: &str = "cmd-]";
pub const SHORTCUT_FOCUS_PREV_PANE: &str = "cmd-[";

// Directional pane focus — iTerm2 parity (cmd-alt-arrow).
pub const SHORTCUT_FOCUS_PANE_LEFT: &str = "cmd-alt-left";
pub const SHORTCUT_FOCUS_PANE_RIGHT: &str = "cmd-alt-right";
pub const SHORTCUT_FOCUS_PANE_UP: &str = "cmd-alt-up";
pub const SHORTCUT_FOCUS_PANE_DOWN: &str = "cmd-alt-down";

// ============================================================================
// Docks
// ============================================================================
pub const SHORTCUT_TOGGLE_LEFT_DOCK: &str = "cmd-b";
pub const SHORTCUT_TOGGLE_BOTTOM_DOCK: &str = "cmd-j";
pub const SHORTCUT_TOGGLE_RIGHT_DOCK: &str = "cmd-shift-b";

// ============================================================================
// Command palette
// ============================================================================
pub const SHORTCUT_COMMAND_PALETTE: &str = "cmd-shift-p";

// ============================================================================
// File viewer search
// ============================================================================
pub const SHORTCUT_FILE_VIEWER_SEARCH_OPEN: &str = "cmd-f";
pub const SHORTCUT_FILE_VIEWER_SEARCH_NEXT: &str = "enter";
pub const SHORTCUT_FILE_VIEWER_SEARCH_PREV: &str = "shift-enter";

// ============================================================================
// Project
// ============================================================================
pub const SHORTCUT_OPEN_FOLDER: &str = "cmd-o";
pub const SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW: &str = "cmd-shift-o";
pub const SHORTCUT_NEW_WINDOW: &str = "cmd-n";
pub const SHORTCUT_CLOSE_PROJECT: &str = "cmd-shift-w";
pub const SHORTCUT_NEW_GROUP: &str = "cmd-shift-n";
pub const SHORTCUT_RENAME_PROJECT: &str = "cmd-shift-r";
pub const SHORTCUT_MOVE_PROJECT_TO_GROUP: &str = "cmd-shift-m";

// ============================================================================
// Worktrees
// ============================================================================
pub const SHORTCUT_ACTIVATE_WORKTREE_1: &str = "cmd-ctrl-1";
pub const SHORTCUT_ACTIVATE_WORKTREE_2: &str = "cmd-ctrl-2";
pub const SHORTCUT_ACTIVATE_WORKTREE_3: &str = "cmd-ctrl-3";
pub const SHORTCUT_ACTIVATE_WORKTREE_4: &str = "cmd-ctrl-4";
pub const SHORTCUT_ACTIVATE_WORKTREE_5: &str = "cmd-ctrl-5";
pub const SHORTCUT_ACTIVATE_WORKTREE_6: &str = "cmd-ctrl-6";
pub const SHORTCUT_ACTIVATE_WORKTREE_7: &str = "cmd-ctrl-7";
pub const SHORTCUT_ACTIVATE_WORKTREE_8: &str = "cmd-ctrl-8";
pub const SHORTCUT_ACTIVATE_WORKTREE_9: &str = "cmd-ctrl-9";

// ============================================================================
// Files view
// ============================================================================
pub const SHORTCUT_FILES_TOGGLE_HIDDEN: &str = "cmd-shift-.";
pub const SHORTCUT_FILES_SELECT_NEXT: &str = "down";
pub const SHORTCUT_FILES_SELECT_PREV: &str = "up";
pub const SHORTCUT_FILES_EXPAND: &str = "right";
pub const SHORTCUT_FILES_COLLAPSE: &str = "left";
pub const SHORTCUT_FILES_ACTIVATE: &str = "enter";
pub const SHORTCUT_FILES_REFRESH: &str = "cmd-r";

// ============================================================================
// Git Changes view
// ============================================================================
pub const SHORTCUT_GIT_CHANGES_SELECT_NEXT: &str = "down";
pub const SHORTCUT_GIT_CHANGES_SELECT_PREV: &str = "up";
/// Space toggles the staged state of the row under the keyboard cursor.
pub const SHORTCUT_GIT_CHANGES_TOGGLE_STAGE: &str = "space";
/// Enter opens the diff viewer for the row under the keyboard cursor.
pub const SHORTCUT_GIT_CHANGES_ACTIVATE: &str = "enter";

// ============================================================================
// Command history picker
// ============================================================================
/// `Cmd+Shift+H` — opens the picker that lists every completed
/// command captured by the FTCS B/C marks. Mirrors iTerm2's
/// "Toolbelt > Commands" entry-point.
pub const SHORTCUT_OPEN_COMMAND_HISTORY: &str = "cmd-shift-h";

// ============================================================================
// Right panel — Skills
// ============================================================================
//
// `NewSkill` has no default keybinding. It is discoverable through the
// command palette (`Skills: New skill`) and through the keybinding
// override system in `surface::action_map`, so users who want a
// shortcut can bind it themselves without daruda picking a chord that
// might collide with their existing keymap. If a default chord is ever
// added, follow the `SHORTCUT_*` convention (`pub const … = "cmd-…"`)
// and register it in `main.rs`.

/// Focus the right-bar Skills search input. `cmd-/` avoids the bare
/// `/` which would block the same key inside the terminal, and matches
/// the "open quick search" convention from VS Code / Sublime.
pub const SHORTCUT_FOCUS_SKILL_SEARCH: &str = "cmd-/";

/// Open the global skill palette — pick any skill from any scope and
/// invoke it. `cmd-shift-s` mirrors VS Code's `cmd-shift-p` palette
/// chord pattern (modifier + capital letter) without colliding with
/// the existing `cmd-shift-p` Command Palette.
pub const SHORTCUT_INVOKE_SKILL_PALETTE: &str = "cmd-shift-s";
