//! Terminal mode flags, config getters, buffer accessors, and one-shot
//! event drains.

use super::*;

impl TerminalSession {
    pub fn cols(&self) -> u16 {
        self.dims.cols
    }

    pub fn rows(&self) -> u16 {
        self.dims.rows
    }

    pub fn default_foreground(&self) -> Rgb {
        self.config.default_fg
    }

    pub fn default_background(&self) -> Rgb {
        self.config.default_bg
    }

    pub fn bracketed_paste_enabled(&self) -> bool {
        self.bracketed_paste_enabled
    }

    pub fn focus_event_enabled(&self) -> bool {
        self.focus_event_enabled
    }

    pub fn mouse_reporting_enabled(&self) -> bool {
        self.mouse_x10_enabled || self.mouse_button_event_enabled || self.mouse_any_event_enabled
    }

    /// True when X10 press-only mode (1000) is active and neither
    /// button-event (1002) nor any-event (1003) mode is also set.
    /// X10 must not generate release or motion events.
    pub fn mouse_x10_only(&self) -> bool {
        self.mouse_x10_enabled && !self.mouse_button_event_enabled && !self.mouse_any_event_enabled
    }

    pub fn mouse_sgr_enabled(&self) -> bool {
        self.mouse_sgr_enabled
    }

    pub fn mouse_button_event_enabled(&self) -> bool {
        self.mouse_button_event_enabled
    }

    pub fn mouse_any_event_enabled(&self) -> bool {
        self.mouse_any_event_enabled
    }

    pub fn mouse_utf8_enabled(&self) -> bool {
        self.mouse_utf8_enabled
    }

    pub fn alternate_scroll_enabled(&self) -> bool {
        self.alternate_scroll_enabled
    }

    pub fn decckm_enabled(&self) -> bool {
        self.decckm_enabled
    }

    pub fn decnkm_enabled(&self) -> bool {
        self.decnkm_enabled
    }

    pub fn synchronized_output_enabled(&self) -> bool {
        self.synchronized_output
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Current working directory reported by the shell (OSC 7).
    /// Returns the local path (without `file://hostname` prefix).
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub(crate) fn window_title_updates_enabled(&self) -> bool {
        self.config.update_window_title
    }

    pub(crate) fn visual_bell_enabled(&self) -> bool {
        self.config.visual_bell
    }

    pub(crate) fn prompt_jump_scroll_mode(&self) -> crate::config::PromptJumpScroll {
        self.config.prompt_jump_scroll
    }

    pub(crate) fn natural_text_editing(&self) -> bool {
        self.config.natural_text_editing
    }

    pub(crate) fn copy_on_select(&self) -> bool {
        self.config.copy_on_select
    }

    /// Push the config-owned input behaviour into a running session: each
    /// setting is read per keystroke, chunk or selection, so a change applies
    /// from the next one.
    pub fn apply_input_settings(&mut self, settings: crate::InputSettings) {
        self.config.natural_text_editing = settings.natural_text_editing;
        self.config.osc1337_max_bytes = settings.osc1337_max_bytes;
        self.config.copy_on_select = settings.copy_on_select;
    }

    /// Initial font point size. Used by `TerminalView` at construction
    /// before any runtime zoom action.
    pub fn font_size(&self) -> f32 {
        self.config.font_size
    }

    /// Line height multiplier (iTerm2 `KEY_VERTICAL_SPACING`, 0.5–2.0).
    pub fn vertical_spacing(&self) -> f32 {
        self.config.vertical_spacing
    }

    /// Cell width multiplier (iTerm2 `KEY_HORIZONTAL_SPACING`, 0.5–2.0).
    pub fn horizontal_spacing(&self) -> f32 {
        self.config.horizontal_spacing
    }

    /// Background opacity (0.0–1.0). Mirrors iTerm2 `transparencyAlpha`.
    pub fn background_alpha(&self) -> f32 {
        self.config.background_alpha
    }

    /// Horizontal pane inset in pixels (iTerm2 `TerminalMargin`). Used by
    /// `TerminalView` at construction before any runtime config reload.
    pub fn inset_x(&self) -> f32 {
        self.config.inset_x
    }

    /// Vertical pane inset in pixels (iTerm2 `TerminalVMargin`).
    pub fn inset_y(&self) -> f32 {
        self.config.inset_y
    }

    /// Cursor shape used while the program has not set one.
    pub fn default_cursor_shape(&self) -> crate::CursorShape {
        self.config.default_cursor_shape
    }

    pub fn hyperlink_at(&self, col: u16, row: u16) -> Option<String> {
        self.terminal.hyperlink_at(col, row)
    }

    pub fn take_clipboard_write(&mut self) -> Option<String> {
        self.clipboard_write.take()
    }

    /// Drain the pending macOS user-attention request (`OSC 1337 ;
    /// RequestAttention=…`). Returns `None` if no new sequence has
    /// arrived since the last call.
    pub fn take_attention_request(&mut self) -> Option<AttentionKind> {
        self.pending_attention.take()
    }

    /// Drain the pending desktop notification request from OSC 9 / 777.
    /// Returns `None` if no notification sequence has arrived since
    /// the last call.
    pub fn take_notification_request(&mut self) -> Option<NotificationRequest> {
        self.pending_notification.take()
    }

    /// Drain the elapsed time of the most recently finished command
    /// (FTCS D). Returns `None` if no CommandFinished mark has arrived
    /// since the last call, or if the command was not framed by a
    /// preceding CommandStart. Workspace applies the threshold +
    /// focus gates before surfacing this as a notification.
    pub fn take_finished_command_elapsed(&mut self) -> Option<Duration> {
        self.pending_finished_command_elapsed.take()
    }

    /// Borrow the logical-line scrollback buffer. Consumers wrap it
    /// against the live cell width at render time.
    pub fn line_buffer(&self) -> &LineBuffer {
        &self.line_buffer
    }

    /// Test-only direct access to the interval tree for coordinate
    /// inspection after internal operations (e.g. after capture_scrolled_out
    /// rebinds viewport marks to buffered positions).
    #[cfg(test)]
    pub(crate) fn interval_tree(&self) -> &IntervalTree<MarkPayload> {
        &self.interval_tree
    }

    /// DECRQM parameter value for the given private mode number.
    ///
    /// Returns: `0` = not recognised, `1` = set, `2` = reset.
    pub(super) fn decrqm_pm(&self, ps: u32) -> u8 {
        let enabled = match ps {
            1 => self.decckm_enabled,
            // DECAWM (auto-wrap): ghostty_vt tracks this internally but
            // the Rust FFI does not expose a getter. Returning "set" (1)
            // matches the power-on default; apps that send `CSI ?7 l` will
            // receive a stale answer. TODO: expose via ghostty_vt FFI.
            7 => true,
            25 => self.terminal.cursor_visible(),
            66 => self.decnkm_enabled,
            1000 => self.mouse_x10_enabled,
            1002 => self.mouse_button_event_enabled,
            1003 => self.mouse_any_event_enabled,
            1004 => self.focus_event_enabled,
            1005 => self.mouse_utf8_enabled,
            1006 => self.mouse_sgr_enabled,
            1007 => self.alternate_scroll_enabled,
            47 | 1047 | 1049 => self.alt_screen,
            2004 => self.bracketed_paste_enabled,
            2026 => self.synchronized_output,
            _ => return 0,
        };
        if enabled { 1 } else { 2 }
    }
}
