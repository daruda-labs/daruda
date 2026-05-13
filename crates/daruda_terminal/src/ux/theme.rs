//! Terminal-view theme — colors, pixel sizes, font sizes.
//!
//! Every `hsla(…)` / `px(…)` literal that isn't dictated by a protocol
//! belongs here so retheming touches a single file.
//!
//! Strings and `Duration`s go in [`super::strings`] so reskinning
//! (this file) stays orthogonal to localization / timing tuning.

use gpui::Hsla;

/// `const`-friendly `Hsla` constructor — daruda speaks in degrees
/// (0–360 for hue) while `gpui::Hsla` stores hue as a fraction in
/// [0, 1]. The conversion lives here, at the daruda↔gpui boundary, so
/// every constant in this file reads naturally as
/// `hsla(<degrees>, <saturation>, <lightness>, <alpha>)` without
/// per-call `/ 360.0` boilerplate. This also closes the trap where
/// `gpui::hsla(210.0, …)` silently clamped to red — daruda's
/// constants now always pass through this normalizing constructor.
///
/// `gpui::hsla` is not a `const fn` (it `clamp`s each component), so
/// we couldn't reuse it for `pub const` declarations anyway; the
/// struct-literal init below matches the exact field layout gpui's
/// runtime version produces and is fully const-evaluable.
const fn hsla(h_degrees: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        h: h_degrees / 360.0,
        s,
        l,
        a,
    }
}

// ============================================================================
// Search overlay — panel chrome
// ============================================================================

/// Dark translucent panel background behind the search bar.
pub const SEARCH_PANEL_BG: Hsla = hsla(0.0, 0.0, 0.13, 0.97);

/// 1-pixel border around the search panel.
pub const SEARCH_PANEL_BORDER: Hsla = hsla(0.0, 0.0, 0.30, 0.95);

/// Corner radius of the search panel (px).
pub const SEARCH_PANEL_RADIUS: f32 = 6.0;

// ============================================================================
// Search overlay — query input
// ============================================================================

pub const SEARCH_INPUT_BG: Hsla = hsla(0.0, 0.0, 0.08, 1.0);
pub const SEARCH_INPUT_BORDER: Hsla = hsla(0.0, 0.0, 0.25, 1.0);

/// Leading "🔍" glyph color.
pub const SEARCH_ICON: Hsla = hsla(0.0, 0.0, 0.7, 1.0);

/// Prev/Next/Close button resting color.
pub const SEARCH_BUTTON: Hsla = hsla(0.0, 0.0, 0.65, 1.0);

// ============================================================================
// Search overlay — status / counter labels
// ============================================================================

/// Regex-error banner tint.
pub const SEARCH_LABEL_ERROR: Hsla = hsla(0.0, 0.7, 0.65, 1.0);

/// "no matches" banner tint.
pub const SEARCH_LABEL_EMPTY: Hsla = hsla(0.0, 0.5, 0.65, 1.0);

/// Tint for the empty-query placeholder (faint gray).
pub const SEARCH_LABEL_IDLE: Hsla = hsla(0.0, 0.0, 0.6, 1.0);

/// `focused/total` counter tint (light gray).
pub const SEARCH_LABEL_COUNTER: Hsla = hsla(0.0, 0.0, 0.75, 1.0);

// Every const that used to live in this file but is read only from the
// `app` crate (workspace chrome, modal chrome, banners, settings,
// agent panels, file viewer, right-panel surfaces, toast / error
// report, badges, claude-status pills, etc.) has moved to
// `app/src/ui/theme/palette.rs`. The bridge module at
// `app/src/ui/theme/mod.rs` re-exports both palettes so app-side call
// sites continue to write `theme::FOO` against `crate::ui::theme`.
//
// What stays here: every constant that *terminal-side* widgets in
// this crate (cell rendering, cursor, prompt-mark gutter, scrollback
// search overlay, terminal scrollbar, search bar) read directly. The
// one-way dependency rule (CLAUDE.md §11) forbids importing
// `crate::ui::*` from here, so anything the terminal renderer needs
// must remain locally defined.

