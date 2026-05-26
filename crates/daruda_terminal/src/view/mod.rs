mod actions;
mod bg_merge;
mod box_drawing;
pub(crate) mod element;
mod hangul_composer;
mod ime;
mod input;
mod jamo;
pub(crate) mod jump;
pub mod layout;
mod mouse;
mod output;
mod overlay;
mod render;
mod search;
mod search_bar;
pub(super) mod selection;
mod selection_policy;
pub(crate) mod state;
mod style;
mod text_edit;
pub(crate) mod text_metrics;
mod url;
mod viewport;

use super::TerminalSession;
use gpui::{App, Context, FocusHandle, KeyBinding, Pixels, SharedString, Subscription, actions};
use selection::{
    ByteSelection, CellAnchor, SelectionMode, Side, block_copy_text, pixel_to_cell_anchor,
    selection_mode_from_modifiers,
};
use std::sync::Once;

// Re-exports used by sibling modules / external app
pub use layout::TerminalLayout;
#[cfg(test)]
pub(crate) use text_metrics::byte_index_for_column_in_line;

// Crate-level test helpers
#[cfg(test)]
pub(crate) use box_drawing::{box_drawing_mask, line_has_box_drawing};
#[cfg(test)]
pub(crate) use input::{ctrl_byte_for_keystroke, should_skip_key_down_for_ime};
#[cfg(test)]
pub(crate) use mouse::{sgr_mouse_button_value, sgr_mouse_sequence};

actions!(
    terminal_view,
    [
        Copy,
        Paste,
        SelectAll,
        Tab,
        TabPrev,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ToggleFullscreen,
        ClearBuffer,
        ClearScrollback,
        SearchOpen,
        SearchClose,
        SearchNext,
        SearchPrev,
        SearchBackspace,
        PromptJumpPrev,
        PromptJumpNext,
        CommandJumpPrev,
        CommandJumpNext,
        CopyLastCommandOutput,
        SearchCursorLeft,
        SearchCursorRight,
        SearchCursorHome,
        SearchCursorEnd,
        SearchDeleteForward,
        SearchClearQuery,
        SearchToggleRegex,
    ]
);

const KEY_CONTEXT: &str = "Terminal";
static KEY_BINDINGS: Once = Once::new();

