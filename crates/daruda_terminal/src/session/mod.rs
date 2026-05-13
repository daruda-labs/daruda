use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ghostty_vt::{Error, Rgb, Terminal};

use crate::TerminalConfig;
use crate::ansi;
use crate::vt_codes::{
    AttentionKind, CSI_ALT_SCREEN, CSI_ALT_SCREEN_LEGACY, CSI_ALT_SCREEN_SAVE_CURSOR, CsiMode,
    NotificationRequest, OSC_DEFAULT_BG, OSC_DEFAULT_FG,
};
use crate::vt_limits::{PARSE_TAIL_LIMIT, PROMPT_MARKS_CAP};

mod scanners;
#[cfg(test)]
use scanners::parse_osc133_payload;
use scanners::{
    CapabilityQuery, CapabilityScanner, DsrScanState, Osc133Scanner, OscDispatch, OscQuery,
    OscQueryScanState, TerminalQuery, XtGetTcapScanner, build_xtgettcap_response,
    commit_osc_payload,
};

#[cfg(test)]
mod tests;

/// One completed shell command surfaced by walking `prompt_marks`.
/// Built from a `(CommandStart, CommandExecuted)` pair and, when
/// available, a following `CommandFinished`. Each entry carries the
/// command text the user typed, the absolute screen row of the
/// `CommandStart` mark (for jump-to-command in scrollback), and the
/// exit code from `D` if observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandHistoryEntry {
    pub command_text: String,
    pub start_row: u32,
    pub exit_code: Option<i32>,
}

/// One semantic boundary reported by a shell that speaks FinalTerm / OSC
/// 133. Captured with the absolute screen row it landed on so jumps
/// work even after the mark scrolls out of the viewport. Mirrors
/// iTerm2 `VT100Terminal.m:4520-4616`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptMark {
    pub kind: PromptMarkKind,
    /// 0-indexed row in the screen coordinate space (scrollback +
    /// active). Use `TerminalSession::viewport_row_offset` to translate
    /// to viewport coordinates for rendering.
    pub screen_row: u32,
    /// 1-indexed cursor column at the moment the mark fired. Captured
    /// alongside `screen_row` so the command-history extractor can
    /// slice the typed command out of `[B.col .. C.col]` on the
    /// command's row(s) without re-deriving prompt-prefix length from
    /// the grid.
    pub screen_col: u16,
    /// Exit code attached to `CommandFinished` (FTCS D). `None` for A/B/C.
    pub exit_code: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptMarkKind {
    /// FTCS A — prompt drawing started.
    PromptStart,
    /// FTCS B — user command input begins.
    CommandStart,
    /// FTCS C — command has been sent to the shell and is executing.
    CommandExecuted,
    /// FTCS D — command finished, optionally with an exit code.
    CommandFinished,
    /// FTCS E — semantic text block begins (usually command output).
    /// Used by iTerm2's "Copy command output" — delimits the region
    /// emitted between `C` and `D`.
    SemanticTextStart,
    /// FTCS F — semantic text block ends.
    SemanticTextEnd,
}

