use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ghostty_vt::{Error, Rgb, Terminal};

use crate::TerminalConfig;
use crate::ansi;
use crate::session::interval_tree::{
    IntervalTree, LineCoord, LineRange, MarkId, MarkPayload, MarkRecordSink,
};
use crate::vt_codes::{
    AttentionKind, CSI_ALT_SCREEN, CSI_ALT_SCREEN_LEGACY, CSI_ALT_SCREEN_SAVE_CURSOR, CsiMode,
    NotificationRequest, OSC_DEFAULT_BG, OSC_DEFAULT_FG,
};
use crate::vt_limits::{PARSE_TAIL_LIMIT, PROMPT_MARKS_CAP};

mod annotation_ops;
pub mod interval_tree;
mod line_buffer;
mod scanners;

pub use annotation_ops::AnnotationError;
pub use line_buffer::{
    EolKind, FindContext, FindMatchRange, FindOptions, LbCell, LineBuffer, LineBufferPosition,
    LogicalLine,
};
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
/// command text the user typed, the current-frame screen row at
/// translation time of the `CommandStart` mark (for jump-to-command
/// in scrollback), and the exit code from `D` if observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandHistoryEntry {
    pub command_text: String,
    /// Current-frame screen row of the `CommandStart` mark, translated
    /// from its stored `abs_y` at the time `command_history()` was
    /// called. Not stable across subsequent eviction — re-query
    /// `command_history()` after the frame to refresh.
    pub start_row: u32,
    pub exit_code: Option<i32>,
    /// Wall-clock duration between the `CommandExecuted` (FTCS C) and
    /// `CommandFinished` (FTCS D) marks. `None` if D has not arrived
    /// yet or the C-mark timestamp was not captured.
    pub duration: Option<std::time::Duration>,
}

/// One semantic boundary reported by a shell that speaks FinalTerm / OSC
/// 133. Captured with an *absolute* Y coordinate that includes lines
/// already evicted from `LineBuffer` (via `LineBuffer::overflow()`), so
/// marks survive ring eviction. Translate to a current-frame screen row
/// via [`TerminalSession::abs_to_screen_row`] before indexing into
/// `dump_screen_row`. Mirrors iTerm2 `VT100Terminal.m:4520-4616`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptMark {
    pub kind: PromptMarkKind,
    /// Ever-increasing absolute Y at the time the mark fired,
    /// computed as `line_buffer.overflow() + line_buffer.wrapped_row_count
    /// + cursor_y - 1`. Stable across `LineBuffer` ring eviction.
    pub abs_y: u64,
    /// 1-indexed cursor column at the moment the mark fired. Captured
    /// alongside `abs_y` so the command-history extractor can slice
    /// the typed command out of `[B.col .. C.col]` on the command's
    /// row(s) without re-deriving prompt-prefix length from the grid.
    pub screen_col: u16,
    /// Exit code attached to `CommandFinished` (FTCS D). `None` for A/B/C.
    pub exit_code: Option<i32>,
    /// Wall-clock instant the mark was dispatched. Used by
    /// `command_history` to compute the C→D elapsed duration. Mirrors
    /// iTerm2 `LineBlockMetadata.lineMetadata` per-mark timestamps.
    pub timestamp: std::time::SystemTime,
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
    /// iTerm2-style logical-line scrollback. Filled by
    /// `capture_scrolled_out` after every feed; consumers wrap it lazily
    /// against the live cell width. Independent of ghostty's own
    /// physical scrollback (which we still own for input/echo correctness).
    line_buffer: LineBuffer,
    /// Highest absolute screen-row index already drained into
    /// `line_buffer`. `None` until the first capture; reset on
    /// alt-screen entry (the grid is wiped so the row indices
    /// associated with this counter are no longer meaningful).
    last_captured_abs_row: Option<u32>,
    /// Daruda-owned scroll position in rows back from the bottom. `0`
    /// pins the viewport to the live grid; positive values reveal rows
    /// from `line_buffer` above the live viewport. Replaces ghostty's
    /// internal `scroll_viewport` state — we no longer rely on its
    /// scrollback for navigation (Task 3 cut ghostty's retained
    /// scrollback to a tiny capture window; see `GHOSTTY_TRANSIENT_SCROLLBACK`).
    scroll_offset: u32,
    /// Augmented interval tree storing user-authored marks (annotations
    /// today; prompt regions and search hits in future SP). Lifecycle is
    /// wired through `capture_scrolled_out` (eviction), `resize` (column
    /// clamp), and the alt-screen toggle (visibility filter). See
    /// `crate::session::annotation_ops` for the public API.
    pub(in crate::session) interval_tree: IntervalTree<MarkPayload>,
}

