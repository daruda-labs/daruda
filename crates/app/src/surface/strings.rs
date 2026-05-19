//! Menu labels and any other app-shell display text.
//!
//! Localisation hook-point for the menu bar. Terminal-view text
//! (search overlay, fallback title) is in
//! `daruda_terminal::ux::strings`; this file is only for the app's
//! chrome.

use super::constants::APP_NAME;

// ============================================================================
// Top-level menu names
// ============================================================================

pub const MENU_APP: &str = APP_NAME;
pub const MENU_FILE: &str = "File";
pub const MENU_VIEW: &str = "View";
pub const MENU_EDIT: &str = "Edit";
pub const MENU_WORKTREE: &str = "Worktree";
pub const MENU_WINDOW: &str = "Window";
pub const MENU_HELP: &str = "Help";

// ============================================================================
// App menu
// ============================================================================

pub const MENU_QUIT_APP: &str = "Quit Daruda";
pub const MENU_SERVICES: &str = "Services";

// ============================================================================
// File menu
// ============================================================================

pub const MENU_NEW_WINDOW: &str = "New Window";
pub const MENU_OPEN: &str = "Open…";
pub const MENU_OPEN_IN_NEW_WINDOW: &str = "Open in New Window…";
pub const MENU_OPEN_RECENT: &str = "Open Recent";
pub const MENU_OPEN_RECENT_IN_NEW_WINDOW: &str = "Open Recent in New Window";
pub const MENU_CLOSE_PROJECT: &str = "Close Project";
pub const MENU_NO_RECENT: &str = "No recent projects";
pub const MENU_NEW_TAB: &str = "New Tab";
pub const MENU_CLOSE_PANE: &str = "Close Pane";
pub const MENU_CLOSE_TAB: &str = "Close Tab";

// ============================================================================
// View menu
// ============================================================================

pub const MENU_SPLIT_RIGHT: &str = "Split Right";
pub const MENU_SPLIT_DOWN: &str = "Split Down";
pub const MENU_NEXT_PANE: &str = "Next Pane";
pub const MENU_PREV_PANE: &str = "Previous Pane";
pub const MENU_FOCUS_PANE_LEFT: &str = "Focus Pane Left";
pub const MENU_FOCUS_PANE_RIGHT: &str = "Focus Pane Right";
pub const MENU_FOCUS_PANE_UP: &str = "Focus Pane Up";
pub const MENU_FOCUS_PANE_DOWN: &str = "Focus Pane Down";
pub const MENU_MOVE_TAB_LEFT: &str = "Move Tab Left";
pub const MENU_MOVE_TAB_RIGHT: &str = "Move Tab Right";

// ============================================================================
// Edit menu
// ============================================================================

pub const MENU_COPY: &str = "Copy";
pub const MENU_PASTE: &str = "Paste";
pub const MENU_SELECT_ALL: &str = "Select All";
pub const MENU_FIND: &str = "Find\u{2026}";
pub const MENU_FIND_NEXT: &str = "Find Next";
pub const MENU_FIND_PREV: &str = "Find Previous";
pub const MENU_CLEAR_BUFFER: &str = "Clear Buffer";
pub const MENU_CLEAR_SCROLLBACK: &str = "Clear Scrollback";

// ============================================================================
// View menu (additions)
// ============================================================================

pub const MENU_TOGGLE_FULL_SCREEN: &str = "Toggle Full Screen";
pub const MENU_TOGGLE_LEFT_DOCK: &str = "Toggle Left Dock";
pub const MENU_TOGGLE_BOTTOM_DOCK: &str = "Toggle Bottom Dock";
pub const MENU_TOGGLE_RIGHT_DOCK: &str = "Toggle Right Dock";
pub const MENU_JUMP_PROMPT_PREV: &str = "Jump to Previous Prompt";
pub const MENU_JUMP_PROMPT_NEXT: &str = "Jump to Next Prompt";

// ============================================================================
// Worktree menu
// ============================================================================

pub const MENU_ACTIVATE_WORKTREE_1: &str = "Activate Worktree 1";
pub const MENU_ACTIVATE_WORKTREE_2: &str = "Activate Worktree 2";
pub const MENU_ACTIVATE_WORKTREE_3: &str = "Activate Worktree 3";
pub const MENU_ACTIVATE_WORKTREE_4: &str = "Activate Worktree 4";
pub const MENU_ACTIVATE_WORKTREE_5: &str = "Activate Worktree 5";
pub const MENU_ACTIVATE_WORKTREE_6: &str = "Activate Worktree 6";
pub const MENU_ACTIVATE_WORKTREE_7: &str = "Activate Worktree 7";
pub const MENU_ACTIVATE_WORKTREE_8: &str = "Activate Worktree 8";
pub const MENU_ACTIVATE_WORKTREE_9: &str = "Activate Worktree 9";

// ============================================================================
// Window menu
// ============================================================================

pub const MENU_MINIMIZE: &str = "Minimize";
pub const MENU_ZOOM: &str = "Zoom";
pub const MENU_EDIT_WINDOW_TITLE: &str = "Edit Window Title\u{2026}";

pub const EDIT_WINDOW_TITLE_MODAL_TITLE: &str = "Edit Window Title";
pub const EDIT_WINDOW_TITLE_PLACEHOLDER: &str = "Window title (e.g. daruda — review window)";

// ============================================================================
// Help menu
// ============================================================================

pub const MENU_DARUDA_HELP: &str = "Daruda Help";
pub const MENU_KEYBOARD_SHORTCUTS: &str = "Keyboard Shortcuts";
pub const MENU_EDIT_KEYMAP: &str = "Edit Keymap\u{2026}";
pub const MENU_REPORT_ISSUE: &str = "Report Issue\u{2026}";
pub const MENU_GITHUB_REPO: &str = "GitHub Repository";

// ============================================================================
// External URLs (Help menu targets)
// ============================================================================

pub const URL_GITHUB_REPO: &str = "https://github.com/daruda-ai/daruda";
pub const URL_REPORT_ISSUE: &str = "https://github.com/daruda-ai/daruda/issues/new";
pub const URL_HELP: &str = "https://github.com/daruda-ai/daruda#readme";

// ============================================================================
// Dock panel labels
// ============================================================================

pub const DOCK_PANEL_AGENT_TASKS: &str = "Agent Tasks";
pub const DOCK_PANEL_FILES: &str = "Files";
pub const DOCK_PANEL_GIT: &str = "Git";
pub const DOCK_PANEL_MACROS: &str = "Macros";
pub const DOCK_PANEL_OUTPUT: &str = "Output";
pub const DOCK_PANEL_WORKTREES: &str = "Worktrees";

// ============================================================================
// Right panel tab labels
// ============================================================================

pub const RIGHT_PANEL_TAB_USAGE: &str = "Usage";
pub const RIGHT_PANEL_TAB_SKILLS: &str = "Skills";
pub const RIGHT_PANEL_TAB_TOOLS: &str = "Tools";
pub const RIGHT_PANEL_TAB_TASKS: &str = "Tasks";

// ============================================================================
// Dock (left dock) tab labels
// ============================================================================

pub const SIDEBAR_TAB_WORKTREES: &str = "Worktrees";
pub const SIDEBAR_TAB_GIT: &str = "Git";
pub const SIDEBAR_TAB_FILES: &str = "Files";

// ============================================================================
// Right panel — Tasks tab labels (R-11 ~ R-18)
// ============================================================================

/// Filter dropdown labels.
pub const TASK_FILTER_ALL: &str = "All";
pub const TASK_FILTER_BACKLOG: &str = "Backlog";
pub const TASK_FILTER_RUNNING: &str = "Running";
pub const TASK_FILTER_DONE: &str = "Done";

/// `[+ New]` button label.
pub const TASK_NEW_BUTTON: &str = "+ New";

/// Tasks tab search bar — substring filter over title / prompt /
/// notes / branch_name. Placement and behaviour mirror the Skills tab
/// search (`SKILLS_SEARCH_*`). Cleared via the in-field `✕` overlay.
pub const TASK_SEARCH_PLACEHOLDER: &str = "Search tasks…";
pub const TASK_SEARCH_EMPTY_PREFIX: &str = "No tasks match ";
pub const TASK_SEARCH_CLEAR_ICON: &str = "✕";