// ---- Context menu ----

// ---- TextInput caret + IME composition ----

// ---- TextArea scrollbar ----

// ---- Modal checkbox (Remove worktree, future opt-ins) ----

// ============================================================================
// ValidationBanner — inline severity banner inside form modals
// ============================================================================
//
// Shared by `FormModalShell::error` slot in Skills / Tools / Tasks creation
// modals. Distinct tone from `MODAL_ERROR_TEXT` (text-only inline error in
// `single_field_modal`) — banner has a tinted fill + an icon glyph + a row
// of consistent padding so multi-field forms can spotlight which constraint
// failed without reflowing the form layout.

// ============================================================================
// Settings window
// ============================================================================

// ============================================================================
// Command palette
// ============================================================================

// ============================================================================
// Workspace chrome — pixel metrics
// ============================================================================

// ============================================================================
// Git Changes view (left-dock sidebar panel)
// ============================================================================

// ============================================================================
// Workspace chrome — typography & spacing
// ============================================================================

// ----------------------------------------------------------------
// Bottom dock panels — macro / widget buttons (B-2)
// ----------------------------------------------------------------

// ----------------------------------------------------------------
// Files view (W-7)
// ----------------------------------------------------------------

// ============================================================================
// Agent panels — activity log, task list, chat
// ============================================================================

// ============================================================================
// Dock default sizes (configurable via daruda_config in future)
// ============================================================================

// ============================================================================
// Welcome screen
// ============================================================================

// ============================================================================
// Pane-area file viewer
// ============================================================================

// ============================================================================
// File viewer — search panel
// Panel chrome (bg / border / radius) reuses SEARCH_PANEL_* from the
// terminal search overlay so both panels look identical.
// ============================================================================

// ============================================================================
// File viewer — Markdown preview
// ============================================================================

// ============================================================================
// Terminal text — underline / strikethrough decoration
// ============================================================================

/// Thickness of the underline / strikethrough decoration drawn beneath
/// each glyph (px). Matches the design intent: ~1 device pixel.
pub const TERMINAL_UNDERLINE_THICKNESS: f32 = 1.0;

// ============================================================================
// Terminal cursor — bar / underline metrics + high-contrast colors
// ============================================================================

/// Width of the bar-style cursor (DECSCUSR 5/6) and the inner reverse
/// stripe inside a focused block cursor (px).
pub const CURSOR_BAR_W: f32 = 2.0;

/// Vertical offset of the underline cursor below the line top (px).
pub const CURSOR_UNDERLINE_OFFSET_Y: f32 = 2.0;

/// Extra y-nudge applied to the underline cursor end point (px).
pub const CURSOR_UNDERLINE_END_OFFSET_Y: f32 = 1.0;

/// High-contrast cursor color used against a light cell background.
pub const CURSOR_DARK: Hsla = hsla(0.0, 0.0, 0.0, 1.0);

/// High-contrast cursor color used against a dark cell background.
pub const CURSOR_LIGHT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

// ============================================================================
// Prompt mark gutter (left-side strip + scrollbar tick)
// ============================================================================

/// Width of the prompt-mark gutter strip drawn beside each row (px).
pub const PROMPT_MARK_STRIP_W: f32 = 2.0;

/// Height of the prompt-mark scrollbar tick (px).
pub const PROMPT_MARK_TICK_H: f32 = 2.0;

/// Gutter color: focused row, prompt-mark kinds other than `Prompt`.
pub const PROMPT_MARK_FOCUSED_OTHER: Hsla = hsla(36.0, 0.85, 0.55, 0.75);

/// Gutter color: unfocused row, prompt-mark kinds other than `Prompt`.
pub const PROMPT_MARK_OTHER: Hsla = hsla(50.0, 0.70, 0.50, 0.45);

/// Gutter color: focused row showing a `Prompt` mark.
pub const PROMPT_MARK_FOCUSED_PROMPT: Hsla = hsla(0.0, 0.7, 0.55, 0.9);