pub struct TerminalSession {
    config: TerminalConfig,
    terminal: Terminal,
    bracketed_paste_enabled: bool,
    focus_event_enabled: bool,
    mouse_x10_enabled: bool,
    mouse_button_event_enabled: bool,
    mouse_any_event_enabled: bool,
    mouse_sgr_enabled: bool,
    mouse_utf8_enabled: bool,
    /// When true, wheel scroll in alt-screen emits arrow-key sequences instead
    /// of scroll sequences.  Enabled by DECSET 1007; defaults to true
    /// (matches Alacritty / most modern terminals).
    alternate_scroll_enabled: bool,
    /// Application Cursor Keys mode (DECCKM, mode 1). When enabled, arrow
    /// keys emit `\x1bOA`–`\x1bOD` (application) sequences.
    decckm_enabled: bool,
    /// Application Keypad Mode (DECNKM, mode 66). When enabled, numeric
    /// keypad keys emit application sequences.
    decnkm_enabled: bool,
    /// Synchronized Output mode (BSynchronized, mode 2026). Apps toggle this
    /// to bracket full-frame updates and prevent tearing.
    synchronized_output: bool,
    title: Option<String>,
    /// Current working directory reported by the shell via OSC 7.
    /// Format from shell: `file://hostname/path`. We strip the URI prefix
    /// and store just the local path. None until first OSC 7 sequence.
    cwd: Option<String>,
    clipboard_write: Option<String>,
    parse_tail: Vec<u8>,
    dsr_state: DsrScanState,
    osc_query_state: OscQueryScanState,
    osc133_scanner: Osc133Scanner,
    capability_scanner: CapabilityScanner,
    xtgettcap_scanner: XtGetTcapScanner,
    prompt_marks: VecDeque<PromptMark>,
    /// Whether the terminal is currently in alt-screen mode (DECSET 47/1047/1049).
    alt_screen: bool,
    /// Set whenever alt-screen state transitions; consumed by
    /// `take_screen_changed()`.
    screen_changed: bool,
    /// Set when a PromptStart (OSC 133 A) mark is received. Consumed by
    /// `take_prompt_arrived()` to trigger a scroll-to-bottom in TerminalView.
    prompt_arrived: bool,
    /// Pending macOS user-attention request from `OSC 1337 ; RequestAttention=…`.
    /// Drained by `take_attention_request()`.
    pending_attention: Option<AttentionKind>,
    /// Pending desktop notification from OSC 9 / OSC 777. Drained by
    /// `take_notification_request()`.
    pending_notification: Option<NotificationRequest>,
    /// Wall-clock start of the current command — set on FTCS B
    /// (CommandStart), cleared on FTCS D (CommandFinished) and on the
    /// next FTCS A (PromptStart) so an aborted command (Ctrl-C) does
    /// not leak its start time into the next one.
    command_started_at: Option<Instant>,
    /// Elapsed time of the most recent CommandFinished. Drained by
    /// `take_finished_command_elapsed()`. Workspace decides whether to
    /// surface it as a notification based on its threshold + focus
    /// gates.
    pending_finished_command_elapsed: Option<Duration>,
    /// `OSC 1337 ; ClearScrollback` was observed in the most recent
    /// chunk. `feed` drains the flag *after* `feed_incremental` so the
    /// chunk's own pre-clear output (e.g. cursor positioning the shell
    /// emits before clearing) lands on screen first.
    pending_clear_scrollback: bool,
}

impl TerminalSession {
    pub fn new(config: TerminalConfig) -> Result<Self, Error> {
        let mut terminal =
            Terminal::with_scrollback(config.cols, config.rows, config.max_scrollback)?;
        terminal.set_default_colors(config.default_fg, config.default_bg);
        if let Some(palette) = &config.palette {
            for (i, [r, g, b]) in palette.iter().enumerate() {
                let seq = crate::ansi::osc4_set_color(i as u8, *r, *g, *b);
                let _ = terminal.feed(seq.as_bytes());
            }
        }
        Ok(Self {
            config,
            terminal,
            bracketed_paste_enabled: false,
            focus_event_enabled: false,
            mouse_x10_enabled: false,
            mouse_button_event_enabled: false,
            mouse_any_event_enabled: false,
            mouse_sgr_enabled: false,
            mouse_utf8_enabled: false,
            alternate_scroll_enabled: true,
            decckm_enabled: false,
            decnkm_enabled: false,
            synchronized_output: false,
            title: None,
            cwd: None,
            clipboard_write: None,
            parse_tail: Vec::new(),
            dsr_state: DsrScanState::default(),
            osc_query_state: OscQueryScanState::default(),
            osc133_scanner: Osc133Scanner::default(),
            capability_scanner: CapabilityScanner::default(),
            xtgettcap_scanner: XtGetTcapScanner::default(),
            prompt_marks: VecDeque::new(),
            alt_screen: false,
            screen_changed: false,
            prompt_arrived: false,
            pending_attention: None,
            pending_notification: None,
            command_started_at: None,
            pending_finished_command_elapsed: None,
            pending_clear_scrollback: false,
        })
    }

