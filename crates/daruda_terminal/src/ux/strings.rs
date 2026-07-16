//! User-visible strings and perceptible timings.
//!
//! Localisation hook-point. Every literal the user actually reads
//! (overlay banners, fallback titles, …) lives here, next to the
//! `Duration`s that control how fast those elements appear or fade.
//! Colors and pixel sizes go in [`super::theme`] instead.

use std::time::Duration;

// ============================================================================
// Overlay banners
// ============================================================================

/// Value advertised to child shells via `TERM_PROGRAM`. Shell
/// integration scripts (powerlevel10k, starship, fish) key off this
/// to decide whether to emit OSC 133 / OSC 7 sequences.
pub const TERM_PROGRAM_VALUE: &str = "daruda";

/// Right-aligned status label shown while the search query fails to
/// compile as a regex.
pub const SEARCH_REGEX_ERROR: &str = "regex error";

/// Right-aligned status label when the current query compiles cleanly
/// but finds no hits in scrollback.
pub const SEARCH_NO_MATCHES: &str = "no matches";

// ============================================================================
// Fallback / placeholder text
// ============================================================================

/// Title returned when a session hasn't yet received an OSC 0 / 2
/// title. Shown in tab bars and window chrome.
pub const FALLBACK_TITLE: &str = "shell";

// ============================================================================
// Key contexts (for `KeyContext::new`)
// ============================================================================

/// Default focus context — standard terminal keymap applies.
pub const KEY_CONTEXT_TERMINAL: &str = "Terminal";

/// Overlay context — printable keys route into the search query
/// instead of the PTY, enter/shift-enter navigate matches, etc.
pub const KEY_CONTEXT_SEARCH: &str = "TerminalSearch";

// ============================================================================
// Perceptible timings
// ============================================================================

/// Wrap-flash duration for prompt/command jumps that wrapped around
/// the list (Cmd+Shift+↑/↓). Short enough to feel like a flicker, long
/// enough to register.
pub const PROMPT_JUMP_FLASH: Duration = Duration::from_millis(180);

/// Visual-bell flash fade-out for an xterm `BEL`. Kept short so the
/// terminal reads as responsive; long flashes feel like a freeze.
pub const BELL_FLASH: Duration = Duration::from_millis(100);

/// Caret blink half-period for text inputs (input field, multi-line area).
/// 530ms matches the convention used by macOS NSTextView and most editors.
pub const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(530);

// ============================================================================
// Agent — status summaries and icons
// ============================================================================

/// Fallback phase when agent is running but no Thinking event logged.
pub const AGENT_STATUS_WORKING: &str = "working";
/// Agent completed summary.
pub const AGENT_STATUS_DONE: &str = "done";
/// Agent error summary.
pub const AGENT_STATUS_ERROR: &str = "error";

/// Agent event icons — used in activity log timeline.
pub const AGENT_ICON_TASK_STARTED: &str = "▶";
pub const AGENT_ICON_THINKING: &str = "🧠";
pub const AGENT_ICON_TOOL_READ: &str = "🔍";
pub const AGENT_ICON_TOOL_EDIT: &str = "📝";
pub const AGENT_ICON_TOOL_BASH: &str = "🔧";
pub const AGENT_ICON_SUCCESS: &str = "✓";
pub const AGENT_ICON_ERROR: &str = "✗";

/// Task state icons — used in task list panel.
pub const AGENT_TASK_QUEUED: &str = "○";
pub const AGENT_TASK_RUNNING: &str = "◉";
pub const AGENT_TASK_DONE: &str = "✓";
pub const AGENT_TASK_ERROR: &str = "✗";
pub const AGENT_TASK_CANCELLED: &str = "⊘";

/// Chat panel placeholder text.
pub const AGENT_CHAT_PLACEHOLDER: &str = "Start a conversation";
/// Chat input box placeholder.
pub const AGENT_CHAT_INPUT_HINT: &str = "Ask the agent...";
/// Activity log empty placeholder.
pub const AGENT_LOG_EMPTY: &str = "No agent activity";
/// Task list empty placeholder.
pub const AGENT_TASK_LIST_EMPTY: &str = "No agent tasks";

// ============================================================================
// Toast notification timings
// ============================================================================

/// Default auto-dismiss duration for informational toasts (no action button).
pub const TOAST_DEFAULT_DURATION: Duration = Duration::from_secs(10);