/// Gutter color: unfocused row showing OSC 133 PromptStart.
pub const PROMPT_MARK_PROMPT_START: Hsla = hsla(198.0, 0.5, 0.6, 0.7);

/// Gutter color: unfocused row showing OSC 133 CommandFinished.
pub const PROMPT_MARK_COMMAND_FINISHED: Hsla = hsla(119.0, 0.5, 0.55, 0.7);

/// Gutter color: unfocused row showing OSC 133 CommandExecuted.
pub const PROMPT_MARK_COMMAND_EXECUTED: Hsla = hsla(50.0, 0.7, 0.55, 0.75);

/// Scrollbar tick color: focused prompt-mark row.
pub const PROMPT_MARK_TICK_FOCUSED: Hsla = hsla(36.0, 0.85, 0.55, 0.95);

/// Scrollbar tick color: any other prompt-mark row.
pub const PROMPT_MARK_TICK_DEFAULT: Hsla = hsla(50.0, 0.70, 0.50, 0.65);

// ============================================================================
// Terminal viewport overlays
// ============================================================================

/// Background highlight color for an in-viewport scrollback search match.
pub const TERMINAL_SEARCH_HIGHLIGHT: Hsla = hsla(209.0, 0.9, 0.55, 0.35);

/// Full-viewport bell flash tint — white wash with very low alpha so
/// the flash is perceptible without obscuring text.
pub const BELL_FLASH_OVERLAY: Hsla = hsla(0.0, 0.0, 1.0, 0.1);

/// Prompt-jump wrap flash stripe color (drawn at top of viewport).
pub const PROMPT_JUMP_FLASH_STRIPE: Hsla = hsla(198.0, 0.70, 0.60, 0.85);

/// Prompt-jump wrap flash stripe height (px).
pub const PROMPT_JUMP_FLASH_STRIPE_H: f32 = 3.0;

/// Terminal viewport scrollbar thumb fill — white wash with low
/// alpha so the thumb fades against any background but stays
/// distinguishable from the cell content underneath.
pub const TERMINAL_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.3);

// ============================================================================
// Terminal scrollback search bar (overlay panel rendered inside the
// terminal view; separate from the app-crate file viewer search bar)
// ============================================================================

/// Search bar query input text color.
pub const SEARCH_BAR_INPUT_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

/// Search bar prev/next/close button hover text color.
pub const SEARCH_BAR_BUTTON_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

/// Search bar input area inner gap (between caret + glyph + counter, px).
pub const SEARCH_BAR_INPUT_GAP: f32 = 6.0;

/// Search bar input area corner radius (px).
pub const SEARCH_BAR_INPUT_RADIUS: f32 = 4.0;

/// Search bar caret indicator width (px).
pub const SEARCH_BAR_CARET_W: f32 = 1.0;

/// Search bar caret indicator height (px).
pub const SEARCH_BAR_CARET_H: f32 = 14.0;

/// Search bar match counter font size (px).
pub const SEARCH_BAR_COUNTER_FONT_SIZE: f32 = 11.0;

/// Search bar button horizontal padding (px).
pub const SEARCH_BAR_BUTTON_PAD_X: f32 = 6.0;

/// Search bar close button left margin (px) — visually separates it
/// from the prev / next nav buttons.
pub const SEARCH_BAR_CLOSE_ML: f32 = 4.0;

// ============================================================================
// Workspace render layout — structural minima
// ============================================================================

// ============================================================================
// Toast layer
// ============================================================================

// ============================================================================
// Toast severity tints (icon + accent for the leading bar / glyph)
// ============================================================================

// ============================================================================
// Error-report Details modal
// ============================================================================

// ============================================================================
// Terminal canvas background / foreground defaults
// ============================================================================

/// Default terminal text color (white; overridden per cell by ghostty_vt style runs).
pub const TERMINAL_FG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

// ============================================================================
// Terminal scrollbar (TERMINAL_SCROLLBAR_*)
// ============================================================================

