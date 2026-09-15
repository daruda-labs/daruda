//! User-visible strings and perceptible timings owned by this crate.
//!
//! Localisation goes through the `terminal.*` keys in `locales/`; the
//! `pub const`s that remain are glyphs and numbers, which read the same
//! in every locale. Strings the *app* renders live in
//! `daruda::surface::strings` instead — this module is not their home.
//! Colors and pixel sizes go in [`super::theme`].

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
pub fn search_regex_error() -> String {
    rust_i18n::t!("terminal.search_regex_error").into_owned()
}

/// Right-aligned status label when the current query compiles cleanly
/// but finds no hits in scrollback.
pub fn search_no_matches() -> String {
    rust_i18n::t!("terminal.search_no_matches").into_owned()
}

// ============================================================================
// Fallback / placeholder text
// ============================================================================

/// Title returned when a session hasn't yet received an OSC 0 / 2
/// title. Shown in tab bars and window chrome.
pub fn fallback_title() -> String {
    rust_i18n::t!("terminal.fallback_title").into_owned()
}

pub fn pty_reader_thread_died() -> String {
    rust_i18n::t!("terminal.pty_reader_thread_died").into_owned()
}

pub fn pty_writer_thread_died() -> String {
    rust_i18n::t!("terminal.pty_writer_thread_died").into_owned()
}

pub fn pty_open_failed(error: &str) -> String {
    rust_i18n::t!("terminal.pty_open_failed", error = error).into_owned()
}

pub fn pty_spawn_shell_failed(error: &str) -> String {
    rust_i18n::t!("terminal.pty_spawn_shell_failed", error = error).into_owned()
}

pub fn pty_io_failed(error: &str) -> String {
    rust_i18n::t!("terminal.pty_io_failed", error = error).into_owned()
}

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

// ============================================================================
// Agent — task state glyphs
// ============================================================================

/// Task state icons — used in task list panel.
pub const AGENT_TASK_QUEUED: &str = "○";
pub const AGENT_TASK_RUNNING: &str = "◉";
pub const AGENT_TASK_DONE: &str = "✓";
pub const AGENT_TASK_ERROR: &str = "✗";
pub const AGENT_TASK_CANCELLED: &str = "⊘";

// ============================================================================
// Right-panel Tasks tab
// ============================================================================

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

/// Number of tool-use failures past which the inline `failures N/M`
/// counter starts surfacing on the row. Below this threshold the
/// occasional `Bash` retry is too noisy to show. Intentionally lower
/// than `daruda_store::tasks::TASK_TOOL_USE_FAILURE_THRESHOLD` (the cap
/// that auto-escalates to `Error`) so the user sees the row trending
/// before it actually flips state.
pub const RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD: u32 = 3;