/// Rows ghostty retains as a transient buffer so `capture_scrolled_out`
/// can read a row after the feed that evicted it from the live viewport
/// but before the next feed evicts it from ghostty too. Daruda's own
/// scrollback lives in `LineBuffer`; ghostty's tiny ring exists purely
/// to bridge "row scrolled out of live viewport" → "row copied into
/// LineBuffer". Sized as `max(GHOSTTY_TRANSIENT_SCROLLBACK, rows * 2)`
/// in `new()` so taller terminals always have at least a viewport's
/// worth of headroom.
///
/// LIMITATION: a single `feed()` that scrolls more rows than
/// `max(GHOSTTY_TRANSIENT_SCROLLBACK, rows * 2)` out of the viewport
/// will lose the oldest ones (they evict from ghostty before
/// `capture_scrolled_out` runs at the end of the feed). For typical
/// interactive use (one PTY chunk per frame) this is harmless; for
/// bulk output (`cat large_file`) the oldest lines of any feed batch
/// may be lost. A future improvement: invoke `capture_scrolled_out`
/// from within ghostty's eviction hook so no row can be evicted
/// without being snapshotted first.
const GHOSTTY_TRANSIENT_SCROLLBACK: usize = 64;

impl TerminalSession {
    pub fn new(config: TerminalConfig) -> Result<Self, Error> {
        // Ghostty keeps a small ring so `capture_scrolled_out` can still
        // read evicted rows before the next feed drops them; daruda's
        // own scrollback lives in `line_buffer`.
        let ghostty_scrollback = GHOSTTY_TRANSIENT_SCROLLBACK.max((config.rows as usize) * 2);
        let mut terminal = Terminal::with_scrollback(config.cols, config.rows, ghostty_scrollback)?;
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
            line_buffer: LineBuffer::new(config.max_scrollback),
            last_captured_abs_row: None,
            scroll_offset: 0,
            interval_tree: IntervalTree::new(),
        })
    }

    /// Attach an NDJSON sink to the interval tree. Subsequent mutation
    /// methods on `IntervalTree<MarkPayload>` will emit records into
    /// `sink`. Replaces any previously installed sink.
    ///
    /// Kept `pub` because the wiring (lane_dir → sink → session) happens
    /// in app code, outside this crate.
    pub fn set_marks_sink(&mut self, sink: Box<dyn MarkRecordSink>) {
        self.interval_tree.set_sink(sink);
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

    /// Translate an absolute screen row `row` (0 = topmost scrollback row,
    /// matching the [`Self::dump_screen_row`] / [`Self::total_rows`]
    /// coordinate space) into a [`LineCoord`] from the interval tree's
    /// coordinate system.
    ///
    /// Rows inside the `LineBuffer` region resolve to a width-invariant
    /// [`LineCoord::Buffered`] anchor; rows on the live viewport resolve
    /// to [`LineCoord::Viewport`] with `abs_y = overflow + row`.
    ///
    /// Returns `None` only when `row < lb_rows` and `position_for_visual_row`
    /// cannot resolve the row (e.g. zero cols or a sparse buffer). Viewport
    /// rows (`row >= lb_rows`) always return `Some(LineCoord::Viewport { .. })`.
    pub fn screen_row_to_line_coord(&self, row: u32) -> Option<LineCoord> {
        let cols = self.config.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if row < lb_rows {
            self.line_buffer
                .position_for_visual_row(row, cols)
                .map(|(pos, _, _)| LineCoord::Buffered(pos))
        } else {
            // Viewport region. The `abs_y` mirrors `PromptMark.abs_y`'s
            // construction (`overflow + lb_rows + (row - lb_rows)`),
            // which simplifies to `overflow + row`.
            Some(LineCoord::Viewport {
                abs_y: self.line_buffer.overflow().saturating_add(row as u64),
            })
        }
    }

    /// Return `true` when `screen_row` is a wrap-continuation row inside
    /// `LineBuffer` — i.e. the sub-row index within the logical line is > 0.
    ///
    /// The annotation overlay paint loop uses this to skip continuation rows
    /// so each wrapped logical line produces exactly one annotation box
    /// (anchored to the head row, sub_row == 0) instead of one per wrapped
    /// visual row.
    ///
    /// SP-1 limitation: rows in the live ghostty viewport (`screen_row >=
    /// lb_rows`) always return `false` — ghostty_vt does not expose a
    /// wrap-continuation predicate for viewport rows in a way that can be
    /// consumed here without new FFI. The annotation-paint duplication bug
    /// (resize-wrap) is triggered by rows already captured in `LineBuffer`,
    /// so this limitation is acceptable for SP-1.
    pub fn is_wrap_continuation(&self, screen_row: u32) -> bool {
        let cols = self.config.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if screen_row >= lb_rows {
            // SP-1: viewport-region wrap continuations are not detected;
            // ghostty does not expose per-row wrap info for live grid rows.
            return false;
        }
        self.line_buffer
            .position_for_visual_row(screen_row, cols)
            .map(|(_, sub_row, _)| sub_row > 0)
            .unwrap_or(false)
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
    /// Returns `None` if no closed block has been observed, or if
    /// either bound has been evicted from `LineBuffer`. Rows are in
    /// the current screen-frame coordinate space — pass directly to
    /// [`Self::dump_screen_row`].
    pub fn last_command_output_rows(&self) -> Option<std::ops::Range<u32>> {
        // Walk backwards so we always pick the most recent complete
        // pair. Prefer the semantic E/F pair (output body only) over
        // the C/D pair (which includes the command echo itself).
        let mut end_abs: Option<u64> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::SemanticTextEnd if end_abs.is_none() => {
                    end_abs = Some(mark.abs_y);
                }
                PromptMarkKind::SemanticTextStart => {
                    if let Some(end) = end_abs {
                        return self.abs_range_to_screen(mark.abs_y, end);
                    }
                }
                _ => {}
            }
        }
        // Fallback: the C/D pair captures the whole command-through-
        // output region when the shell doesn't emit E/F.
        let mut end_abs: Option<u64> = None;
        for mark in self.prompt_marks.iter().rev() {
            match mark.kind {
                PromptMarkKind::CommandFinished if end_abs.is_none() => {
                    end_abs = Some(mark.abs_y);
                }
                PromptMarkKind::CommandExecuted => {
                    if let Some(end) = end_abs {
                        return self.abs_range_to_screen(mark.abs_y, end);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Translate an `[start_abs, end_abs)` pair of absolute Y
    /// coordinates to a current-frame `Range<u32>`. Returns `None` if
    /// either bound has been evicted from `LineBuffer`, or if the range
    /// would be empty.
    fn abs_range_to_screen(&self, start_abs: u64, end_abs: u64) -> Option<std::ops::Range<u32>> {
        let start = self.abs_to_screen_row(start_abs)?;
        let end = self.abs_to_screen_row(end_abs)?;
        if start >= end {
            return None;
        }
        Some(start..end)
    }

    /// Walk `prompt_marks` for completed shell commands. Each
    /// `(CommandStart, CommandExecuted)` pair becomes one entry; a
    /// following `CommandFinished` populates the exit code. Entries
    /// are returned oldest-first so a UI consumer can render the
    /// natural top-down or reverse it for "most recent first".
    /// Pairs whose `B` mark has been evicted from `LineBuffer` are
    /// dropped (we can no longer slice the typed command text).
    pub fn command_history(&self) -> Vec<CommandHistoryEntry> {
        let mut entries: Vec<CommandHistoryEntry> = Vec::new();
        let mut last_b: Option<PromptMark> = None;
        // Carry the C-mark timestamp of the most recently opened entry so
        // a following D mark can compute the C→D elapsed duration. Reset
        // when a D mark consumes it (or when a fresh C opens a new entry).
        let mut last_c_ts: Option<std::time::SystemTime> = None;
        for mark in &self.prompt_marks {
            match mark.kind {
                PromptMarkKind::CommandStart => {
                    last_b = Some(*mark);
                }
                PromptMarkKind::CommandExecuted => {
                    let Some(b) = last_b.take() else {
                        continue;
                    };
                    let Some(b_row) = self.abs_to_screen_row(b.abs_y) else {
                        continue;
                    };
                    let Some(text) = self.extract_command_text(&b, mark, b_row) else {
                        continue;
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    entries.push(CommandHistoryEntry {
                        command_text: text,
                        start_row: b_row,
                        exit_code: None,
                        duration: None,
                    });
                    last_c_ts = Some(mark.timestamp);
                }
                PromptMarkKind::CommandFinished => {
                    if let Some(last) = entries.last_mut()
                        && last.exit_code.is_none()
                    {
                        last.exit_code = mark.exit_code;
                        if let Some(c_ts) = last_c_ts.take() {
                            last.duration = mark.timestamp.duration_since(c_ts).ok();
                        }
                    }
                }
                _ => {}
            }
        }
        entries
    }

    /// Slice the typed command out of the grid using the captured
    /// `(abs_y, col)` of the `B` and `C` marks. Spans multiple rows
    /// when the shell's prompt + command wrapped. Wide characters
    /// (CJK / emoji) widen the column span by 2 each but are sliced
    /// by char count in the dumped row string — close enough for
    /// the picker label, which is truncated visually anyway.
    ///
    /// Returns `None` when either mark has been evicted from
    /// `LineBuffer` — the typed command is no longer reachable.
    ///
    /// `b_row` is the pre-translated screen row of `b.abs_y` — callers
    /// already need it for `CommandHistoryEntry::start_row`, so pass it
    /// in to avoid a redundant `abs_to_screen_row` call.
    fn extract_command_text(&self, b: &PromptMark, c: &PromptMark, b_row: u32) -> Option<String> {
        let start_col = (b.screen_col as usize).saturating_sub(1);
        let end_col = (c.screen_col as usize).saturating_sub(1);
        let c_row = self.abs_to_screen_row(c.abs_y)?;
        if b_row > c_row {
            // Defensive: monotonic abs_y + monotonic translation makes this
            // unreachable in practice, but eviction of just one of the two
            // marks could in principle invert the range.
            return None;
        }

        if b_row == c_row {
            let row = self.dump_screen_row(b_row).unwrap_or_default();
            return Some(slice_chars(&row, start_col, end_col).trim_end().to_string());
        }

        let mut out = String::new();
        if let Ok(row) = self.dump_screen_row(b_row) {
            out.push_str(slice_chars(&row, start_col, usize::MAX).trim_end());
        }
        let mut r = b_row + 1;
        while r < c_row {
            if let Ok(row) = self.dump_screen_row(r) {
                out.push('\n');
                out.push_str(row.trim_end());
            }
            r += 1;
        }
        if end_col > 0
            && let Ok(row) = self.dump_screen_row(c_row)
        {
            out.push('\n');
            out.push_str(slice_chars(&row, 0, end_col).trim_end());
        }
        Some(out.trim_end().to_string())
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
                            self.interval_tree.set_alt_screen_active(self.alt_screen);
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
        self.feed_internal(bytes, |_| {})
    }

    pub fn feed_with_pty_responses(
        &mut self,
        bytes: &[u8],
        send: impl FnMut(&[u8]),
    ) -> Result<(), Error> {
        self.feed_internal(bytes, send)
    }

    fn feed_internal(&mut self, bytes: &[u8], send: impl FnMut(&[u8])) -> Result<(), Error> {
        let was_alt = self.alt_screen;
        self.update_state_from_output(bytes);
        let entering_alt = !was_alt && self.alt_screen;
        if entering_alt {
            // ghostty_vt is still on the primary screen here — \x1b[3J
            // clears primary-screen scrollback before feed_incremental()
            // processes \x1b[?1049h and switches buffers.
            let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        }
        let result = self.feed_incremental(bytes, send);
        if std::mem::take(&mut self.pending_clear_scrollback) {
            self.apply_clear_scrollback();
        }
        if entering_alt {
            self.on_enter_alt_screen();
        } else {
            self.sync_after_ghostty_scrollback_shrink();
            self.capture_scrolled_out();
        }
        result
    }

    /// Consolidated handler for the `OSC 1337 ; ClearScrollback`
    /// post-feed wipe. Drains after `feed_incremental` so the chunk's
    /// own pre-clear output (cursor home, status line redraw, …) is
    /// still in the visible viewport when `\x1b[3J` wipes the rows
    /// above it.
    ///
    /// LineBuffer mirrors ghostty's scrollback; clearing one without the
    /// other would desync the dispatcher and leave `last_captured_abs_row`
    /// pointing past the (now-reset) `viewport_row_offset`, silently
    /// breaking every subsequent capture for the rest of the session.
    fn apply_clear_scrollback(&mut self) {
        // SILENT-OK: ERASE_SCROLLBACK is a fixed constant sequence; a feed
        // error on a known-valid byte sequence indicates a terminal in a
        // broken state that will surface through subsequent feeds with
        // proper error reporting.
        let _ = self.terminal.feed(crate::ansi::ERASE_SCROLLBACK);
        self.clear_line_buffer_and_shift_marks();
    }

    /// Snapshot every row that has scrolled out of the live viewport
    /// since the last call into `line_buffer`. Wrap-continuations
    /// (DECAWM soft-wrap) merge into the same logical line so the
    /// scrollback stores joined text irrespective of the original
    /// physical row width.
    ///
    /// Idempotent fast path: if `viewport_row_offset` has not advanced
    /// past `last_captured_abs_row`, returns without dumping.
    fn capture_scrolled_out(&mut self) {
        // Capture is meaningless while ghostty is rendering into the
        // alt-screen buffer — those rows are by definition transient
        // UI, not scrollback content. We only sample primary-screen
        // rows as they scroll off the top.
        if self.alt_screen {
            return;
        }
        let viewport_top = self.terminal.viewport_row_offset();
        let start = self
            .last_captured_abs_row
            .map(|r| r.saturating_add(1))
            .unwrap_or(0);
        if start >= viewport_top {
            return;
        }
        for y in start..viewport_top {
            // Empty text on dump failure (e.g., row evicted from ghostty
            // scrollback); preserves line numbering.
            let text = self.terminal.dump_screen_row(y).unwrap_or_default();
            let runs = self
                .terminal
                .dump_screen_row_style_runs(y)
                .unwrap_or_default();
            let url_ids = self.terminal.dump_screen_row_url_ids(y).unwrap_or_default();
            let eol = match self.terminal.row_wrap_kind(y) {
                ghostty_vt::WrapKind::Soft => EolKind::Soft,
                ghostty_vt::WrapKind::Hard => EolKind::Hard,
            };
            self.line_buffer.append(&text, &runs, eol);
            self.line_buffer.attach_url_ids_to_tail(&url_ids);
        }
        self.last_captured_abs_row = Some(viewport_top.saturating_sub(1));
        // Rebind viewport-resident marks whose row has just been captured
        // into LineBuffer. Design §6: a mark registered with
        // LineCoord::Viewport{abs_y} while the row was live must be
        // rewritten to LineCoord::Buffered once the row enters scrollback,
        // otherwise the Ord mismatch (Buffered < Viewport) causes every
        // range query to miss the mark from this point on.
        {
            let overflow = self.line_buffer.overflow();
            let lb_rows = self.line_buffer.wrapped_row_count(self.config.cols);
            let cols = self.config.cols;
            let buffered_upper_excl = overflow + lb_rows as u64;

            // Collect first: iter() borrows &self, update_payload_range needs &mut self.
            let to_rebind: Vec<(MarkId, LineRange)> = self
                .interval_tree
                .iter()
                .filter_map(|m| match (m.range.start, m.range.end) {
                    // SP-1 marks are single-line (start == end).  Only rebind when
                    // both endpoints are Viewport and fall inside the LineBuffer
                    // window.  Multi-line marks are out of scope for SP-1.
                    (LineCoord::Viewport { abs_y: s }, LineCoord::Viewport { abs_y: e })
                        if s == e && s >= overflow && s < buffered_upper_excl =>
                    {
                        let row_in_lb = (s - overflow) as u32;
                        let pos = self.line_buffer.position_for_visual_row(row_in_lb, cols)?.0;
                        let new_coord = LineCoord::Buffered(pos);
                        Some((m.id, LineRange::new(new_coord, new_coord)))
                    }
                    _ => None,
                })
                .collect();

            for (id, new_range) in to_rebind {
                self.interval_tree.update_payload_range(id, new_range);
            }
        }
        // Drop interval-tree marks whose range fell off the bottom of
        // the live scrollback window. `min_position` returns `None` on
        // an empty buffer (nothing to evict against).
        if let Some(min) = self.line_buffer.min_position() {
            self.interval_tree.evict_below(LineCoord::Buffered(min));
        }
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

    /// Detect a `\x1b[3J` (or equivalent) ghostty-side scrollback wipe
    /// and mirror it into `line_buffer`. We don't parse the CSI ED
    /// directly — we infer the wipe from ghostty's
    /// `viewport_row_offset` dropping below the highest row we've
    /// already captured. That's the only way for ghostty's scrollback
    /// to shrink between feeds in a primary-screen session, so the
    /// inference is unambiguous.
    fn sync_after_ghostty_scrollback_shrink(&mut self) {
        let Some(last) = self.last_captured_abs_row else {
            return;
        };
        if self.terminal.viewport_row_offset() <= last {
            self.clear_line_buffer_and_shift_marks();
        }
    }

    /// Shared post-wipe bookkeeping: clear `LineBuffer`, reset the
    /// capture cursor and scroll offset, and shift any viewport-resident
    /// prompt marks down by the cleared history while dropping marks
    /// whose row was inside the cleared history.
    fn clear_line_buffer_and_shift_marks(&mut self) {
        let overflow = self.line_buffer.overflow();
        let history = self.line_buffer.wrapped_row_count(self.config.cols) as u64;
        let history_top = overflow + history;
        self.line_buffer.clear();
        self.last_captured_abs_row = None;
        // `scroll_offset` indexes into the now-empty `line_buffer`;
        // leaving it set would pin the viewport above the live grid.
        self.scroll_offset = 0;
        // Drop marks whose abs_y addressed a row inside the cleared
        // history (they would alias onto unrelated viewport rows after
        // the shift). Surviving viewport-resident marks shift down by
        // `history` so they keep pointing at the right row.
        //
        // INVARIANT: LineBuffer::clear() preserves overflow(). The shift
        // below removes the wiped-history rows from each mark's abs_y,
        // keeping translation against the unchanged overflow valid.
        self.prompt_marks.retain(|m| m.abs_y >= history_top);
        for m in &mut self.prompt_marks {
            m.abs_y = m.abs_y.saturating_sub(history);
        }
    }

    /// Alt-screen entry handler. Seals any partial tail (the next time
    /// we re-enter primary screen and start capturing, a soft tail
    /// shouldn't grow into an unrelated row) and resets the capture
    /// cursor — ghostty's screen-row indices are no longer meaningful
    /// once the buffer is switched out.
    ///
    /// Asymmetric by design: entry seals + resets, but exit
    /// (alt → primary) has no counterpart. When the primary screen
    /// is restored, ghostty repaints it from row 0; the next
    /// `capture_scrolled_out` pass naturally re-anchors with
    /// `last_captured_abs_row == None`, so there's nothing to reset.
    fn on_enter_alt_screen(&mut self) {
        // Seal a partial tail so a re-entry into primary screen does
        // not append into a now-stale soft-wrap continuation. We do
        // NOT clear `line_buffer` — daruda's persistent scrollback
        // survives alt-screen cycles (iTerm2 / Alacritty parity).
        // Ghostty's transient ring is wiped via the `\x1b[3J`
        // injection in `feed`, but that's a separate buffer used
        // only as a capture window.
        self.line_buffer.seal_partial();
        self.last_captured_abs_row = None;
        // Alt-screen apps own the full grid; any user-initiated scroll
        // into history made before the switch is no longer meaningful.
        self.scroll_offset = 0;
    }

    /// Feed `bytes` to ghostty_vt in segments split at points where we
    /// need to observe or respond to state:
    ///   * DSR / OSC color queries — synthesize a response.
    ///   * OSC 133 FTCS marks — capture cursor position *after* the
    ///     segment ending with the terminator so the captured `abs_y`
    ///     reflects the row the shell intended.
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
                let abs_y = self.current_abs_y_at_cursor();
                self.push_prompt_mark(PromptMark {
                    kind,
                    abs_y,
                    screen_col: cursor.0,
                    exit_code,
                    timestamp: std::time::SystemTime::now(),
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

    /// Dump the visible viewport. When `scroll_offset > 0` the viewport
    /// has been scrolled into `line_buffer` history, so compose the
    /// output from the dispatcher (`dump_screen_row`) instead of
    /// ghostty's live grid — otherwise PageUp / scrollbar would be a
    /// visual no-op. Output shape matches ghostty's `dump_viewport`:
    /// rows joined by `\n`, no trailing newline (see
    /// `view::viewport::split_viewport_lines`).
    pub fn dump_viewport(&self) -> Result<String, Error> {
        if self.scroll_offset == 0 {
            return self.terminal.dump_viewport();
        }
        let rows = self.config.rows as u32;
        let top = self.viewport_row_offset();
        let mut out = String::new();
        for i in 0..rows {
            let y = top + i;
            let row = self.dump_screen_row(y).unwrap_or_default();
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&row);
        }
        Ok(out)
    }

    pub fn dump_viewport_row(&self, row: u16) -> Result<String, Error> {
        self.terminal.dump_viewport_row(row)
    }

    /// Dump a single row from the `screen` coordinate space. `y`
    /// ranges over `[0, total_rows())`, covering scrollback and the
    /// active viewport.
    ///
    /// Dispatcher: when `y` addresses a row inside `line_buffer`'s
    /// wrapped range, the row text comes from there; rows beyond that
    /// land on ghostty's live viewport.
    pub fn dump_screen_row(&self, y: u32) -> Result<String, Error> {
        let cols = self.config.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if y < lb_rows {
            self.line_buffer
                .visual_row(y, cols)
                .ok_or(Error::DumpFailed)
        } else {
            let vp_row = (y - lb_rows) as u16;
            self.terminal.dump_viewport_row(vp_row)
        }
    }

    /// Style runs for any absolute screen row. Dispatcher mirror of
    /// [`Self::dump_screen_row`]: scrollback rows come from
    /// `line_buffer`, viewport rows from ghostty. Output columns are
    /// 1-indexed inclusive (StyleRun convention).
    pub fn dump_screen_row_styles(&self, y: u32) -> Result<Vec<ghostty_vt::StyleRun>, Error> {
        let cols = self.config.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if y < lb_rows {
            Ok(self
                .line_buffer
                .visual_row_with_styles(y, cols)
                .map(|(_, r)| r)
                .unwrap_or_default())
        } else {
            let vp_row = (y - lb_rows) as u16;
            self.terminal.dump_viewport_row_style_runs(vp_row)
        }
    }

    pub fn dump_viewport_row_cell_styles(
        &self,
        row: u16,
    ) -> Result<Vec<ghostty_vt::CellStyle>, Error> {
        self.terminal.dump_viewport_row_cell_styles(row)
    }

    /// Viewport-relative style runs. Keeps the original 0..rows
    /// addressing the live grid uses (background quads, dirty-row
    /// repaint). When `scroll_offset > 0` the row addresses a position
    /// inside `line_buffer` history, so dispatch through
    /// [`Self::dump_screen_row_styles`] to stay consistent with
    /// [`Self::dump_viewport`]. Pure-scrollback callers (no viewport
    /// translation) should reach for [`Self::dump_screen_row_styles`]
    /// directly.
    pub fn dump_viewport_row_style_runs(
        &self,
        row: u16,
    ) -> Result<Vec<ghostty_vt::StyleRun>, Error> {
        if self.scroll_offset == 0 {
            return self.terminal.dump_viewport_row_style_runs(row);
        }
        let top = self.viewport_row_offset();
        let y = top + row as u32;
        self.dump_screen_row_styles(y)
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

    /// Total visible rows across the union (`line_buffer` scrollback +
    /// the live viewport). Stays in sync as captures land and rows
    /// scroll off the top of the grid.
    pub fn total_rows(&self) -> u32 {
        let cols = self.config.cols;
        self.line_buffer
            .wrapped_row_count(cols)
            .saturating_add(self.config.rows as u32)
    }

    /// Compute the ever-increasing absolute Y of the row containing the
    /// cursor right now. Captured when an OSC 133 mark fires so the
    /// mark survives `LineBuffer` ring eviction — translate back to a
    /// current-frame screen row via [`Self::abs_to_screen_row`].
    fn current_abs_y_at_cursor(&self) -> u64 {
        let lb_rows = self.line_buffer.wrapped_row_count(self.config.cols) as u64;
        // ghostty rows that scrolled out but haven't yet been captured into
        // line_buffer (capture runs at end of feed; OSC 133 may fire mid-feed).
        // Account for them so the captured abs_y matches the row's position
        // after capture lands.
        let ghostty_top = self.terminal.viewport_row_offset() as u64;
        let next_capture = self
            .last_captured_abs_row
            .map(|r| r as u64 + 1)
            .unwrap_or(0);
        let uncaptured = ghostty_top.saturating_sub(next_capture);
        let vp_row = self
            .terminal
            .cursor_position()
            .map(|(_, y)| u32::from(y.saturating_sub(1)))
            .unwrap_or(0);
        self.line_buffer.overflow() + lb_rows + uncaptured + vp_row as u64
    }

    /// Translate a stored `abs_y` (captured at OSC 133 dispatch) into
    /// a current screen-frame row. Returns `None` when the line
    /// containing it has been evicted from `LineBuffer` (or pushed
    /// past the live viewport bottom — should not happen in practice).
    pub fn abs_to_screen_row(&self, abs_y: u64) -> Option<u32> {
        let overflow = self.line_buffer.overflow();
        if abs_y < overflow {
            return None;
        }
        let rel = abs_y - overflow;
        let total = self.total_rows() as u64;
        if rel >= total {
            return None;
        }
        Some(rel as u32)
    }

    /// Absolute row index at the top of the visible viewport.
    /// `scroll_offset == 0` pins the viewport to the bottom; positive
    /// `scroll_offset` walks back into `line_buffer` scrollback.
    pub fn viewport_row_offset(&self) -> u32 {
        self.total_rows()
            .saturating_sub(self.config.rows as u32)
            .saturating_sub(self.scroll_offset)
    }

    /// Absolute line index of the topmost visible row.
    ///
    /// This is an ever-increasing value: `line_buffer.overflow() +
    /// viewport_row_offset()`. Use it to pin the viewport to a fixed reading
    /// position — the value survives grid scrolls and `LineBuffer` captures
    /// because overflow grows monotonically. Translate back to a current
    /// screen row via [`Self::abs_to_screen_row`].
    pub fn viewport_top_abs_y(&self) -> u64 {
        self.line_buffer
            .overflow()
            .saturating_add(self.viewport_row_offset() as u64)
    }

    /// Current scroll offset in rows back from the bottom of the
    /// unified frame. `0` pins the viewport to the live grid; positive
    /// values walk back into `line_buffer` history. Exposed for tests
    /// that need to assert exact scroll positions; production callers
    /// should reach for [`Self::viewport_row_offset`] instead.
    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> u32 {
        self.scroll_offset
    }

    /// Move the viewport `delta_lines` rows within the unified frame.
    /// `delta < 0` scrolls UP into history (increases `scroll_offset`),
    /// matching existing call-site convention (`scroll_viewport(-step)`
    /// for PageUp / wheel-up). Clamps to the available scrollback so
    /// `scroll_offset` never exceeds `line_buffer.wrapped_row_count`.
    pub fn scroll_viewport(&mut self, delta_lines: i32) -> Result<(), Error> {
        let max_scroll = self.line_buffer.wrapped_row_count(self.config.cols);
        let cur = self.scroll_offset as i32;
        let new_offset = (cur - delta_lines).clamp(0, max_scroll as i32);
        self.scroll_offset = new_offset as u32;
        Ok(())
    }

    /// Pin the viewport to the top of the unified frame (oldest
    /// `line_buffer` row at the top of the visible area).
    pub fn scroll_viewport_top(&mut self) -> Result<(), Error> {
        self.scroll_offset = self.line_buffer.wrapped_row_count(self.config.cols);
        Ok(())
    }

    /// Pin the viewport to the live grid (`scroll_offset = 0`).
    pub fn scroll_viewport_bottom(&mut self) -> Result<(), Error> {
        self.scroll_offset = 0;
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        self.config.cols = cols;
        self.config.rows = rows;
        let result = self.terminal.resize(cols, rows);
        // Ghostty reflows soft-wrapped rows on resize, so
        // `viewport_row_offset` may drop (widen) or rise (narrow) by an
        // unpredictable amount. Re-anchor the capture cursor against
        // the post-resize ghostty state so:
        //   1. `sync_after_ghostty_scrollback_shrink` does not interpret
        //      a widen-induced drop as a `\x1b[3J` scrollback wipe and
        //      clear LineBuffer.
        //   2. The next `capture_scrolled_out` does not re-walk rows
        //      we've already captured (LineBuffer keeps its contents
        //      as-is; ghostty's pre-viewport rows are treated as
        //      already accounted for).
        let new_top = self.terminal.viewport_row_offset();
        self.last_captured_abs_row = if new_top == 0 {
            None
        } else {
            Some(new_top - 1)
        };
        // Invalidate LineBuffer wrap cache before querying at new width.
        // Width change → all per-line cached wraps become stale. Clearing
        // ensures wrapped_row_count below recalculates from cells without
        // per-line cache hits on potentially outdated (width, rows) pairs.
        self.line_buffer.invalidate_wrap_cache();
        // `wrapped_row_count` depends on the column count; clamp so a
        // narrower viewport doesn't leave `scroll_offset` past the new
        // top of the unified frame.
        let max_scroll = self.line_buffer.wrapped_row_count(cols);
        if self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
        // Drag any column-bound mark metadata down to the new width so
        // narrowing does not leave stale start/end column pairs pointing
        // past the right margin.
        self.interval_tree.clamp_payload_cols(cols);
        result
    }

    pub fn take_dirty_viewport_rows(&mut self) -> Vec<u16> {
        self.terminal
            .take_dirty_viewport_rows(self.config.rows)
            .unwrap_or_default()
    }

    pub fn take_viewport_scroll_delta(&mut self) -> i32 {
        self.terminal.take_viewport_scroll_delta()
    }

    /// Drain one-shot grid events (alt-screen toggle, RIS) recorded by
    /// ghostty since the last call. Used by the viewport reconcile path
    /// to decide whether to invalidate selection.
    pub fn take_grid_events(&mut self) -> Vec<ghostty_vt::GridEvent> {
        self.terminal.take_grid_events()
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