/// Width of the terminal scrollbar track (px).
pub const TERMINAL_SCROLLBAR_W: f32 = 6.0;
/// Horizontal margin between the scrollbar and the terminal right edge (px).
pub const TERMINAL_SCROLLBAR_MARGIN: f32 = 2.0;
/// Thumb default color (idle).
// TERMINAL_SCROLLBAR_THUMB (existing) covers this; defined at line ~1185
/// Thumb active (hover / drag) color — brighter than idle.
pub const TERMINAL_SCROLLBAR_THUMB_ACTIVE: Hsla = hsla(0.0, 0.0, 1.0, 0.50);
/// Minimum thumb height so it stays grabbable (px).
pub const TERMINAL_SCROLLBAR_THUMB_MIN_H: f32 = 20.0;

// ============================================================================
// Keystroke input widget
// ============================================================================

// ============================================================================
// Popover menu
// ============================================================================

// ----------------------------------------------------------------
// Drag ghost pill (sidebar path drag)
// ----------------------------------------------------------------

// ============================================================================
// Claude Code status indicator (sidebar worktree row)
// ============================================================================
// 4 states (Working / NeedsAttention / Idle / Connecting). Mockup:
// `Claude-Code-Status-Indicator-Mockup.html`. Palette A — classic
// semantic.

// ---- Palette A — light theme ----
// Working: #3aa0ff
// ExecutingTool: #d97706 (amber-600)
// NeedsAttention: #dc2626
// Idle: #22c55e
// Connecting: #6b7280

// ---- Palette A — dark theme ----
// Working: #5eb6ff
// ExecutingTool: #fbbf24 (amber-400)
// NeedsAttention: #ef4444
// Idle: #34d399
// Connecting: #9ca3af

// ---- Phase D — sub-row per-session badge strip ----
// Rendered beneath the sublabel when a worktree has ≥ 2 active Claude
// sessions. The leading indicator still shows the aggregate priority;
// these badges drill into the individual sessions.

// ---- Phase E — active session badge outline ----
// 1 px ring drawn outside the badge for the session attached to the
// focused tab. Distinguishes "this is what your terminal is talking
// to" from sibling sessions in the same cwd.

// ---- Install banner ("Claude integration disabled") ----

// ============================================================================
// Right panel (Usage / Skills / Tools / Tasks tabs in the right dock)
// ============================================================================

// ---- Tasks tab (right_panel/tasks.rs) ----

// ============================================================================
// Status pill (R-26)
// ============================================================================
//
// The pill is the trailing-edge dropdown trigger on each task row.
// Its visual treatment carries one signal — the task's current state —
// using a tinted background plus the existing state colour for the
// label. Numeric constants live here so the right-panel renderer and
// any future status pill test fixture pull from a single source.

// ============================================================================
// Generic font family names
// ============================================================================

// ============================================================================
// Generic Badge widget (`ui::Badge`)
// ============================================================================

// ============================================================================
// Generic Divider widget (`ui::Divider`)
// ============================================================================

// ============================================================================
// Right Panel — gauges + service-status pill (R-4 / R-5)
// ============================================================================
//
// Colors mirror Apple's San Francisco "vivid" palette as used in the
// Übersicht widget the Usage tab is modelled on (`#34C759` /
// `#FFD60A` / `#FF453A` / `#FF9F0A`). HSL conversions are exact —
// staying numerically aligned with the reference widget keeps the
// "5h gauge turned yellow" visual cue identical for users coming
// from there.

// ----------------------------------------------------------------
// Skills tab — invocation badges, aux chip, row spacing
// ----------------------------------------------------------------

// ============================================================================
// Right panel — Tools tab (MCP servers)
// ============================================================================

// ============================================================================
// Structural helpers
// ============================================================================

/// Construct an algorithmic monochrome (grayscale) color from a
/// runtime lightness + alpha. Matches `gpui::hsla(0, 0, l, a)` but
/// names the "no hue, no saturation" semantic so the call site
/// doesn't have to embed the literal definition of grayscale.
pub fn monochrome(l: f32, a: f32) -> Hsla {
    Hsla {
        h: 0.0,
        s: 0.0,
        l,
        a,
    }
}