/// Auto-dismiss duration for toasts that carry an action button, giving the
/// user more time to react before the notice disappears.
pub const TOAST_ACTION_DURATION: Duration = Duration::from_secs(15);

// ============================================================================
// Bottom dock — customizable panels (macros, future widgets)
// ============================================================================

/// Body shown when the user has deleted every tab.
pub const PANELS_NO_TABS: &str = "No tabs. Click [+] to create one.";
/// Body shown when active_tab_id points at a tab that no longer exists.
pub const PANELS_NO_ACTIVE_TAB: &str = "Select a tab.";

/// Macro edit modal — shortcut record button labels.
pub const MACRO_RECORD_BUTTON_IDLE: &str = "● Record";
pub const MACRO_RECORD_BUTTON_RECORDING: &str = "Press shortcut\u{2026}";

// ============================================================================
// Lanes view
// ============================================================================

/// Hint shown in the Lanes view when the project is not a Git repo.
pub const LANE_NON_GIT_HINT: &str = "Open a Git repository to use multiple lanes.";

/// Label for the Git initialization affordance button.
pub const LANE_GIT_INIT_LABEL: &str = "[ Initialize Git Repo ]";

// ============================================================================
// Right-panel Tasks tab
// ============================================================================

/// State labels shown next to a task row's title.
pub const RIGHT_PANEL_TASK_BACKLOG_LABEL: &str = "Backlog";
pub const RIGHT_PANEL_TASK_RUNNING_LABEL: &str = "Running";
pub const RIGHT_PANEL_TASK_CANCELLED_LABEL: &str = "Cancelled";

/// Prefix for `Done` state — the `end_reason` flavour is appended
/// inline (e.g. `Done (Stop)`).
pub const RIGHT_PANEL_TASK_DONE_LABEL_PREFIX: &str = "Done";

/// Prefix for `Error` state — the truncated message is appended
/// inline (e.g. `Error: exit nonzero`).
pub const RIGHT_PANEL_TASK_ERROR_LABEL_PREFIX: &str = "Error";

/// Empty-state placeholders by filter.
pub const RIGHT_PANEL_TASK_EMPTY_ALL: &str = "No tasks. Click [+ New] to create one.";
pub const RIGHT_PANEL_TASK_EMPTY_BACKLOG: &str = "No backlog tasks.";
pub const RIGHT_PANEL_TASK_EMPTY_RUNNING: &str = "No running tasks.";
pub const RIGHT_PANEL_TASK_EMPTY_DONE: &str = "No completed tasks.";

/// Length of the leading session-id slice used as a Tasks-tab badge.
pub const RIGHT_PANEL_TASK_SESSION_BADGE_LEN: usize = 8;

/// Glyph prefixing the per-row subtask progress badge (`☑done/total`).
/// Rendered at the same trailing position as the duration / session
/// cells so every row keeps the same column layout.
pub const RIGHT_PANEL_SUBTASK_PROGRESS_GLYPH: &str = "☑";

/// Glyph trailing the session-id badge while the matching Claude
/// session is generating tokens or running a tool — matches the
/// "spinning" indicator vocabulary used in the lane dock.
pub const RIGHT_PANEL_TASK_SESSION_STATUS_WORKING: &str = "⟳";

/// Glyph trailing the session-id badge while the session is idle
/// (turn ended, waiting for the next user prompt) or still
/// connecting. Drawn neutral so a quiet session reads as quiet.
pub const RIGHT_PANEL_TASK_SESSION_STATUS_IDLE: &str = "●";

/// Glyph trailing the session-id badge while the session is waiting
/// for the user — permission prompt, idle prompt, elicitation.
pub const RIGHT_PANEL_TASK_SESSION_STATUS_NEEDS_ATTENTION: &str = "⚠";

/// Inline-text prefix for the `failures N/M` counter rendered when a
/// `Running` task's tool-use failure count climbs past
/// [`RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD`] — the threshold is
/// intentionally lower than
/// `daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD` (the cap
/// that auto-escalates to `Error`) so the user sees the row trending
/// before it actually flips state.
pub const RIGHT_PANEL_TASK_FAILURES_LABEL: &str = "failures ";

/// Number of tool-use failures past which the inline `failures N/M`
/// counter starts surfacing on the row. Below this threshold the
/// occasional `Bash` retry is too noisy to show.
pub const RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD: u32 = 3;