/// Action labels rendered inside the status-pill dropdown. The pill
/// itself shows the task's current state; the dropdown lists the
/// transitions and meta actions valid for that state (R-26).
pub const TASK_ACTION_START: &str = "Start";
pub const TASK_ACTION_OPEN: &str = "Open worktree";
pub const TASK_ACTION_STOP: &str = "Stop";
pub const TASK_ACTION_DELETE: &str = "Delete";
pub const TASK_ACTION_REOPEN: &str = "Reopen";
pub const TASK_ACTION_RETRY: &str = "Retry";
pub const TASK_ACTION_EDIT: &str = "Edit";
/// `View error` (R-26 Error state). Opens an OK-only alert dialog
/// showing the full `TaskState::Error.message` so users can see the
/// truncated row text in full.
pub const TASK_ACTION_VIEW_ERROR: &str = "View error";
/// Title shown on the alert dialog opened by `View error`.
pub const TASK_ERROR_DIALOG_TITLE: &str = "Task error";
/// OK-button label on the View error alert dialog.
pub const TASK_ERROR_DIALOG_CLOSE: &str = "Close";

/// `[📄 Open file]` button shown next to the Prompt section header in
/// the TaskEdit pane (R-20 follow-up). Click opens
/// `<wt>/.daruda/task-<branch>.md` in a fresh file viewer tab when
/// the task has a worktree (Backlog / draft tasks disable the button
/// since no on-disk file exists yet).
pub const TASK_EDIT_OPEN_FILE_BUTTON: &str = "📄 Open file";

/// Subtext rendered directly under the "Notes" label in the TaskEdit
/// pane. Notes are stored on the `Task` and surface in search, but
/// `render_task_prompt` deliberately excludes them — the hint makes
/// that contract visible so users don't expect the agent to read
/// their journal. Kept agent-agnostic since `AgentType` is reserved
/// for future codex / gemini / copilot expansion.
pub const TASK_EDIT_NOTES_HINT: &str = "Not included in the agent prompt.";

/// Field label for the base-worktree selector on the TaskEdit pane.
pub const TASK_EDIT_BASE_LABEL: &str = "Base";

/// Trailing suffix on the option that maps to "no explicit base
/// worktree — fall through to the project's active worktree at
/// start_task time". Used as both the placeholder and the
/// first-option label so empty drafts read as "Active worktree"
/// rather than blank.
pub const TASK_EDIT_BASE_ACTIVE_LABEL: &str = "Active worktree";

/// Glyph appended to the status pill label as the dropdown chevron.
/// Leading space provides the visual gap between the label and the
/// triangle.
pub const TASK_PILL_CHEVRON: &str = " ▾";

/// TaskEdit pane close prompt — copy mirrors Zed `pane.rs:1981-1998`
/// (R-25 / I-8). Draft branch uses a stronger "Discard new task?"
/// heading since the work has never been persisted.
pub const TASK_EDIT_SAVE_PROMPT_PREFIX: &str = "Save changes to “";
pub const TASK_EDIT_SAVE_PROMPT_SUFFIX: &str = "” before closing?";
pub const TASK_EDIT_DISCARD_DRAFT_PROMPT: &str = "Discard new task?";
pub const TASK_EDIT_SAVE: &str = "Save";
pub const TASK_EDIT_SAVE_DRAFT: &str = "Save Draft";
pub const TASK_EDIT_DISCARD: &str = "Discard";
pub const TASK_EDIT_CANCEL: &str = "Cancel";

/// Tab-title dirty indicator (R-25). Painted before the title with a
/// trailing space so titles align across dirty / clean tabs.
pub const TAB_TITLE_DIRTY_DOT: &str = "● ";

/// Prompt-file watcher conflict prompt (R-20 / I-13). Fires when an
/// external editor rewrites `<wt>/.daruda/task-<branch>.md` and the
/// pane already has unsaved edits.
pub const PROMPT_WATCHER_HEADING_PREFIX: &str = "“";
pub const PROMPT_WATCHER_HEADING_SUFFIX: &str = "” changed on disk";
pub const PROMPT_WATCHER_DETAIL: &str = "Reload the file or keep your in-pane edits?";
pub const PROMPT_WATCHER_USE_DISK: &str = "Use disk version";
pub const PROMPT_WATCHER_KEEP_MINE: &str = "Keep my version";
pub const PROMPT_WATCHER_DIFF: &str = "Diff";

/// Tab / window batch close prompt (R-25). Single 3-button modal
/// summarising every dirty TaskEdit pane in the closing scope.
pub const TAB_CLOSE_BATCH_HEADING: &str = "Save changes to the following tasks before closing?";
pub const TAB_CLOSE_BATCH_SAVE_ALL: &str = "Save all";
pub const TAB_CLOSE_BATCH_DISCARD_ALL: &str = "Discard all";

/// Title for the toast surfaced when one or more panes in a Save-all
/// batch fail to commit because their branch input is invalid. The
/// detail message lists the affected task titles (M-2 review note).
pub const TASK_BATCH_SAVE_FAILED_TITLE: &str = "Some tasks couldn't be saved";

/// Done end-reason flavour appended inline as `Done (<flavour>)`.
pub const TASK_DONE_FLAVOUR_STOP: &str = "Stop";
pub const TASK_DONE_FLAVOUR_PROMPT_INPUT_EXIT: &str = "Prompt Exit";
pub const TASK_DONE_FLAVOUR_LOGOUT: &str = "Logout";
pub const TASK_DONE_FLAVOUR_OTHER: &str = "Other";

/// Subtask UI strings (R-21). The section title is suffixed with the
/// `(done/total done)` counter at render time; the auto/manual labels
/// surface the `source_session_id` namespace split (I-14).
pub const TASK_SUBTASK_SECTION_TITLE: &str = "Subtasks";
pub const TASK_SUBTASK_PROGRESS_SUFFIX: &str = " done";
pub const TASK_SUBTASK_ADD_PLACEHOLDER: &str = "Add subtask…";
pub const TASK_SUBTASK_DRAFT_HINT: &str = "Save the task to add subtasks.";
pub const TASK_SUBTASK_AUTO_LABEL: &str = "auto";
pub const TASK_SUBTASK_MANUAL_LABEL: &str = "manual";

// ============================================================================
// Right panel — Usage tab labels
// ============================================================================

/// Heading shown above the per-session list — the row that aggregates
/// every active session's tokens + estimated cost.
pub const USAGE_TOTAL_LABEL: &str = "Total";

/// Inline label for inbound (user → Claude) tokens.
pub const USAGE_IN_LABEL: &str = "in";

/// Inline label for outbound (Claude → user) tokens.
pub const USAGE_OUT_LABEL: &str = "out";

/// Inline label for prompt-cache tokens (read + creation combined).
pub const USAGE_CACHE_LABEL: &str = "cache";

/// Body shown when no Claude Code session has produced any usage
/// data yet. Intentionally instructional rather than a bare "no
/// data" — first-launch users may not realise daruda needs an
/// active Claude session to populate this view.
pub const USAGE_EMPTY_STATE: &str = "Waiting for Claude Code activity in any worktree.";

/// Fallback worktree label when a session's `worktree_path` has no
/// resolvable file-name component (e.g. root path).
pub const USAGE_UNKNOWN_WORKTREE: &str = "?";

/// Time-window dropdown labels. The data-layer
/// (`daruda_store::project::UsageWindow`) is intentionally free of
/// user-facing strings; lookups go through [`usage_window_label`]
/// so a single source of truth for the picker's option list lives
/// here in the surface module.
pub const USAGE_WINDOW_LIFETIME: &str = "Lifetime";
pub const USAGE_WINDOW_LAST_5H: &str = "Last 5h";
pub const USAGE_WINDOW_LAST_24H: &str = "Last 24h";
pub const USAGE_WINDOW_LAST_7D: &str = "Last 7d";

