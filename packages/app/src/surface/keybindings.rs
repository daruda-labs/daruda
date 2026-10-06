//! GPUI `KeyBinding` shortcut strings. Remap = edit one line here.
//!
//! Keep this file strictly declarative: `pub const` strings only, no
//! `KeyBinding::new` calls. `main.rs` composes them with the action
//! types so this module stays free of `gpui` imports and can be reused
//! by future docs / cheat-sheet tooling.
//!
//! `secondary-` is cmd on macOS and ctrl elsewhere, which covers most
//! chords. [`per_platform`] is for the few naming *both* modifiers.

/// The spelling this platform uses. Both stay in the source so a reader —
/// or a cheat sheet — sees what the other platform presses.
const fn per_platform(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        mac
    } else {
        other
    }
}

// ============================================================================
// App
// ============================================================================
pub const SHORTCUT_QUIT: &str = "secondary-q";
pub const SHORTCUT_SETTINGS: &str = "secondary-,";

// ============================================================================
// Edit
// ============================================================================
pub const SHORTCUT_SELECT_ALL: &str = "secondary-a";
pub const SHORTCUT_COPY: &str = "secondary-c";
pub const SHORTCUT_PASTE: &str = "secondary-v";
pub const SHORTCUT_FIND: &str = "secondary-f";
/// Find-again is `cmd-g` on macOS and F3 everywhere else — and off macOS
/// `secondary-shift-g` would be `ctrl-shift-g`, which is
/// [`SHORTCUT_TOGGLE_GIT_CHANGES_FOCUS`]. That one is registered globally
/// and is the only keyboard way into the panel, so this is the pair that
/// has to move.
pub const SHORTCUT_FIND_NEXT: &str = per_platform("cmd-g", "f3");
pub const SHORTCUT_FIND_PREV: &str = per_platform("cmd-shift-g", "shift-f3");
pub const SHORTCUT_CLEAR_BUFFER: &str = "secondary-k";
pub const SHORTCUT_CLEAR_SCROLLBACK: &str = "secondary-shift-k";

// ============================================================================
// Window
// ============================================================================
pub const SHORTCUT_MINIMIZE: &str = "secondary-m";
pub const SHORTCUT_TOGGLE_FULL_SCREEN: &str = per_platform("ctrl-cmd-f", "f11");

// ============================================================================
// Help
// ============================================================================
pub const SHORTCUT_KEYBOARD_SHORTCUTS: &str = "secondary-/";
/// macOS HIG Help menu standard (`⌘?`). Aliased to the same action as
/// [`SHORTCUT_KEYBOARD_SHORTCUTS`] so users coming from native Mac apps
/// hit the command palette via the conventional Help shortcut.
pub const SHORTCUT_KEYBOARD_SHORTCUTS_ALT: &str = "secondary-shift-/";

// ============================================================================
// View — prompt navigation
// ============================================================================
pub const SHORTCUT_JUMP_PROMPT_PREV: &str = "secondary-shift-up";
pub const SHORTCUT_JUMP_PROMPT_NEXT: &str = "secondary-shift-down";

// ============================================================================
// Tabs
// ============================================================================
pub const SHORTCUT_NEW_TAB: &str = "secondary-t";
pub const SHORTCUT_CLOSE_PANE: &str = "secondary-w";
pub const SHORTCUT_NEXT_TAB: &str = "ctrl-tab";
pub const SHORTCUT_PREV_TAB: &str = "ctrl-shift-tab";

pub const SHORTCUT_ACTIVATE_TAB_1: &str = "secondary-1";
pub const SHORTCUT_ACTIVATE_TAB_2: &str = "secondary-2";
pub const SHORTCUT_ACTIVATE_TAB_3: &str = "secondary-3";
pub const SHORTCUT_ACTIVATE_TAB_4: &str = "secondary-4";
pub const SHORTCUT_ACTIVATE_TAB_5: &str = "secondary-5";
pub const SHORTCUT_ACTIVATE_TAB_6: &str = "secondary-6";
pub const SHORTCUT_ACTIVATE_TAB_7: &str = "secondary-7";
pub const SHORTCUT_ACTIVATE_TAB_8: &str = "secondary-8";
pub const SHORTCUT_ACTIVATE_TAB_9: &str = "secondary-9";