fn ensure_key_bindings(cx: &mut App) {
    KEY_BINDINGS.call_once(|| {
        cx.bind_keys([
            KeyBinding::new("tab", Tab, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-tab", TabPrev, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-=", ZoomIn, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd--", ZoomOut, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-0", ResetZoom, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-ctrl-f", ToggleFullscreen, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-k", ClearBuffer, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-k", ClearScrollback, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-f", SearchOpen, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-g", SearchNext, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-g", SearchPrev, Some(KEY_CONTEXT)),
            KeyBinding::new("escape", SearchClose, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("enter", SearchNext, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("shift-enter", SearchPrev, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("backspace", SearchBackspace, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("delete", SearchDeleteForward, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("left", SearchCursorLeft, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("right", SearchCursorRight, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("home", SearchCursorHome, Some(SEARCH_KEY_CONTEXT)),
            KeyBinding::new("end", SearchCursorEnd, Some(SEARCH_KEY_CONTEXT)),
            // iTerm2 convention — Cmd+Up / Cmd+Down conflict with macOS
            // document-navigation defaults on some focused elements, so
            // use the shift variant to match iTerm2's "Mark Navigation".
            KeyBinding::new("cmd-shift-up", PromptJumpPrev, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-down", PromptJumpNext, Some(KEY_CONTEXT)),
            // Navigate between executed commands (FTCS C boundaries).
            KeyBinding::new("cmd-shift-alt-up", CommandJumpPrev, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-alt-down", CommandJumpNext, Some(KEY_CONTEXT)),
            // Copy the last command's output (FTCS E/F or C/D fallback).
            KeyBinding::new("cmd-shift-c", CopyLastCommandOutput, Some(KEY_CONTEXT)),
        ]);
    });
}

const SEARCH_KEY_CONTEXT: &str = "TerminalSearch";

// ---------------------------------------------------------------------------
// TerminalInput
// ---------------------------------------------------------------------------

type TerminalSendFn = dyn Fn(&[u8]) + Send + Sync + 'static;

pub struct TerminalInput {
    send: Box<TerminalSendFn>,
}

impl TerminalInput {
    pub fn new(send: impl Fn(&[u8]) + Send + Sync + 'static) -> Self {
        Self {
            send: Box::new(send),
        }
    }

    pub fn send(&self, bytes: &[u8]) {
        (self.send)(bytes);
    }
}

// ---------------------------------------------------------------------------
// TerminalView — core struct
// ---------------------------------------------------------------------------

pub struct TerminalView {
    pub(super) session: TerminalSession,
    pub(super) line_layouts: Vec<Option<gpui::ShapedLine>>,
    /// `(font_size, line_height, cell_width, font_hash)` — shape cache key.
    /// Every axis that affects glyph geometry must appear here; missing one
    /// turns a zoom or font change into a stale-shape bug for existing rows.
    pub(super) line_layout_key: Option<(Pixels, Pixels, Pixels, u64)>,
    pub(super) focus_handle: FocusHandle,
    pub(super) input: Option<TerminalInput>,
    /// Shared paint+event data. Carries every field that both the
    /// renderer and the event path read or mutate (Pitfall #8 quartet,
    /// search, selection, viewport snapshot, IME, jump). The entity
    /// is left with only GPUI-bound resources (subscriptions, tasks)
    /// and the paint-only shape cache (`line_layouts`).
    pub(super) state: state::TerminalViewState,
    /// Task handle for the auto-scroll loop.  Dropping it cancels the
    /// loop; set to `None` to stop.
    pub(super) autoscroll_task: Option<gpui::Task<()>>,
    /// Focus-in subscription for DECSET 1004 reporting. Kept alive by
    /// the struct; dropped on `Drop`.
    _focus_in_sub: Option<Subscription>,
    /// Focus-out (blur) subscription for DECSET 1004 reporting.
    _focus_out_sub: Option<Subscription>,
    /// Keyboard-layout subscription: macOS silently swallows the input-source
    /// switch shortcut, so we subscribe to reset the Hangul composer before
    /// the next keystroke would inherit the stale jamo.
    _keyboard_layout_sub: Option<Subscription>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HoveredUrl {
    pub(super) row: u16,       // 0-indexed viewport row
    pub(super) start_col: u16, // 1-indexed inclusive
    pub(super) end_col: u16,   // 1-indexed inclusive
    pub(super) url: SharedString,
}

pub(crate) use layout::font_hash;
pub use search::SearchState;

// ---------------------------------------------------------------------------
// TerminalView — core implementation
// ---------------------------------------------------------------------------

impl TerminalView {
    pub fn new(session: TerminalSession, focus_handle: FocusHandle) -> Self {
        Self::build(session, focus_handle, None)
    }

    /// Title reported by the terminal (OSC-2 window title), used by both
    /// the workspace tab bar and per-pane headers in split mode. Falls back
    /// to "shell" before any title sequence is seen.
    pub fn terminal_title(&self) -> &str {
        self.state
            .last_window_title
            .as_deref()
            .unwrap_or(crate::ux::strings::FALLBACK_TITLE)
    }

    /// Current working directory as reported by the shell (OSC 7).
    /// `None` until the shell emits its first OSC 7 (or when
    /// `TerminalConfig::track_cwd` is disabled).
    pub fn terminal_cwd(&self) -> Option<&str> {
        self.session.cwd()
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// `true` when a text selection is active (linear or block mode).
    pub fn has_selection(&self) -> bool {
        self.state.selection.is_some()
    }

    /// Terminal body font size in points. Updated by the zoom
    /// actions; readable so the surrounding app (workspace resize,
    /// settings UI) can observe the current value.
    pub fn font_size(&self) -> f32 {
        self.state.font_size
    }

    /// Line height multiplier (iTerm2 `KEY_VERTICAL_SPACING`).
    pub fn vertical_spacing(&self) -> f32 {
        self.state.vertical_spacing
    }

    /// Cell width multiplier (iTerm2 `KEY_HORIZONTAL_SPACING`).
    pub fn horizontal_spacing(&self) -> f32 {
        self.state.horizontal_spacing
    }

    /// Current primary font (read-only). Needed by external callers (e.g.
    /// workspace resize) so they measure cell metrics with the same font
    /// the renderer uses, not a hardcoded default.
    pub fn font(&self) -> &gpui::Font {
        &self.state.font
    }

    pub fn new_with_input(
        session: TerminalSession,
        focus_handle: FocusHandle,
        input: TerminalInput,
    ) -> Self {
        Self::build(session, focus_handle, Some(input))
    }

    /// Send raw bytes to the PTY exactly as if the user had typed
    /// them on the keyboard. No-op when the view was constructed
    /// without a PTY backing (test stubs). Used by external dispatchers
    /// like the bottom-dock macro buttons.
    pub fn send_input(&self, bytes: &[u8]) {
        if let Some(input) = &self.input {
            input.send(bytes);
        }
    }

    /// Shared constructor — the only place that enumerates the full
    /// `TerminalView` field set. `new` / `new_with_input` just pick
    /// whether an input sink is wired up. Keeping a single struct
    /// literal prevents default-value drift (a field added in one
    /// constructor but not the other).
    fn build(
        session: TerminalSession,
        focus_handle: FocusHandle,
        input: Option<TerminalInput>,
    ) -> Self {
        let state = state::TerminalViewState::new(
            crate::default_terminal_font(),
            session.font_size(),
            session.vertical_spacing(),
            session.horizontal_spacing(),
            session.background_alpha(),
        );
        Self {
            session,
            line_layouts: Vec::new(),
            line_layout_key: None,
            focus_handle,
            input,
            state,
            autoscroll_task: None,
            _focus_in_sub: None,
            _focus_out_sub: None,
            _keyboard_layout_sub: None,
        }
        .with_refreshed_viewport()
    }

    /// Update the search query and recompute matches against the
    /// current viewport. Empty query clears state. `is_regex = true`
    /// compiles `query` as a regex; compile failures surface through
    /// `search_state().regex_error()` with no matches.
    pub fn set_search_query(
        &mut self,
        query: &str,
        case_insensitive: bool,
        is_regex: bool,
        cx: &mut Context<Self>,
    ) {
        self.state.search.query = query.to_string();
        self.state.search.case_insensitive = case_insensitive;
        self.state.search.is_regex = is_regex;
        self.recompute_search_matches();
        cx.notify();
    }

    /// Advance the focused match to the next (forward) or previous one.
    /// Wraps around. No-op when there are no matches. When the target
    /// match lies outside the current viewport the viewport is
    /// scrolled to bring it into view.
    pub fn search_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.state.search.matches.is_empty() {
            self.state.search.focused = None;
            return;
        }
        let len = self.state.search.matches.len();
        let next = match self.state.search.focused {
            None => 0,
            Some(i) if forward => (i + 1) % len,
            Some(i) => (i + len - 1) % len,
        };
        self.state.search.focused = Some(next);

        let target_row = self.state.search.matches[next].row;
        self.scroll_to_screen_row(target_row);
        self.state.pending_refresh = true;
        cx.notify();
    }

    /// Clear search state and its highlights.
    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.state.search = SearchState::default();
        cx.notify();
    }

    /// Read-only view of the current search state.
    pub fn search_state(&self) -> &SearchState {
        &self.state.search
    }

    /// Scroll the viewport so an absolute screen row sits at the top.
    pub fn jump_to_screen_row_top(&mut self, row: u32, cx: &mut Context<Self>) {
        self.scroll_screen_row_to_top(row);
        cx.notify();
    }

    /// Read-only access to the session for callers that need to walk
    /// command history, prompt marks, etc.
    pub fn session(&self) -> &TerminalSession {
        &self.session
    }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Side-channel events surfaced by a [`TerminalView`] for the host
/// (Workspace) to dispatch into platform APIs that the view itself
/// must not depend on directly. Kept narrow on purpose — clipboard
/// writes still go through `cx.write_to_clipboard` because GPUI
/// already abstracts that platform; events appear here only when
/// the GPUI core has no equivalent.
#[derive(Debug, Clone)]
pub enum TerminalViewEvent {
    /// `OSC 1337 ; RequestAttention=…` arrived. Workspace bounces
    /// the dock / cancels prior request via AppKit, gated by
    /// `daruda_config::NotificationsConfig::attention_enabled`.
    AttentionRequested(crate::vt_codes::AttentionKind),
    /// `OSC 9` or `OSC 777 ; notify ; …` arrived. Workspace surfaces
    /// it as a desktop notification, gated by `osc9_enabled` /
    /// `osc777_enabled` and `skip_focused_pane`.
    NotificationRequested(crate::vt_codes::NotificationRequest),
    /// FTCS D arrived after a CommandStart. Workspace compares
    /// `elapsed` to `long_running_threshold_secs` and surfaces a
    /// "command finished" notification when the threshold is
    /// crossed, gated by `long_running_enabled` and
    /// `skip_focused_pane`.
    CommandFinishedAfter { elapsed: std::time::Duration },
}

impl gpui::EventEmitter<TerminalViewEvent> for TerminalView {}

#[cfg(test)]
mod tests;