/// Resolve a `UsageWindow` variant to its dropdown / summary label.
pub fn usage_window_label(window: daruda_store::project::UsageWindow) -> &'static str {
    use daruda_store::project::UsageWindow;
    match window {
        UsageWindow::All => USAGE_WINDOW_LIFETIME,
        UsageWindow::Last5h => USAGE_WINDOW_LAST_5H,
        UsageWindow::Last24h => USAGE_WINDOW_LAST_24H,
        UsageWindow::Last7d => USAGE_WINDOW_LAST_7D,
    }
}

// ----------------------------------------------------------------
// Plan-limit gauges (R-4)
// ----------------------------------------------------------------

/// Label for the 5-hour rolling window gauge.
pub const USAGE_LIMIT_5H_LABEL: &str = "5h";
/// Label for the 7-day rolling window gauge.
pub const USAGE_LIMIT_7D_LABEL: &str = "7d";
/// Placeholder label for either gauge when the OAuth token is
/// unavailable, the API call failed, or the window is missing from
/// the response.
pub const USAGE_LIMIT_UNAVAILABLE: &str = "Unavailable";

/// Format a "resets in …" countdown for the gauge subtitle. Lives
/// here (not on `Duration`) so the precise rounding behaviour is
/// covered by tests next to the rest of the surface strings.
///
/// Buckets:
/// - `≥ 1 day` → `"<d>d <h>h"`
/// - `≥ 1 hour` → `"<h>h <m>m"`
/// - `≥ 1 minute` → `"<m>m"`
/// - `1..60 seconds` → `"<1m"` (avoids the awkward "Resets in 0m"
///   that would linger for a whole minute before the reset)
/// - `0` → `"now"`
///
/// Always prefixed with `"Resets in "` so callers concatenate a
/// single string and don't have to reason about pluralization.
pub fn format_reset_countdown(remaining: std::time::Duration) -> String {
    let secs = remaining.as_secs();
    if secs == 0 {
        return "Resets now".to_string();
    }
    let mins_total = secs / 60;
    let hours_total = mins_total / 60;
    let days_total = hours_total / 24;
    if days_total >= 1 {
        format!("Resets in {}d {}h", days_total, hours_total % 24)
    } else if hours_total >= 1 {
        format!("Resets in {}h {}m", hours_total, mins_total % 60)
    } else if mins_total >= 1 {
        format!("Resets in {}m", mins_total)
    } else {
        "Resets in <1m".to_string()
    }
}

// ----------------------------------------------------------------
// Service status pill (R-5)
// ----------------------------------------------------------------

/// Label shown on the green pill when Anthropic Statuspage reports
/// "operational". The Übersicht widget unconditionally hides the
/// upstream description on the green path because Statuspage tends
/// to leave stale "All systems normal" descriptions; daruda mirrors
/// that behavior.
pub const STATUS_LABEL_OPERATIONAL: &str = "Operational";
/// Default label for `minor` indicator when the response carries no
/// description (rare but possible).
pub const STATUS_LABEL_MINOR_DEFAULT: &str = "Minor disruption";
/// Default label for `major` indicator when the response carries no
/// description.
pub const STATUS_LABEL_MAJOR_DEFAULT: &str = "Partial outage";
/// Default label for `critical` indicator when the response carries
/// no description.
pub const STATUS_LABEL_CRITICAL_DEFAULT: &str = "Major outage";
/// Label shown when the indicator is `Unknown` (parse miss or
/// before-first-fetch). Distinct from operational so the renderer
/// can dim the pill instead of pretending green.
pub const STATUS_LABEL_UNKNOWN: &str = "Status unavailable";