pub const SHORTCUT_MOVE_TAB_LEFT: &str = per_platform("cmd-shift-ctrl-left", "ctrl-shift-pageup");
pub const SHORTCUT_MOVE_TAB_RIGHT: &str =
    per_platform("cmd-shift-ctrl-right", "ctrl-shift-pagedown");

// ============================================================================
// Splits
// ============================================================================
pub const SHORTCUT_SPLIT_RIGHT: &str = "secondary-d";
pub const SHORTCUT_SPLIT_DOWN: &str = "secondary-shift-d";

pub const SHORTCUT_FOCUS_NEXT_PANE: &str = "secondary-]";
pub const SHORTCUT_FOCUS_PREV_PANE: &str = "secondary-[";

// Directional pane focus — iTerm2 parity (cmd-alt-arrow).
pub const SHORTCUT_FOCUS_PANE_LEFT: &str = "secondary-alt-left";
pub const SHORTCUT_FOCUS_PANE_RIGHT: &str = "secondary-alt-right";
pub const SHORTCUT_FOCUS_PANE_UP: &str = "secondary-alt-up";
pub const SHORTCUT_FOCUS_PANE_DOWN: &str = "secondary-alt-down";

// ============================================================================
// Docks
// ============================================================================
pub const SHORTCUT_TOGGLE_LEFT_DOCK: &str = "secondary-b";
/// Jump into the left dock's Git Changes panel — and back out on a second
/// press. Matches zed's `git_panel::ToggleFocus`.
pub const SHORTCUT_TOGGLE_GIT_CHANGES_FOCUS: &str = "ctrl-shift-g";
/// Same door for the Files panel. Matches zed's `project_panel::ToggleFocus`.
pub const SHORTCUT_TOGGLE_FILES_FOCUS: &str = "secondary-shift-e";
pub const SHORTCUT_TOGGLE_BOTTOM_DOCK: &str = "secondary-j";
pub const SHORTCUT_TOGGLE_RIGHT_DOCK: &str = "secondary-shift-b";

// ============================================================================
// Command palette
// ============================================================================
pub const SHORTCUT_COMMAND_PALETTE: &str = "secondary-shift-p";

/// Lane switcher — fuzzy quick-switch across every project's lanes.
/// Pairs with the command palette one modifier away, as VS Code's
/// quick-open does.
pub const SHORTCUT_LANE_SWITCHER: &str = "secondary-p";

// ============================================================================
// File viewer
// ============================================================================
pub const SHORTCUT_SAVE_FILE_PANE: &str = "secondary-s";
pub const SHORTCUT_FILE_VIEWER_SEARCH_OPEN: &str = "secondary-f";
pub const SHORTCUT_FILE_VIEWER_SEARCH_NEXT: &str = "enter";
pub const SHORTCUT_FILE_VIEWER_SEARCH_PREV: &str = "shift-enter";

// ============================================================================
// Project
// ============================================================================
pub const SHORTCUT_OPEN_FOLDER: &str = "secondary-o";
pub const SHORTCUT_OPEN_FOLDER_IN_NEW_WINDOW: &str = "secondary-shift-o";
pub const SHORTCUT_NEW_WINDOW: &str = "secondary-n";
pub const SHORTCUT_CLOSE_PROJECT: &str = "secondary-shift-w";
pub const SHORTCUT_NEW_GROUP: &str = "secondary-shift-n";
pub const SHORTCUT_RENAME_PROJECT: &str = "secondary-shift-r";
pub const SHORTCUT_MOVE_PROJECT_TO_GROUP: &str = "secondary-shift-m";