    /// Push updated default colors and ANSI palette to the running terminal.
    /// Uses the same mechanism as initialisation: feeds OSC 4 sequences so
    /// ghostty_vt's colour tables update without requiring a restart.
    /// Changes appear on the next dirty-row flush.
    pub fn apply_colors(&mut self, fg: Rgb, bg: Rgb, palette: &[[u8; 3]; 16]) {
        self.terminal.set_default_colors(fg, bg);
        for (i, &[r, g, b]) in palette.iter().enumerate() {
            let seq = crate::ansi::osc4_set_color(i as u8, r, g, b);
            let _ = self.terminal.feed(seq.as_bytes());
        }
        self.config.default_fg = fg;
        self.config.default_bg = bg;
        self.config.palette = Some(*palette);
    }

    /// Returns whether the terminal is currently in alt-screen mode.
    pub fn is_alt_screen(&self) -> bool {
        self.alt_screen
    }

    /// Consumes and returns the pending alt-screen state change, if any.
    /// `Some(true)` = entered alt-screen; `Some(false)` = exited;
    /// `None` = no change since the last call.
    pub fn take_screen_changed(&mut self) -> Option<bool> {
        if self.screen_changed {
            self.screen_changed = false;
            Some(self.alt_screen)
        } else {
            None
        }
    }

    /// Recently observed FinalTerm / OSC 133 boundaries, oldest first,
    /// bounded to ~1024 entries.
    pub fn prompt_marks(&self) -> &VecDeque<PromptMark> {
        &self.prompt_marks
    }

    fn push_prompt_mark(&mut self, mark: PromptMark) {
        match mark.kind {
            PromptMarkKind::PromptStart => {
                self.prompt_arrived = true;
                // A new prompt without an intervening CommandFinished
                // means the previous command was aborted (Ctrl-C, kill,
                // shell exit). Drop the orphaned start so the next
                // command's elapsed measurement starts fresh.
                self.command_started_at = None;
            }
            PromptMarkKind::CommandStart => {
                self.command_started_at = Some(Instant::now());
            }
            PromptMarkKind::CommandFinished => {
                if let Some(start) = self.command_started_at.take() {
                    self.pending_finished_command_elapsed = Some(start.elapsed());
                }
            }
            _ => {}
        }
        if self.prompt_marks.len() >= PROMPT_MARKS_CAP {
            self.prompt_marks.pop_front();
        }
        self.prompt_marks.push_back(mark);
    }

    /// Consume the pending prompt-arrival signal set when a PromptStart
    /// (OSC 133 A) mark is received.  Returns `true` once per new prompt.
    pub fn take_prompt_arrived(&mut self) -> bool {
        std::mem::take(&mut self.prompt_arrived)
    }