/// Resolve the user-visible label for a service-status snapshot.
///
/// Mirrors the Übersicht widget rule: `None` ignores the upstream
/// description (always show the hard-coded "Operational"), other
/// indicators prefer the upstream description and fall back to a
/// canned default when it's empty. `Unknown` ignores description
/// entirely — a parse miss shouldn't surface garbage strings.
pub fn service_status_label(status: &daruda_claude::ServiceStatus) -> String {
    use daruda_claude::StatusIndicator;
    match status.indicator {
        StatusIndicator::None => STATUS_LABEL_OPERATIONAL.to_string(),
        StatusIndicator::Unknown => STATUS_LABEL_UNKNOWN.to_string(),
        StatusIndicator::Minor => {
            if status.description.is_empty() {
                STATUS_LABEL_MINOR_DEFAULT.to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Major => {
            if status.description.is_empty() {
                STATUS_LABEL_MAJOR_DEFAULT.to_string()
            } else {
                status.description.clone()
            }
        }
        StatusIndicator::Critical => {
            if status.description.is_empty() {
                STATUS_LABEL_CRITICAL_DEFAULT.to_string()
            } else {
                status.description.clone()
            }
        }
    }
}

// ============================================================================
// Welcome screen
// ============================================================================

pub const WELCOME_TITLE: &str = "daruda";
pub const WELCOME_VERSION: &str = "v0.1.0";
pub const WELCOME_OPEN_FOLDER: &str = "Open Folder...";
pub const WELCOME_RECENT: &str = "Recent Projects";
pub const WELCOME_NEW_EMPTY: &str = "New Empty Window";
pub const WELCOME_NO_RECENT: &str = "No recent projects";

/// Short changelog line shown at the bottom of the welcome panel.
/// Announces the post-multi-project shortcut semantics — `Cmd+O` now
/// adds the project to the current window (policy-aware) instead of
/// spawning a new one. Plain text; no Markdown rendering.
pub const WELCOME_CHANGELOG_OPEN_POLICY: &str =
    "Cmd+O adds the project to this window · Cmd+Shift+O opens in a new window";

// ============================================================================
// File viewer (pane-area viewer opened from Git Changes dock)
// ============================================================================

pub const FILE_VIEWER_LOADING: &str = "Loading…";
pub const FILE_VIEWER_BINARY: &str = "Binary file";
pub const FILE_VIEWER_DELETED: &str = "File deleted";
pub const FILE_VIEWER_EMPTY_DIFF: &str = "No changes";
pub const FILE_VIEWER_STAGED_BADGE: &str = " (staged)";
pub const FILE_VIEWER_PATH_SEP: &str = std::path::MAIN_SEPARATOR_STR;
pub const FILE_VIEWER_TAB_RAW: &str = "Raw";
pub const FILE_VIEWER_TAB_PREVIEW: &str = "Preview";
pub const FILE_VIEWER_TAB_CHANGES: &str = "Changes";
pub const FILE_VIEWER_SHOW_ALL: &str = "Show all";
pub const FILE_VIEWER_HIDE_UNCHANGED: &str = "Hide unchanged";
pub const FILE_VIEWER_CLOSE: &str = "×";
pub const FILE_VIEWER_NO_NEWLINE: &str = "\\ No newline at end of file";
pub const FILE_VIEWER_COPY_ABS_PATH: &str = "Copy Absolute Path";
pub const FILE_VIEWER_COPY_REL_PATH: &str = "Copy Path from Worktree Root";

pub fn file_viewer_more_lines(count: usize) -> String {
    format!("… ({count} more lines)")
}

pub fn file_viewer_byte_truncated(shown: usize, max_bytes: usize, total_count: usize) -> String {
    let size = if max_bytes >= 1024 * 1024 {
        format!("{} MB", max_bytes / (1024 * 1024))
    } else {
        format!("{} KB", max_bytes / 1024)
    };
    if total_count > shown {
        format!("File exceeds {size} — showing first {shown} of {total_count}+ lines")
    } else {
        format!("File exceeds {size} — showing first {shown} lines")
    }
}

pub const FILE_VIEWER_SEARCH_PLACEHOLDER: &str = "Search…";
pub const FILE_VIEWER_SEARCH_NO_MATCH: &str = "No matches";
pub const FILE_VIEWER_SEARCH_PREV: &str = "◀";
pub const FILE_VIEWER_SEARCH_NEXT: &str = "▶";
pub const FILE_VIEWER_SEARCH_CLOSE_BTN: &str = "✕";
pub const FILE_VIEWER_SEARCH_CLEAR: &str = "✕";

// ============================================================================
// Generic widget UI strings
// ============================================================================

/// Checkmark glyph rendered inside a checked checkbox.
pub const UI_CHECKMARK: &str = "✓";
/// Generic loading placeholder used in list widgets while items are fetched.
pub const UI_LOADING: &str = "Loading…";
/// Keystroke-input hint shown while waiting for keystrokes to be recorded.
pub const KEYSTROKE_HINT_RECORDING: &str = "Press keys…";
/// Keystroke-input hint shown when the widget is idle (nothing recorded yet).
pub const KEYSTROKE_HINT_IDLE: &str = "Click to record";

// ----------------------------------------------------------------
// Files view (W-7)
// ----------------------------------------------------------------

pub const FILES_HEADER_LABEL: &str = "Files";
pub const FILES_REFRESH_TOOLTIP: &str = "Refresh";
pub const FILES_REFRESH_GLYPH: &str = "⟳";
pub const FILES_LOADING: &str = "Loading…";
pub const FILES_EMPTY_DIR: &str = "(empty)";
pub const FILES_LOAD_ERROR_PREFIX: &str = "Cannot read:";
pub const FILES_CHEVRON_COLLAPSED: &str = "▸";
pub const FILES_CHEVRON_EXPANDED: &str = "▾";
pub const FILES_CHEVRON_PENDING: &str = "…";

// ----------------------------------------------------------------
// Worktree context menu (W-8)
// ----------------------------------------------------------------

pub const CTX_REVEAL_IN_FINDER: &str = "Reveal in Finder";
pub const CTX_COPY_PATH: &str = "Copy Path";
pub const CTX_EDIT_DESCRIPTION: &str = "Edit Description\u{2026}";
pub const CTX_RENAME: &str = "Rename\u{2026}";
pub const EDIT_DESCRIPTION_MODAL_TITLE: &str = "Edit Description";
pub const EDIT_DESCRIPTION_PLACEHOLDER: &str = "Description (e.g. PR #123 review)";
pub const RENAME_MODAL_TITLE: &str = "Rename Worktree";
pub const RENAME_PLACEHOLDER: &str = "Display name";

// Merge into context menu + modal
pub const CTX_MERGE_INTO: &str = "Merge into\u{2026}";
pub const CTX_MERGE_DISABLED_DIRTY: &str = "Commit your changes first";
pub const CTX_MERGE_DISABLED_DETACHED: &str = "No branch (Detached HEAD)";
pub const MERGE_MODAL_BRANCH_LABEL: &str = "Target branch:";
pub const MERGE_MODAL_NO_TARGETS: &str = "Add another worktree to merge into";
pub const MERGE_MODAL_MERGING: &str = "Merging\u{2026}";
pub const MERGE_MODAL_ALREADY_UP_TO_DATE: &str = "Already up to date";
pub const MERGE_MODAL_TARGET_DIRTY: &str =
    "Target branch has uncommitted changes \u{2014} commit or stash them first";
pub const MERGE_MODAL_CONFLICTS_NOTE: &str =
    "Conflicts detected \u{2014} resolve them in the target worktree, then commit.";
pub const MERGE_MODAL_ABORT_MERGE: &str = "Abort Merge";
pub const MERGE_MODAL_REMOVE_AFTER: &str = "Remove worktree and branch after merge";

// ----------------------------------------------------------------
// Bottom dock panels — tab modals + context menu (B-3 / B-4)
// ----------------------------------------------------------------

pub const CREATE_PANEL_TAB_MODAL_TITLE: &str = "New Panel Tab";
pub const CREATE_PANEL_TAB_PLACEHOLDER: &str = "Tab name (e.g. AI, Build, Git)";
pub const RENAME_PANEL_TAB_MODAL_TITLE: &str = "Rename Tab";
pub const RENAME_PANEL_TAB_PLACEHOLDER: &str = "New tab name";
pub const CTX_PANEL_TAB_RENAME: &str = "Rename\u{2026}";
pub const CTX_PANEL_TAB_DELETE: &str = "Delete\u{2026}";
pub const DELETE_PANEL_TAB_MODAL_TITLE: &str = "Delete Tab";
pub const DELETE_PANEL_TAB_CONFIRM_LABEL: &str = "Delete";
pub const CTX_MACRO_EDIT: &str = "Edit\u{2026}";
pub const CTX_MACRO_DELETE: &str = "Delete\u{2026}";
pub const DELETE_MACRO_MODAL_TITLE: &str = "Delete Macro";
pub const DELETE_MACRO_CONFIRM_LABEL: &str = "Delete";

// Bottom dock — row-preset selector (suffix in the tab strip).
pub const ROW_PRESET_1_LABEL: &str = "1 row";
pub const ROW_PRESET_2_LABEL: &str = "2 rows";
pub const ROW_PRESET_3_LABEL: &str = "3 rows";
pub const ROW_PRESET_CHECK_PREFIX: &str = "\u{2713} ";
pub const ROW_PRESET_UNCHECK_PREFIX: &str = "  ";
pub const ROW_PRESET_TOOLTIP: &str = "Bottom dock height";

// ----------------------------------------------------------------
// Bottom dock — terminal input panel (B-series)
// ----------------------------------------------------------------

pub const BOTTOM_INPUT_TAB_LABEL: &str = "Input";
pub const BOTTOM_INPUT_PLACEHOLDER: &str = "Type to send to terminal\u{2026}";
pub const BOTTOM_INPUT_SEND_BUTTON: &str = "Submit";

// ----------------------------------------------------------------
// Worktrees view — section header + empty state
// ----------------------------------------------------------------

pub const WORKTREES_SECTION_HEADER: &str = "WORKTREES";
pub const WORKTREES_EMPTY_STATE: &str = "No project open";

// ----------------------------------------------------------------
// Claude integration banner (dock prompt to install hooks)
// ----------------------------------------------------------------

pub const CLAUDE_BANNER_ICON: &str = "ⓘ";
pub const CLAUDE_BANNER_TITLE: &str = "Claude Code integration disabled";
pub const CLAUDE_BANNER_HINT: &str = "Click to enable accurate session status";

pub const CLAUDE_CONSENT_TITLE: &str = "Enable Claude Code integration?";
pub const CLAUDE_CONSENT_BODY: &str = concat!(
    "daruda will register hook entries in ~/.claude/settings.json so Claude Code ",
    "can report session status (Working / Needs attention / Idle) for each ",
    "worktree.\n\nOther tools' hooks are preserved. ",
    "You can disable this anytime via the command palette ",
    "(\"Claude: Uninstall Hook Integration\")."
);
pub const CLAUDE_CONSENT_CONFIRM: &str = "Enable";

/// Per-badge tooltip — appended after the session_id prefix to mark
/// the truncation. Localized separately from the active suffix so a
/// single en-dash / horizontal-ellipsis swap covers every badge.
pub const CLAUDE_BADGE_TOOLTIP_ELLIPSIS: &str = "…";
/// Suffix appended to the active session's badge tooltip to identify
/// the one that's bound to the focused tab. Empty for inactive
/// siblings.
pub const CLAUDE_BADGE_TOOLTIP_ACTIVE_SUFFIX: &str = " (active in this tab)";
/// Sub-row label preceding the session badges (e.g. `"3 sessions:"`).
/// Rendered as `format!("{count}{SUFFIX}")`.
pub const CLAUDE_SESSIONS_LABEL_SUFFIX: &str = " sessions:";

// ----------------------------------------------------------------
// Git Changes view (W-6)
// ----------------------------------------------------------------

/// Placeholder text for the git commit message input.
pub const GIT_COMMIT_PLACEHOLDER: &str = "Commit message\u{2026} (Cmd+Enter to commit)";
/// Button label for the commit action in the git commit footer.
pub const GIT_COMMIT_BTN: &str = "Commit";
/// Button label for the push action in the git commit footer.
pub const GIT_PUSH_BTN: &str = "Push";

/// Placeholder shown while the first `git status` for a worktree is
/// still in flight (cache miss).
pub const GIT_LOADING_CHANGES: &str = "Loading changes\u{2026}";
/// Manual-refresh button label inside the loading placeholder.
pub const GIT_REFRESH_BTN: &str = "Refresh";

/// Title for the discard-file confirmation dialog.
pub const GIT_CONFIRM_DISCARD_TITLE: &str = "Discard changes?";
/// OK button label for the discard-file confirmation dialog.
pub const GIT_CONFIRM_DISCARD_OK: &str = "Discard";

/// Single-conflict banner shown at the top of the Git Changes view when
/// `git status` reports one merge conflict. Multi-conflict variants are
/// formatted inline with the count.
pub const GIT_CONFLICT_BANNER_SINGLE: &str = "1 conflict — resolve before committing.";

/// Button label in the non-git worktree placeholder. Click runs
/// `git init` in the worktree path.
pub const GIT_INIT_BTN: &str = "Initialize Git Repo";

/// Title for the push confirmation dialog.
pub const GIT_CONFIRM_PUSH_TITLE: &str = "Push to remote?";
/// Body text for the push confirmation dialog.
pub const GIT_CONFIRM_PUSH_BODY: &str = "Push the current branch to its remote.";
/// OK button label for the push confirmation dialog.
pub const GIT_CONFIRM_PUSH_OK: &str = "Push";

/// Title for the commit confirmation dialog.
pub const GIT_CONFIRM_COMMIT_TITLE: &str = "Commit changes?";
/// OK button label for the commit confirmation dialog.
pub const GIT_CONFIRM_COMMIT_OK: &str = "Commit";

/// Title for the amend confirmation dialog.
pub const GIT_CONFIRM_AMEND_TITLE: &str = "Amend last commit?";
/// Body text for the amend confirmation dialog.
pub const GIT_CONFIRM_AMEND_BODY: &str =
    "This rewrites the last commit. If it has already been pushed, you will need a force-push.";
/// OK button label for the amend confirmation dialog.
pub const GIT_CONFIRM_AMEND_OK: &str = "Amend";

/// Branch label fallback when HEAD is detached.
pub const GIT_DETACHED_LABEL: &str = "detached";
/// Section header for staged files in the Git Changes panel.
pub const GIT_SECTION_STAGED: &str = "Staged";
/// Section header for unstaged / untracked files in the Git Changes panel.
pub const GIT_SECTION_CHANGES: &str = "Changes";
/// Button label to stage all unstaged files at once.
pub const GIT_STAGE_ALL: &str = "Stage All";
/// Button label shown when all files are staged — clicks unstages everything.
pub const GIT_UNSTAGE_ALL: &str = "Unstage All";
/// Button label for the fetch action in the git remote bar.
pub const GIT_FETCH_BTN: &str = "Fetch";
/// Button label for the pull action in the git remote bar.
pub const GIT_PULL_BTN: &str = "Pull";
/// Context menu — stage a single file.
pub const CTX_GIT_STAGE: &str = "Stage";
/// Context menu — unstage a single file.
pub const CTX_GIT_UNSTAGE: &str = "Unstage";
/// Context menu — discard working-tree changes for a file.
pub const CTX_GIT_DISCARD: &str = "Discard Changes";
/// Context menu — open the diff viewer for a file.
pub const CTX_GIT_OPEN_DIFF: &str = "Open Diff";
/// Commit dropdown — amend the last commit with the current staged changes.
pub const CTX_GIT_COMMIT_AMEND: &str = "Amend Last Commit";

// ----------------------------------------------------------------
// Agent chat — role labels
// ----------------------------------------------------------------

/// Chat label for messages authored by the user.
pub const AGENT_CHAT_LABEL_USER: &str = "You";
/// Chat label for messages authored by the agent.
pub const AGENT_CHAT_LABEL_AGENT: &str = "Agent";
/// Chat label for system / tool messages injected into the chat stream.
pub const AGENT_CHAT_LABEL_SYSTEM: &str = "System";

// ----------------------------------------------------------------
// Settings panel
// ----------------------------------------------------------------

pub const MENU_SETTINGS: &str = "Settings\u{2026}";
pub const MENU_OPEN_PROJECT_CONFIG: &str = "Open Project Config\u{2026}";

/// Status-bar / error-banner copy for the project-config flow.
pub const PROJECT_CONFIG_NO_PROJECT: &str =
    "Open a project first — project config has nowhere to live.";
pub const PROJECT_CONFIG_NO_DIR: &str = "Cannot resolve project config directory.";

/// Hover text on the small status-bar dot that indicates a
/// project-layer config file exists for the active project.
pub const STATUS_BAR_PROJECT_CONFIG_TOOLTIP: &str =
    "Project config active — this workspace's [shell] section is overridden.";

/// Inline chip label shown in the status bar when the active git
/// worktree is on a detached HEAD. Lowercase so it reads as a state
/// tag, not a sentence.
pub const STATUS_BAR_DETACHED_CHIP: &str = "detached";

/// Initial contents of a freshly-created
/// `~/.config/daruda/projects/<repo>-<hash>/config.toml`. The user
/// edits this file directly; daruda re-reads it on the next config
/// reload (via the recursive watcher under the user config dir).
pub const PROJECT_CONFIG_TEMPLATE: &str = "\
# daruda project-local config.
#
# Sections specified here override the user-global config
# (~/.config/daruda/config.toml) for this project's daruda windows.
# Sections you don't write keep their user-layer values.
#
# Phase 1 supports the [shell] section only.

# [shell]
# program = \"/usr/local/bin/zsh\"
# close_pane_on_exit = true
";
pub const SETTINGS_TITLE: &str = "Settings";
pub const SETTINGS_SECTION_FONT: &str = "FONT";
pub const SETTINGS_SECTION_CURSOR: &str = "CURSOR";
pub const SETTINGS_SECTION_SHELL: &str = "SHELL";
pub const SETTINGS_SECTION_WINDOW: &str = "WINDOW";
pub const SETTINGS_LABEL_FONT_FAMILY: &str = "Family";
pub const SETTINGS_LABEL_FONT_SIZE: &str = "Size";
pub const SETTINGS_LABEL_VERTICAL_SPACING: &str = "Line Height";
pub const SETTINGS_LABEL_HORIZONTAL_SPACING: &str = "Cell Width";
pub const SETTINGS_LABEL_CURSOR_STYLE: &str = "Style";
pub const SETTINGS_LABEL_CURSOR_BLINKING: &str = "Blinking";
pub const SETTINGS_LABEL_CLOSE_ON_EXIT: &str = "Close pane on exit";
pub const SETTINGS_LABEL_WINDOW_OPACITY: &str = "Opacity";
pub const SETTINGS_LABEL_WINDOW_BLUR: &str = "Background Blur";
pub const SETTINGS_CANCEL: &str = "Cancel";
pub const SETTINGS_SAVE: &str = "Save";
pub const SETTINGS_ERR_FONT_SIZE: &str = "Font size must be a number between 6 and 72.";
pub const SETTINGS_ERR_SPACING: &str = "Spacing must be a number between 0.5 and 2.0.";
pub const SETTINGS_ERR_OPACITY: &str = "Opacity must be a number between 0.1 and 1.0.";
pub const SETTINGS_CURSOR_BLOCK: &str = "Block";
pub const SETTINGS_CURSOR_UNDERLINE: &str = "Underline";
pub const SETTINGS_CURSOR_BAR: &str = "Bar";
pub const SETTINGS_SECTION_THEME: &str = "THEME";
pub const SETTINGS_LABEL_THEME: &str = "Preset";
pub const SETTINGS_LABEL_TERMINAL_THEME: &str = "Terminal Preset";
pub const SETTINGS_LABEL_UI_THEME: &str = "UI Theme";
pub const SETTINGS_UI_THEME_PHASE3_TOOLTIP: &str = "More themes coming in Phase 3.";
pub const SETTINGS_SECTION_TERMINAL: &str = "TERMINAL";
pub const SETTINGS_LABEL_SCROLLBACK: &str = "Scrollback Lines";
pub const SETTINGS_ERR_SCROLLBACK: &str = "Scrollback must be a number between 1 000 and 500 000.";
pub const SETTINGS_SECTION_SIDEBAR: &str = "SIDEBAR";
pub const SETTINGS_LABEL_SHOW_HIDDEN: &str = "Show Hidden Files";
pub const SETTINGS_LABEL_USE_GITIGNORE: &str = "Respect .gitignore";
pub const SETTINGS_SECTION_FILE_VIEWER: &str = "FILE VIEWER";
pub const SETTINGS_LABEL_SYNTAX_THEME: &str = "Syntax Theme";

// ----------------------------------------------------------------
// Dock nav labels — title-case for the new left-rail section list.
// (The uppercase `SETTINGS_SECTION_*` consts above are still used as
//  body-area headers inside each rendered section.)
// ----------------------------------------------------------------
pub const SETTINGS_NAV_GENERAL: &str = "General";
pub const SETTINGS_NAV_FONT: &str = "Font";
pub const SETTINGS_NAV_CURSOR: &str = "Cursor";
pub const SETTINGS_NAV_SHELL: &str = "Shell";
pub const SETTINGS_NAV_WINDOW: &str = "Window";
pub const SETTINGS_NAV_TERMINAL: &str = "Terminal";
pub const SETTINGS_NAV_SIDEBAR: &str = "Dock";
pub const SETTINGS_NAV_FILE_VIEWER: &str = "File Viewer";
pub const SETTINGS_NAV_CLIPBOARD: &str = "Clipboard";
pub const SETTINGS_NAV_PANELS: &str = "Panels";
pub const SETTINGS_NAV_CLAUDE_STATUS: &str = "Claude Status";
pub const SETTINGS_NAV_NOTIFICATIONS: &str = "Notifications";
pub const SETTINGS_NAV_KEYMAP: &str = "Keymap";
pub const SETTINGS_NAV_PLUGIN: &str = "Plugin";

// New body-section headers (Phase 1 additions)
pub const SETTINGS_SECTION_GENERAL: &str = "GENERAL";
pub const SETTINGS_SECTION_CLIPBOARD: &str = "CLIPBOARD";
pub const SETTINGS_SECTION_PANELS: &str = "PANELS";
pub const SETTINGS_SECTION_CLAUDE_STATUS: &str = "CLAUDE STATUS";
pub const SETTINGS_SECTION_NOTIFICATIONS: &str = "NOTIFICATIONS";
pub const SETTINGS_SECTION_KEYMAP: &str = "KEYMAP";
pub const SETTINGS_SECTION_PLUGIN: &str = "PLUGINS";

// New labels / actions for Phase 1 sections
pub const SETTINGS_LABEL_CLIPBOARD_STREAMING: &str = "Streaming Cap (bytes)";
pub const SETTINGS_LABEL_GRID_COLUMNS: &str = "Macro Grid Columns";
pub const SETTINGS_LABEL_CLAUDE_STATUS_ENABLE: &str = "Enable Claude Code integration";
pub const SETTINGS_ERR_CLIPBOARD: &str =
    "Streaming cap must be a number between 4 096 and 67 108 864 (4 KiB – 64 MiB).";
pub const SETTINGS_ERR_GRID_COLUMNS: &str =
    "Macro grid columns must be a whole number between 1 and 16.";
pub const SETTINGS_OPEN_CONFIG_FILE: &str = "Open Config File";
pub const SETTINGS_PLACEHOLDER_KEYMAP: &str = "Keymap GUI editor coming soon. For now, edit the [keybindings] section of your config file directly.";
pub const SETTINGS_PLACEHOLDER_NOTIFICATIONS: &str = "Notifications UI coming soon. For now, edit the [notifications] section of your config file directly.";

// Plugin section — install / uninstall UI labels
pub const SETTINGS_PLUGIN_INSTALLED_HEADER: &str = "Installed";
pub const SETTINGS_PLUGIN_AVAILABLE_HEADER: &str = "Available from marketplace";
pub const SETTINGS_PLUGIN_NONE_INSTALLED: &str =
    "No plugins installed. Browse the list below to add one.";
pub const SETTINGS_PLUGIN_NONE_AVAILABLE: &str =
    "No marketplace plugins registered. Run `claude plugin marketplace add <url>` from a terminal.";
pub const SETTINGS_PLUGIN_INSTALL: &str = "Install";
pub const SETTINGS_PLUGIN_UNINSTALL: &str = "Uninstall";
pub const SETTINGS_PLUGIN_INSTALLING: &str = "Installing…";
pub const SETTINGS_PLUGIN_UNINSTALLING: &str = "Uninstalling…";

// Plugin detail pane (Settings → Plugin master-detail layout)
pub const SETTINGS_PLUGIN_DETAIL_EMPTY: &str = "Select a plugin to see details.";
pub const SETTINGS_PLUGIN_DETAIL_MARKETPLACE: &str = "Marketplace";
pub const SETTINGS_PLUGIN_DETAIL_VERSION: &str = "Version";
pub const SETTINGS_PLUGIN_DETAIL_PATH: &str = "Path";
pub const SETTINGS_PLUGIN_DETAIL_SCOPE: &str = "Scope";
pub const SETTINGS_PLUGIN_DETAIL_AVAILABILITY: &str = "Availability";
pub const SETTINGS_PLUGIN_DETAIL_STATUS_INSTALLED: &str = "Installed";
pub const SETTINGS_PLUGIN_DETAIL_STATUS_AVAILABLE: &str = "Available (not installed)";
pub const SETTINGS_PLUGIN_DETAIL_SKILLS_HEADER: &str = "Skills";
pub const SETTINGS_PLUGIN_DETAIL_NO_SKILLS: &str = "This plugin doesn't expose any skills.";
pub const SETTINGS_PLUGIN_DETAIL_UNKNOWN: &str = "—";
pub const SETTINGS_PLUGIN_SKILL_VIEW: &str = "View";
pub const SETTINGS_PLUGIN_SKILL_DESCRIPTION: &str = "Description";
pub const SETTINGS_PLUGIN_SKILL_INVOCATION: &str = "Invocation";
pub const SETTINGS_PLUGIN_SKILL_ARGUMENT_HINT: &str = "Argument hint";
pub const SETTINGS_PLUGIN_SKILL_ALLOWED_TOOLS: &str = "Allowed tools";
pub const SETTINGS_PLUGIN_SKILL_PATHS: &str = "Paths";
pub const SETTINGS_PLUGIN_SKILL_WHEN_TO_USE: &str = "When to use";
pub const SETTINGS_PLUGIN_SKILL_INVOCATION_BOTH: &str = "user + model";
pub const SETTINGS_PLUGIN_SKILL_INVOCATION_USER_ONLY: &str = "user only";
pub const SETTINGS_PLUGIN_SKILL_INVOCATION_MODEL_ONLY: &str = "model only";
pub const SETTINGS_PLUGIN_SKILL_INVOCATION_DISABLED: &str = "disabled";
pub const SETTINGS_PLUGIN_SKILL_BODY: &str = "Body (SKILL.md)";
pub const SETTINGS_PLUGIN_SKILL_BACK: &str = "← Back";
pub const SETTINGS_PLUGIN_SKILL_BODY_LOADING: &str = "Loading SKILL.md…";
pub const SETTINGS_PLUGIN_SKILL_BODY_ERROR: &str = "Failed to load SKILL.md:";

// ============================================================================
// Context menu labels (right-click on tab bar / pane header)
// ============================================================================

pub const CTX_CLOSE_TAB: &str = "Close Tab";
pub const CTX_CLOSE_OTHER_TABS: &str = "Close Other Tabs";
pub const CTX_CLOSE_TABS_TO_RIGHT: &str = "Close Tabs to the Right";
pub const CTX_MOVE_TAB_LEFT: &str = "Move Tab Left";
pub const CTX_MOVE_TAB_RIGHT: &str = "Move Tab Right";
pub const CTX_NEW_TAB: &str = "New Tab";
pub const CTX_SPLIT_RIGHT: &str = "Split Right";
pub const CTX_SPLIT_DOWN: &str = "Split Down";
pub const CTX_COPY_FILE_PATH: &str = "Copy File Path";
pub const CTX_COPY_RELATIVE_PATH: &str = "Copy Relative Path";
pub const CTX_CLOSE_FILE_VIEWER: &str = "Close File Viewer";
pub const CTX_CLOSE_PANE: &str = "Close Pane";
pub const CTX_ZOOM_PANE: &str = "Zoom Pane";
pub const CTX_UNZOOM_PANE: &str = "Unzoom Pane";

// ============================================================================
// Notifications
// ============================================================================

/// Title for "long-running command finished" desktop notifications.
pub const NOTIFICATION_LONG_RUNNING_TITLE: &str = "Command finished";

/// Format a `Duration` as a compact, human-friendly span for the
/// "command finished" notification body. Examples: `42s`, `1m 03s`,
/// `2h 15m`. Sub-second resolution is dropped; the user threshold
/// is in whole seconds and any "completed in <1s" command is below
/// the long-running cut-off anyway.
pub fn format_duration_compact(d: std::time::Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs < 60 {
        return format!("{total_secs}s");
    }
    let hours = total_secs / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m {seconds:02}s")
    }
}

// ============================================================================
// Right panel — Skills tab
// ============================================================================

/// Section heading for the project-scope group.
pub const SKILLS_PROJECT: &str = "Project";
/// Section heading for the personal-scope group.
pub const SKILLS_PERSONAL: &str = "Personal";
/// Section heading for the plugin-scope group. Plugin skills come
/// from marketplace installs (`~/.claude/plugins/cache/...`) and are
/// **read-only** in daruda — Edit / Delete / Rename are disabled.
pub const SKILLS_PLUGIN: &str = "Plugin";
/// Header button — opens the Create modal.
pub const SKILLS_NEW_BUTTON: &str = "+ New skill";
/// Header button on the right-panel Skills tab — opens Settings →
/// Plugin so the user can install / uninstall plugins. Install /
/// Uninstall is no longer surfaced inline on the panel rows.
pub const SKILLS_MANAGE_PLUGINS_BUTTON: &str = "Manage…";

// ----------------------------------------------------------------
// Skill invocation modal — clicking a row opens this, Submit writes
// `/<skill> <input>\n` into the focused terminal pane.
// ----------------------------------------------------------------
pub const SKILLS_INVOKE_CANCEL: &str = "Cancel";
pub const SKILLS_INVOKE_SUBMIT: &str = "Submit";
pub const SKILLS_INVOKE_SUBMITTING: &str = "Sending…";
pub const SKILLS_INVOKE_PLACEHOLDER_DEFAULT: &str = "Enter input…";
pub const SKILLS_INVOKE_NO_TERMINAL: &str = "No active terminal — focus a terminal pane first.";

// ----------------------------------------------------------------
// Skills search bar (C-4) — substring filter atop the Skills tab.
// ----------------------------------------------------------------
pub const SKILLS_SEARCH_PLACEHOLDER: &str = "Search skills…";
pub const SKILLS_SEARCH_EMPTY_PREFIX: &str = "No skills match ";
/// Glyph for the in-field clear button. Rendered on the trailing edge of
/// the search input only while the query is non-empty.
pub const SKILLS_SEARCH_CLEAR_ICON: &str = "✕";

/// Body shown when both scopes are empty.
pub const SKILLS_EMPTY_PROJECT: &str =
    "No project skills — create one to teach Claude a project-specific workflow.";
pub const SKILLS_EMPTY_PERSONAL: &str = "No personal skills yet.";
pub const SKILLS_EMPTY_PLUGIN: &str =
    "No plugin skills installed. Install a plugin via Claude Code's `/plugin` command.";
/// Hover hint shown on plugin rows so the user knows why Edit/× are missing.
pub const SKILLS_PLUGIN_READ_ONLY: &str = "managed by plugin";
/// Chip text on plugin rows discovered through a registered
/// marketplace but not yet `/plugin install`-ed. Surfacing them lets
/// the user browse the catalog.
pub const SKILLS_PLUGIN_AVAILABLE: &str = "available";
/// Tooltip / chip text when a project skill shadows a personal one.
pub const SKILLS_OVERRIDES_PERSONAL: &str = "overrides personal";

/// Title strings for the CRUD modals.
pub const SKILLS_NEW_TITLE: &str = "New skill";
pub const SKILLS_EDIT_TITLE: &str = "Edit skill";
pub const SKILLS_DELETE_TITLE: &str = "Delete skill?";

/// Field labels in the CRUD modals.
pub const SKILLS_FIELD_NAME: &str = "Name";
pub const SKILLS_FIELD_SCOPE: &str = "Scope";
pub const SKILLS_FIELD_DESCRIPTION: &str = "Description";
pub const SKILLS_FIELD_WHEN_TO_USE: &str = "When to use";
pub const SKILLS_FIELD_ALLOWED_TOOLS: &str = "Allowed tools";
pub const SKILLS_FIELD_ARG_HINT: &str = "Argument hint";
pub const SKILLS_FIELD_PATHS: &str = "Paths";
pub const SKILLS_FIELD_MODEL: &str = "Model";
pub const SKILLS_FIELD_BODY: &str = "Body (markdown)";
pub const SKILLS_TOGGLE_USER_INVOCABLE: &str = "User invocable";
pub const SKILLS_TOGGLE_DISABLE_MODEL: &str = "Disable model invocation";
pub const SKILLS_BUTTON_RENAME: &str = "Rename…";
pub const SKILLS_BUTTON_OPEN_FINDER: &str = "Open in Finder";
pub const SKILLS_BUTTON_DELETE: &str = "Delete";
/// Hover-only `[View]` action shown on plugin rows in place of Edit —
/// opens SKILL.md in the daruda file viewer.
pub const SKILLS_BUTTON_VIEW: &str = "View";
/// `[Edit]` label on the skill row's hover-only action overlay.
pub const SKILLS_BUTTON_EDIT: &str = "Edit";
/// Single-glyph `×` delete affordance on the skill row overlay.
/// Lives as a constant so the glyph stays consistent across rows and
/// is easy to swap if a future revision uses an icon.
pub const SKILLS_BUTTON_DELETE_ICON: &str = "×";
pub const SKILLS_DELETE_BODY_PREFIX: &str =
    "This removes the skill directory and every auxiliary file inside it. This cannot be undone.";

/// Validation messages for the modal banner.
pub const SKILLS_NAME_EMPTY: &str = "Name is required.";
pub const SKILLS_NAME_INVALID: &str =
    "Name must be lowercase letters, digits, hyphens, or underscores.";
pub const SKILLS_NAME_LEADING: &str = "Name cannot start with a hyphen or underscore.";
pub const SKILLS_NAME_TOO_LONG: &str = "Name must be 64 characters or fewer.";
pub const SKILLS_NAME_DUPLICATE: &str = "A skill with this name already exists in this scope.";
pub const SKILLS_DESCRIPTION_TOO_LONG_HINT: &str =
    "Description over 1536 characters — Claude Code may truncate it.";
pub const SKILLS_NO_PROJECT_HINT: &str =
    "Project skills require an active worktree. Open a project to enable this scope.";

// ============================================================================
// Right panel — Tools tab (MCP servers)
// ============================================================================

/// Section heading for project-scope MCP servers (`<wt>/.mcp.json`).
pub const MCP_PROJECT: &str = "Project";
/// Section heading for personal-scope MCP servers (`~/.claude/settings.json`).
pub const MCP_PERSONAL: &str = "Personal";
/// Header button — opens AddMcpServerModal.
pub const MCP_NEW_BUTTON: &str = "+ Add server";
/// Body when project scope has no servers and there is an active worktree.
pub const MCP_EMPTY_PROJECT: &str =
    "No project MCP servers — `+ Add server` to create `.mcp.json`.";
/// Body when project scope has no active worktree (welcome-style window).
pub const MCP_NO_PROJECT_HINT: &str =
    "Project MCP servers require an active worktree. Open a project to enable this scope.";
/// Body when personal scope is empty.
pub const MCP_EMPTY_PERSONAL: &str = "No personal MCP servers configured.";
/// Row status label — server is configured and not disabled.
pub const MCP_STATUS_ENABLED: &str = "enabled";
/// Row status label — server has `"disabled": true` in config.
pub const MCP_STATUS_DISABLED: &str = "disabled";
/// Row status label — required fields for the chosen transport are missing.
pub const MCP_STATUS_MALFORMED: &str = "malformed";

/// CRUD modal titles.
pub const MCP_NEW_TITLE: &str = "Add MCP server";
pub const MCP_EDIT_TITLE: &str = "Edit MCP server";
pub const MCP_DELETE_TITLE: &str = "Delete MCP server?";

/// Field labels in the CRUD modals.
pub const MCP_FIELD_NAME: &str = "Name";
pub const MCP_FIELD_SCOPE: &str = "Scope";
pub const MCP_FIELD_TRANSPORT: &str = "Transport";
pub const MCP_FIELD_COMMAND: &str = "Command";
pub const MCP_FIELD_ARGS: &str = "Args (space-separated)";
pub const MCP_FIELD_URL: &str = "URL";
pub const MCP_FIELD_ENV: &str = "Env (KEY=VALUE per line)";
pub const MCP_FIELD_HEADERS: &str = "Headers (KEY=VALUE per line)";

/// Buttons shared across CRUD modals + row hover actions.
pub const MCP_BUTTON_ADD: &str = "Add";
pub const MCP_BUTTON_EDIT: &str = "Edit";
pub const MCP_BUTTON_DELETE: &str = "Delete";
pub const MCP_BUTTON_SAVE: &str = "Save";
pub const MCP_BUTTON_CANCEL: &str = "Cancel";
/// Displayed on the primary action button while the save is in flight.
pub const MCP_SAVING_LABEL: &str = "Saving…";
/// Toggle label inside the AddModal / EditModal that maps to the
/// JSON `"disabled": true` key.
pub const MCP_FIELD_DISABLED: &str = "Disabled";

/// Confirm-modal body for the Delete flow.
pub const MCP_DELETE_BODY_PREFIX: &str =
    "This removes the server entry from disk. This cannot be undone.";

/// Validation messages for the AddModal / EditModal banner.
pub const MCP_NAME_EMPTY: &str = "Name is required.";
pub const MCP_NAME_INVALID: &str = "Name must be alphanumeric, underscore, or hyphen.";
pub const MCP_NAME_LEADING: &str = "Name cannot start with a hyphen or underscore.";
pub const MCP_NAME_TOO_LONG: &str = "Name must be 63 characters or fewer.";
pub const MCP_NAME_DUPLICATE: &str = "A server with this name already exists in this scope.";
pub const MCP_COMMAND_REQUIRED: &str = "Command is required for stdio transport.";
pub const MCP_URL_REQUIRED: &str = "URL is required for sse / http transport.";
pub const MCP_URL_INVALID: &str = "URL must start with http:// or https://.";
pub const MCP_ENV_INVALID: &str = "Each env line must be KEY=VALUE.";

/// Transport option labels for the dropdown.
pub const MCP_TRANSPORT_STDIO: &str = "stdio";
pub const MCP_TRANSPORT_SSE: &str = "sse";
pub const MCP_TRANSPORT_HTTP: &str = "http";

/// Scope option labels for the dropdown.
pub const MCP_SCOPE_PROJECT: &str = "Project (.mcp.json)";
pub const MCP_SCOPE_PERSONAL: &str = "Personal (~/.claude/settings.json)";

// ============================================================================
// Error toast (Layer 1 of the error-reporting pipeline)
// ============================================================================

/// Severity glyphs in the toast leading icon slot.
pub const TOAST_ICON_INFO: &str = "ℹ";
pub const TOAST_ICON_WARNING: &str = "⚠";
pub const TOAST_ICON_ERROR: &str = "✕";

/// Action button labels.
pub const TOAST_BUTTON_COPY: &str = "Copy";
pub const TOAST_BUTTON_DETAILS: &str = "Details";
/// Glyph for the dismiss ✕ button. Same character as the title-bar
/// pane close, separate constant so a future redesign can split them.
pub const TOAST_BUTTON_DISMISS: &str = "×";
/// Transient one-second affordance shown after `[Copy]` is clicked so
/// the user has visual confirmation the clipboard write happened.
pub const TOAST_BUTTON_COPIED: &str = "Copied";
/// `×N` repeat counter prefix.
pub const TOAST_REPEAT_PREFIX: &str = "×";

// ============================================================================
// Error-report Details modal (Layer 2)
// ============================================================================

/// Title prefix, e.g. `Error: PTY writer thread died`. The trailing
/// title is drawn from the underlying [`ErrorReport`] verbatim.
pub const ERROR_MODAL_TITLE_PREFIX: &str = "Error: ";

/// Footer button labels. `Copy report` writes the full plain-text
/// rendering (including system-info trailer) to the clipboard;
/// `Open log file` shells out to the system handler for the day's
/// NDJSON log; `Close` dismisses.
pub const ERROR_MODAL_BUTTON_COPY: &str = "Copy report";
pub const ERROR_MODAL_BUTTON_COPIED: &str = "Copied";
pub const ERROR_MODAL_BUTTON_OPEN_LOG: &str = "Open log file";
pub const ERROR_MODAL_BUTTON_CLOSE: &str = "Close";

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn duration_below_minute_renders_seconds() {
        assert_eq!(format_duration_compact(Duration::from_secs(0)), "0s");
        assert_eq!(format_duration_compact(Duration::from_secs(42)), "42s");
        assert_eq!(format_duration_compact(Duration::from_secs(59)), "59s");
    }

    #[test]
    fn duration_under_hour_renders_minutes_and_seconds() {
        assert_eq!(format_duration_compact(Duration::from_secs(60)), "1m 00s");
        assert_eq!(format_duration_compact(Duration::from_secs(63)), "1m 03s");
        assert_eq!(
            format_duration_compact(Duration::from_secs(3_599)),
            "59m 59s"
        );
    }

    #[test]
    fn duration_over_hour_renders_hours_and_minutes() {
        assert_eq!(
            format_duration_compact(Duration::from_secs(3_600)),
            "1h 00m"
        );
        assert_eq!(
            format_duration_compact(Duration::from_secs(8_115)),
            "2h 15m"
        );
    }

    #[test]
    fn reset_countdown_zero_renders_now() {
        assert_eq!(format_reset_countdown(Duration::from_secs(0)), "Resets now");
    }

    #[test]
    fn reset_countdown_sub_minute_renders_lt1m() {
        // 1..59 seconds collapses to "<1m" so the user doesn't see
        // "Resets in 0m" linger for a whole minute right before
        // the rolling window resets.
        assert_eq!(
            format_reset_countdown(Duration::from_secs(1)),
            "Resets in <1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(30)),
            "Resets in <1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(59)),
            "Resets in <1m"
        );
    }

    #[test]
    fn reset_countdown_under_hour_renders_minutes() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(60)),
            "Resets in 1m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(59 * 60)),
            "Resets in 59m"
        );
    }

    #[test]
    fn reset_countdown_under_day_renders_hours_and_minutes() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(3_600)),
            "Resets in 1h 0m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(2 * 3_600 + 14 * 60)),
            "Resets in 2h 14m"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(23 * 3_600 + 59 * 60)),
            "Resets in 23h 59m"
        );
    }

    #[test]
    fn reset_countdown_over_day_renders_days_and_hours() {
        assert_eq!(
            format_reset_countdown(Duration::from_secs(24 * 3_600)),
            "Resets in 1d 0h"
        );
        assert_eq!(
            format_reset_countdown(Duration::from_secs(3 * 24 * 3_600 + 7 * 3_600)),
            "Resets in 3d 7h"
        );
    }

    #[test]
    fn service_status_label_operational_ignores_description() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::None,
            description: "stale message".into(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Operational");
    }

    #[test]
    fn service_status_label_uses_description_for_incidents() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Minor,
            description: "Increased 4xx errors on /messages".into(),
            fetched_at: None,
        };
        assert_eq!(
            service_status_label(&s),
            "Increased 4xx errors on /messages"
        );
    }

    #[test]
    fn service_status_label_falls_back_to_default_when_description_empty() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Major,
            description: String::new(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Partial outage");
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Critical,
            description: String::new(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Major outage");
    }

    #[test]
    fn service_status_label_unknown_ignores_description() {
        let s = daruda_claude::ServiceStatus {
            indicator: daruda_claude::StatusIndicator::Unknown,
            description: "garbage".into(),
            fetched_at: None,
        };
        assert_eq!(service_status_label(&s), "Status unavailable");
    }
}