// ============================================================================
// Lanes
// ============================================================================
pub const SHORTCUT_ACTIVATE_LANE_1: &str = per_platform("cmd-ctrl-1", "ctrl-alt-1");
pub const SHORTCUT_ACTIVATE_LANE_2: &str = per_platform("cmd-ctrl-2", "ctrl-alt-2");
pub const SHORTCUT_ACTIVATE_LANE_3: &str = per_platform("cmd-ctrl-3", "ctrl-alt-3");
pub const SHORTCUT_ACTIVATE_LANE_4: &str = per_platform("cmd-ctrl-4", "ctrl-alt-4");
pub const SHORTCUT_ACTIVATE_LANE_5: &str = per_platform("cmd-ctrl-5", "ctrl-alt-5");
pub const SHORTCUT_ACTIVATE_LANE_6: &str = per_platform("cmd-ctrl-6", "ctrl-alt-6");
pub const SHORTCUT_ACTIVATE_LANE_7: &str = per_platform("cmd-ctrl-7", "ctrl-alt-7");
pub const SHORTCUT_ACTIVATE_LANE_8: &str = per_platform("cmd-ctrl-8", "ctrl-alt-8");
pub const SHORTCUT_ACTIVATE_LANE_9: &str = per_platform("cmd-ctrl-9", "ctrl-alt-9");

// ============================================================================
// Files view
// ============================================================================
pub const SHORTCUT_FILES_TOGGLE_HIDDEN: &str = "secondary-shift-.";
pub const SHORTCUT_FILES_SELECT_NEXT: &str = "down";
pub const SHORTCUT_FILES_SELECT_PREV: &str = "up";
pub const SHORTCUT_FILES_EXPAND: &str = "right";
pub const SHORTCUT_FILES_COLLAPSE: &str = "left";
pub const SHORTCUT_FILES_ACTIVATE: &str = "enter";
pub const SHORTCUT_FILES_REFRESH: &str = "secondary-r";

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
/// Opens the picker listing every completed command captured by the
/// FTCS B/C marks. Mirrors iTerm2's "Toolbelt > Commands" entry-point.
pub const SHORTCUT_OPEN_COMMAND_HISTORY: &str = "secondary-shift-h";

// ============================================================================
// Right panel — Skills
// ============================================================================
//
// `NewSkill` has no default keybinding. It is discoverable through the
// command palette (`Skills: New skill`) and through the keybinding
// override system in `surface::action_map`, so users who want a
// shortcut can bind it themselves without daruda picking a chord that
// might collide with their existing keymap. If a default chord is ever
// added, follow the `SHORTCUT_*` convention (`pub const … = "secondary-…"`)
// and register it in `main.rs`.

/// Focus the right-bar Skills search input. Modified rather than a bare
/// `/`, which would block the same key inside the terminal; matches the
/// "open quick search" convention from VS Code / Sublime.
pub const SHORTCUT_FOCUS_SKILL_SEARCH: &str = "secondary-/";

/// Open the global skill palette — pick any skill from any scope and
/// invoke it. Mirrors VS Code's palette chord pattern (modifier + capital
/// letter) without colliding with the Command Palette itself.
pub const SHORTCUT_INVOKE_SKILL_PALETTE: &str = "secondary-shift-s";

// ============================================================================
// Status bar — account switcher
// ============================================================================
//
// `SwitchPaneAccount` has no keybinding const (and no `action_map`/command
// palette entry — an intentional exception to the usual G3 chain). Unlike
// `NewSkill` (a plain marker action a user *could* bind to any key),
// `SwitchPaneAccount` carries a concrete `AccountId` chosen from the
// status-bar dropdown's live account list; there is no "the" account to
// give it as a static default, so a keymap.json binding or a static
// palette entry would have nothing meaningful to target. It is dispatched
// only from the status-bar account dropdown
// (`workspace::status_bar::build_account_menu`), one item per managed
// account.