    /// Return the row range of the most recent complete command output,
    /// derived from the last FTCS `E`…`F` pair (or `C`…`D` fallback).
    /// Returns `None` if no closed block has been observed.
    pub fn last_command_output_rows(&self) -> Option<std::ops::Range<u32>> {
        // Walk backwards so we always pick the most recent complete
        // pair. Prefer the semantic E/F pair (output body only) over
        // the C/D pair (which includes the command echo itself).
        let mut end_row: Option<u32> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::SemanticTextEnd if end_row.is_none() => {
                    end_row = Some(mark.screen_row);
                }
                PromptMarkKind::SemanticTextStart => {
                    if let Some(end) = end_row {
                        return Some(mark.screen_row..end);
                    }
                }
                _ => {}
            }
        }
        // Fallback: the C/D pair captures the whole command-through-
        // output region when the shell doesn't emit E/F.
        let mut end_row: Option<u32> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::CommandFinished if end_row.is_none() => {
                    end_row = Some(mark.screen_row);
                }
                PromptMarkKind::CommandExecuted => {
                    if let Some(end) = end_row {
                        return Some(mark.screen_row..end);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Walk `prompt_marks` for completed shell commands. Each
    /// `(CommandStart, CommandExecuted)` pair becomes one entry; a
    /// following `CommandFinished` populates the exit code. Entries
    /// are returned oldest-first so a UI consumer can render the
    /// natural top-down or reverse it for "most recent first".
    pub fn command_history(&self) -> Vec<CommandHistoryEntry> {
        let mut entries: Vec<CommandHistoryEntry> = Vec::new();
        let mut last_b: Option<PromptMark> = None;
        for mark in &self.prompt_marks {
            match mark.kind {
                PromptMarkKind::CommandStart => {
                    last_b = Some(*mark);
                }
                PromptMarkKind::CommandExecuted => {
                    let Some(b) = last_b.take() else {
                        continue;
                    };
                    let text = self.extract_command_text(&b, mark);
                    if text.trim().is_empty() {
                        continue;
                    }
                    entries.push(CommandHistoryEntry {
                        command_text: text,
                        start_row: b.screen_row,
                        exit_code: None,
                    });
                }
                PromptMarkKind::CommandFinished => {
                    if let Some(last) = entries.last_mut()
                        && last.exit_code.is_none()
                    {
                        last.exit_code = mark.exit_code;
                    }
                }
                _ => {}
            }
        }
        entries
    }

    /// Slice the typed command out of the grid using the captured
    /// `(row, col)` of the `B` and `C` marks. Spans multiple rows
    /// when the shell's prompt + command wrapped. Wide characters
    /// (CJK / emoji) widen the column span by 2 each but are sliced
    /// by char count in the dumped row string — close enough for
    /// the picker label, which is truncated visually anyway.
    fn extract_command_text(&self, b: &PromptMark, c: &PromptMark) -> String {
        let start_col = (b.screen_col as usize).saturating_sub(1);
        let end_col = (c.screen_col as usize).saturating_sub(1);

        if b.screen_row == c.screen_row {
            let row = self.dump_screen_row(b.screen_row).unwrap_or_default();
            return slice_chars(&row, start_col, end_col).trim_end().to_string();
        }

        let mut out = String::new();
        if let Ok(row) = self.dump_screen_row(b.screen_row) {
            out.push_str(slice_chars(&row, start_col, usize::MAX).trim_end());
        }
        let mut r = b.screen_row + 1;
        while r < c.screen_row {
            if let Ok(row) = self.dump_screen_row(r) {
                out.push('\n');
                out.push_str(row.trim_end());
            }
            r += 1;
        }
        if end_col > 0
            && let Ok(row) = self.dump_screen_row(c.screen_row)
        {
            out.push('\n');
            out.push_str(slice_chars(&row, 0, end_col).trim_end());
        }
        out.trim_end().to_string()
    }

    pub fn cols(&self) -> u16 {
        self.config.cols
    }

    pub fn rows(&self) -> u16 {
        self.config.rows
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

    fn update_state_from_output(&mut self, bytes: &[u8]) {
        self.parse_tail.extend_from_slice(bytes);
        let buf = self.parse_tail.as_slice();

        // Track the furthest byte the CSI-mode scanner has fully
        // consumed. Used together with the OSC scanner's marker below
        // to drain already-processed prefix from `parse_tail` without
        // losing any cross-chunk partial sequence.
        let mut csi_consumed_upto = 0usize;
        let mut i = 0usize;
        while i + 2 < buf.len() {
            if buf[i] != 0x1b || buf[i + 1] != b'[' || buf[i + 2] != b'?' {
                i += 1;
                csi_consumed_upto = i;
                continue;
            }
            let csi_start = i;

            let mut k = i + 3;
            let mut nums: Vec<u32> = Vec::new();
            let mut num: u32 = 0;
            let mut saw_digit = false;
            let mut consumed = false;

            while k < buf.len() {
                let b = buf[k];
                if b.is_ascii_digit() {
                    saw_digit = true;
                    num = num.saturating_mul(10).saturating_add((b - b'0') as u32);
                    k += 1;
                    continue;
                }

                if b == b';' {
                    if saw_digit {
                        nums.push(num);
                        num = 0;
                        saw_digit = false;
                    }
                    k += 1;
                    continue;
                }

                if b == b'h' || b == b'l' {
                    if saw_digit {
                        nums.push(num);
                    }

                    let enabled = b == b'h';
                    for ps in nums {
                        match CsiMode::from_raw(ps) {
                            Some(CsiMode::DecCkm) => self.decckm_enabled = enabled,
                            Some(CsiMode::DecNkm) => self.decnkm_enabled = enabled,
                            Some(CsiMode::BracketedPaste) => self.bracketed_paste_enabled = enabled,
                            Some(CsiMode::FocusEvent) => self.focus_event_enabled = enabled,
                            Some(CsiMode::MouseX10) => self.mouse_x10_enabled = enabled,
                            Some(CsiMode::MouseButtonEvent) => {
                                self.mouse_button_event_enabled = enabled
                            }
                            Some(CsiMode::MouseAnyEvent) => self.mouse_any_event_enabled = enabled,
                            Some(CsiMode::MouseUtf8) => self.mouse_utf8_enabled = enabled,
                            Some(CsiMode::MouseSgr) => self.mouse_sgr_enabled = enabled,
                            Some(CsiMode::AlternateScroll) => {
                                self.alternate_scroll_enabled = enabled
                            }
                            Some(CsiMode::SynchronizedOutput) => self.synchronized_output = enabled,
                            None => {}
                        }
                        if matches!(
                            ps,
                            CSI_ALT_SCREEN_LEGACY | CSI_ALT_SCREEN | CSI_ALT_SCREEN_SAVE_CURSOR
                        ) && enabled != self.alt_screen
                        {
                            self.alt_screen = enabled;
                            self.screen_changed = true;
                        }
                    }

                    i = k + 1;
                    consumed = true;
                    break;
                }

                // Unknown intermediate/terminator byte — skip the
                // whole CSI up to and including this byte so neither
                // the scanner nor the drain re-parses it.
                i = k + 1;
                consumed = true;
                break;
            }

            if k >= buf.len() && !consumed {
                // Leave the incomplete CSI in `parse_tail` — next feed
                // will finish it. `csi_consumed_upto` already reflects
                // the position just before `csi_start` from the last
                // `i += 1` skip.
                i = csi_start;
                break;
            }

            if consumed {
                csi_consumed_upto = i;
                continue;
            }

            i += 1;
            csi_consumed_upto = i;
        }
        let _ = i;

        let mut osc_out = OscDispatch::default();
        // OSC 133 prompt marks are captured by `Osc133Scanner` during
        // incremental feed — not here — so the cursor position at
        // capture reflects the shell's intended row. This scanner still
        // *recognizes* OSC 133 payloads so `consumed_upto` advances past
        // them, but does not emit events.
        // Track the furthest byte that belongs to a fully-completed
        // sequence. Whatever is beyond `consumed_upto` is either a
        // partial OSC still waiting for its terminator (next chunk
        // will complete it) or plain text we haven't decided to
        // discard yet. Draining `[0..consumed_upto]` after the scan
        // keeps cross-chunk buffering while preventing re-parse
        // duplication of `prompt_marks` entries.
        let mut consumed_upto = 0usize;
        let mut j = 0usize;
        while j + 1 < buf.len() {
            if buf[j] != 0x1b || buf[j + 1] != b']' {
                j += 1;
                consumed_upto = j;
                continue;
            }

            let osc_start = j;
            let mut k = j + 2;
            let mut ps: u32 = 0;
            let mut saw_digit = false;
            while k < buf.len() {
                let b = buf[k];
                if b.is_ascii_digit() {
                    saw_digit = true;
                    ps = ps.saturating_mul(10).saturating_add((b - b'0') as u32);
                    k += 1;
                    continue;
                }
                if b == b';' {
                    k += 1;
                    break;
                }
                break;
            }
            if !saw_digit || k >= buf.len() {
                // Incomplete — preserve this OSC head for the next
                // chunk by stopping advancement at `osc_start`.
                break;
            }
            // Bind so the `let osc_start = j;` above is read.
            let _ = osc_start;

            let payload_start = k;
            let mut terminated = false;
            while k < buf.len() {
                match buf[k] {
                    0x07 => {
                        commit_osc_payload(
                            ps,
                            &buf[payload_start..k],
                            &mut osc_out,
                            self.config.track_cwd,
                        );
                        k += 1;
                        terminated = true;
                        break;
                    }
                    0x1b if k + 1 < buf.len() && buf[k + 1] == b'\\' => {
                        commit_osc_payload(
                            ps,
                            &buf[payload_start..k],
                            &mut osc_out,
                            self.config.track_cwd,
                        );
                        k += 2;
                        terminated = true;
                        break;
                    }
                    _ => k += 1,
                }
            }

            if !terminated {
                // No terminator yet — leave `consumed_upto` before
                // `osc_start` so the next feed can pick up where we
                // stopped.
                break;
            }
            j = k;
            consumed_upto = k;
        }

        // Drain only what BOTH scanners have fully processed, so a
        // CSI mode sequence that spans a feed boundary survives into
        // the next call (same for OSC payloads).
        let drain_to = csi_consumed_upto.min(consumed_upto);
        if drain_to > 0 {
            self.parse_tail.drain(0..drain_to);
        }
        // Runaway protection: if an unterminated sequence keeps the
        // scanner stuck past the cap, abandon it rather than truncate
        // from the front (which would chop the escape header). OSC
        // 1337 `Copy=…:<base64>` payloads can legitimately exceed the
        // generic 64 KiB tail limit, so the effective cap is
        // `max(PARSE_TAIL_LIMIT, osc1337_max_bytes)`.
        let tail_limit = PARSE_TAIL_LIMIT.max(self.config.osc1337_max_bytes);
        if self.parse_tail.len() > tail_limit {
            self.parse_tail.clear();
        }

        if let Some(title) = osc_out.title {
            self.title = Some(title);
        }
        if let Some(clipboard) = osc_out.clipboard {
            self.clipboard_write = Some(clipboard);
        }
        if let Some(cwd) = osc_out.cwd {
            self.cwd = Some(cwd);
        }
        if let Some(kind) = osc_out.attention {
            // Last sequence wins. The drain side (`platform::attention::apply`)
            // cancels any in-flight request before issuing the new one,
            // so a Cancel inside the same feed naturally clears it.
            self.pending_attention = Some(kind);
        }
        if let Some(req) = osc_out.notification {
            self.pending_notification = Some(req);
        }
        if osc_out.clear_scrollback {
            self.pending_clear_scrollback = true;
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let was_alt = self.alt_screen;
        self.update_state_from_output(bytes);
        if !was_alt && self.alt_screen {
            // ghostty_vt is still on the primary screen here — \x1b[3J
            // clears primary-screen scrollback before feed_incremental()
            // processes \x1b[?1049h and switches buffers.
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        let result = self.feed_incremental(bytes, |_| {});
        if std::mem::take(&mut self.pending_clear_scrollback) {
            // Drain after `feed_incremental` so the chunk's own
            // pre-clear output (cursor home, status line redraw, …)
            // is still in the visible viewport when `\x1b[3J`
            // wipes the rows above it.
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        result
    }

    pub fn feed_with_pty_responses(
        &mut self,
        bytes: &[u8],
        send: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        let was_alt = self.alt_screen;
        self.update_state_from_output(bytes);
        if !was_alt && self.alt_screen {
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        let result = self.feed_incremental(bytes, send);
        if std::mem::take(&mut self.pending_clear_scrollback) {
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        result
    }

    /// Feed `bytes` to ghostty_vt in segments split at points where we
    /// need to observe or respond to state:
    ///   * DSR / OSC color queries — synthesize a response.
    ///   * OSC 133 FTCS marks — capture cursor position *after* the
    ///     segment ending with the terminator so `screen_row` reflects
    ///     the row the shell intended.
    fn feed_incremental(&mut self, bytes: &[u8], mut send: impl FnMut(&[u8])) -> Result<(), Error> {
        let mut seg_start = 0usize;
        for (i, &b) in bytes.iter().enumerate() {
            let dsr = self.dsr_state.advance(b);
            let osc_query = self.osc_query_state.advance(b);
            let osc133 = self.osc133_scanner.advance(b);
            let cap = self.capability_scanner.advance(b);
            let xtgettcap = self.xtgettcap_scanner.advance(b);
            if dsr.is_none()
                && osc_query.is_none()
                && osc133.is_none()
                && cap.is_none()
                && xtgettcap.is_none()
            {
                continue;
            }

            self.terminal.feed(&bytes[seg_start..=i])?;
            seg_start = i + 1;

            if let Some(query) = dsr {
                match query {
                    TerminalQuery::DeviceStatus => send(ansi::DSR_OK),
                    TerminalQuery::CursorPosition => {
                        let (col, row) = self.cursor_position().unwrap_or((1, 1));
                        send(ansi::cpr_reply(row, col).as_bytes());
                    }
                }
            }

            if let Some(query) = osc_query {
                let (ps, rgb) = match query {
                    OscQuery::ForegroundColor => {
                        let fg = self.config.default_fg;
                        (OSC_DEFAULT_FG, (fg.r, fg.g, fg.b))
                    }
                    OscQuery::BackgroundColor => {
                        let bg = self.config.default_bg;
                        (OSC_DEFAULT_BG, (bg.r, bg.g, bg.b))
                    }
                };
                send(ansi::osc_color_reply(ps, rgb).as_bytes());
            }

            if let Some((kind, exit_code)) = osc133 {
                let cursor = self.terminal.cursor_position().unwrap_or((1, 1));
                let screen_row =
                    self.terminal.viewport_row_offset() + u32::from(cursor.1.saturating_sub(1));
                self.push_prompt_mark(PromptMark {
                    kind,
                    screen_row,
                    screen_col: cursor.0,
                    exit_code,
                });
            }

            if let Some(query) = cap {
                match query {
                    CapabilityQuery::PrimaryDa => send(ansi::PRIMARY_DA),
                    CapabilityQuery::SecondaryDa => send(ansi::SECONDARY_DA),
                    CapabilityQuery::TertiaryDa => send(ansi::TERTIARY_DA),
                    CapabilityQuery::XtVersion => send(ansi::XTVERSION),
                    CapabilityQuery::KittyKeyboard => send(ansi::KITTY_KEYBOARD_RESPONSE),
                    CapabilityQuery::Decrqm(ps) => {
                        let pm = self.decrqm_pm(ps);
                        send(ansi::decrqm_reply(ps, pm).as_bytes());
                    }
                }
            }

            if let Some(body) = xtgettcap {
                let resp = build_xtgettcap_response(&body);
                if !resp.is_empty() {
                    send(&resp);
                }
            }
        }

        if seg_start < bytes.len() {
            self.terminal.feed(&bytes[seg_start..])?;
        }

        Ok(())
    }

    pub fn dump_viewport(&self) -> Result<String, Error> {
        self.terminal.dump_viewport()
    }

    pub fn dump_viewport_row(&self, row: u16) -> Result<String, Error> {
        self.terminal.dump_viewport_row(row)
    }

    /// Dump a single row from the `screen` coordinate space. `y`
    /// ranges over `[0, total_rows())`, covering scrollback and the
    /// active viewport. Used by scrollback-aware search.
    pub fn dump_screen_row(&self, y: u32) -> Result<String, Error> {
        self.terminal.dump_screen_row(y)
    }

    pub fn dump_viewport_row_cell_styles(
        &self,
        row: u16,
    ) -> Result<Vec<ghostty_vt::CellStyle>, Error> {
        self.terminal.dump_viewport_row_cell_styles(row)
    }

    pub fn dump_viewport_row_style_runs(
        &self,
        row: u16,
    ) -> Result<Vec<ghostty_vt::StyleRun>, Error> {
        self.terminal.dump_viewport_row_style_runs(row)
    }

    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        self.terminal.cursor_position()
    }

    pub fn cursor_style(&self) -> u8 {
        self.terminal.cursor_style()
    }

    pub fn cursor_visible(&self) -> bool {
        self.terminal.cursor_visible()
    }

    pub fn take_bell(&mut self) -> bool {
        self.terminal.take_bell()
    }

    pub fn total_rows(&self) -> u32 {
        self.terminal.total_rows()
    }

    pub fn viewport_row_offset(&self) -> u32 {
        self.terminal.viewport_row_offset()
    }

    pub fn scroll_viewport(&mut self, delta_lines: i32) -> Result<(), Error> {
        self.terminal.scroll_viewport(delta_lines)
    }

    pub fn scroll_viewport_top(&mut self) -> Result<(), Error> {
        self.terminal.scroll_viewport_top()
    }

    pub fn scroll_viewport_bottom(&mut self) -> Result<(), Error> {
        self.terminal.scroll_viewport_bottom()
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        self.config.cols = cols;
        self.config.rows = rows;
        self.terminal.resize(cols, rows)
    }

    pub fn take_dirty_viewport_rows(&mut self) -> Vec<u16> {
        self.terminal
            .take_dirty_viewport_rows(self.config.rows)
            .unwrap_or_default()
    }

    pub fn take_viewport_scroll_delta(&mut self) -> i32 {
        self.terminal.take_viewport_scroll_delta()
    }

    /// DECRQM parameter value for the given private mode number.
    ///
    /// Returns: `0` = not recognised, `1` = set, `2` = reset.
    fn decrqm_pm(&self, ps: u32) -> u8 {
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

/// Char-count-based slice for grid row text. `start` and `end` are
/// 0-indexed char positions, NOT byte offsets — `dump_screen_row`
/// emits one char per cell (wide characters reuse a single char
/// position, even though they occupy two columns of the grid). For
/// pure-ASCII shell commands chars and columns coincide; for CJK
/// the column→char gap is acceptable for the picker label, which
/// the renderer truncates anyway. `end == usize::MAX` is treated
/// as "to end of string".
fn slice_chars(s: &str, start: usize, end: usize) -> &str {
    // Empty / inverted range — guard up front. The naive
    // `nth(take - 1)` formulation collapses 0→0 and reaches the
    // next char, hallucinating a one-char slice when the caller
    // asked for none.
    if end != usize::MAX && end <= start {
        return "";
    }
    let mut iter = s.char_indices();
    let Some((start_byte, start_char)) = iter.by_ref().nth(start) else {
        return "";
    };
    if end == usize::MAX {
        return &s[start_byte..];
    }
    // `nth(start)` already yielded the start char — that counts as
    // one of the `end - start` chars we want to include. Walk the
    // iterator another `end - start - 1` steps, accumulating each
    // char's byte length so the final `byte_end` lands one byte
    // past the last char we keep.
    let mut byte_end = start_byte + start_char.len_utf8();
    for _ in 1..(end - start) {
        match iter.next() {
            Some((_, c)) => byte_end += c.len_utf8(),
            None => break,
        }
    }
    &s[start_byte..byte_end]
}
