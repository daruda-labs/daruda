use std::collections::VecDeque;
use std::time::{Duration, Instant};

use daruda_core::process_env;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use ghostty_vt::{Error, Rgb, Terminal};

use crate::ansi;
use crate::coords::ViewportRow;
use crate::session::interval_tree::{
    IntervalTree, LineCoord, LineRange, MarkId, MarkPayload, MarkRecordSink,
};
use crate::vt_codes::{
    AttentionKind, CSI_ALT_SCREEN, CSI_ALT_SCREEN_LEGACY, CSI_ALT_SCREEN_SAVE_CURSOR, CsiMode,
    NotificationRequest, OSC_DEFAULT_BG, OSC_DEFAULT_FG,
};
use crate::vt_limits::{PARSE_TAIL_LIMIT, PROMPT_MARKS_CAP};
use crate::{TerminalConfig, TerminalDims};

mod annotation_ops;
mod command_marks;
mod feed;
pub mod interval_tree;
mod line_buffer;
mod logical_abs;
mod modes;
mod output_state;
mod resize;
mod scanners;
mod viewport;

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
/// 133. Captured with an *absolute* logical-line index that includes
/// lines already evicted from `LineBuffer` (via `LineBuffer::overflow()`),
/// so marks survive ring eviction. `abs_y` counts Hard-EOL line
/// boundaries — wrap-invariant under resize — and is translated to a
/// current-frame screen row via [`TerminalSession::abs_to_screen_row`]
/// before indexing into `dump_screen_row`. Mirrors iTerm2
/// `VT100Terminal.m:4520-4616` (which also separates the wrap-blind abs
/// from the wrap-aware projection).
///
/// **Equality is not derived.** Adding `seq` made the auto-derived
/// `PartialEq` an ambiguous mix of "same identity" (would compare seq)
/// and "same data" (would compare every field), so it is removed. Callers
/// must say what they mean explicitly: identity → `a.seq == b.seq`;
/// content match → compare specific fields. Compile-fails any
/// `mark_a == mark_b` until the intent is spelled out.
#[derive(Clone, Copy, Debug)]
pub struct PromptMark {
    pub kind: PromptMarkKind,
    /// Monotonic push-order identity, assigned exclusively in
    /// [`TerminalSession::push_prompt_mark`] from a private counter.
    /// Unlike `abs_y` (whose owning mark can be dropped by
    /// `clear_line_buffer_and_drop_history_marks` after a `\x1b[3J` / RIS
    /// scrollback wipe), `seq` is never reused and never resets. Use
    /// it as the position-independent identity for jump-focus
    /// tracking — analogous to iTerm2's `_selectedScreenMark` weak
    /// ref, which survives the mark's position changing.
    ///
    /// `pub(crate)` rather than `pub`: this blocks external crates from
    /// constructing `PromptMark` literals (every field must be visible
    /// for struct-literal syntax), forcing them to go through
    /// [`PromptMarkInit`] + `push_prompt_mark`, which is the single
    /// source of truth for the monotonic invariant.
    pub(crate) seq: u64,
    /// Ever-increasing **logical-line** absolute index at the time the
    /// mark fired, computed as `line_buffer.overflow() +
    /// line_buffer.len() + logical_lines_until_cursor()` (see
    /// [`TerminalSession::current_abs_y_at_cursor`]). Units are logical
    /// lines (one per Hard EOL), **not** visual rows — re-flow at a
    /// new width does not move the mark. Stable across `LineBuffer`
    /// ring eviction and across `\x1b[3J` / RIS scrollback wipes for
    /// surviving marks (the wipe absorbs cleared logical lines into
    /// `LineBuffer::overflow` and drops marks anchored inside the
    /// wiped region without shifting the rest — see
    /// `clear_line_buffer_and_drop_history_marks`). Translate back to a
    /// current-frame screen row via [`TerminalSession::abs_to_screen_row`].
    ///
    /// Stored as [`LogicalLineAbs`] so a future caller cannot pass a
    /// visual-row anchor (e.g. `viewport_top_abs_y()`'s return) here
    /// by accident — the type check rejects it at the seam.
    pub abs_y: LogicalLineAbs,
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

/// Init payload for [`TerminalSession::push_prompt_mark`]. Carries every
/// [`PromptMark`] field except `seq`, which the session assigns from its
/// monotonic counter — making "every mark in `prompt_marks` has a seq
/// from the counter" the only constructable shape inside the module, so a
/// bypassed `push_prompt_mark` can't silently violate the uniqueness
/// invariant with a placeholder `seq`.
struct PromptMarkInit {
    kind: PromptMarkKind,
    abs_y: LogicalLineAbs,
    screen_col: u16,
    exit_code: Option<i32>,
    timestamp: std::time::SystemTime,
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
    dims: TerminalDims,
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
    /// Format from shell: `file://hostname/path`. Kept only when the host is
    /// this machine, as the local path. None until the first one lands.
    cwd: Option<String>,
    clipboard_write: Option<String>,
    parse_tail: Vec<u8>,
    dsr_state: DsrScanState,
    osc_query_state: OscQueryScanState,
    osc133_scanner: Osc133Scanner,
    capability_scanner: CapabilityScanner,
    xtgettcap_scanner: XtGetTcapScanner,
    prompt_marks: VecDeque<PromptMark>,
    /// Monotonic counter assigning [`PromptMark::seq`] on push. Never
    /// resets across `\x1b[3J` / RIS wipes, so it is safe to use as the
    /// position-independent identity of a focused mark.
    prompt_mark_seq: u64,
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
    /// LineBuffer abs index the *next* capture would assign to a new
    /// logical line, snapshotted at the end of the previous
    /// `capture_scrolled_out`. Equal to `line_buffer.next_append_abs()`
    /// at the moment of that snapshot. `None` until the first capture;
    /// reset on alt-screen entry and `\x1b[3J` / RIS wipe (the
    /// LineBuffer / grid relationship that defined the snapshot no
    /// longer applies).
    ///
    /// Lives in LineBuffer abs space — not ghostty's row-index space —
    /// so resize, alt-screen toggle, and ring saturation don't require
    /// re-projecting it. The wipe-detection check in
    /// `sync_after_ghostty_scrollback_shrink` reads only its
    /// `is_none()` state; uncaptured-row counting at OSC 133 dispatch
    /// reads `Terminal::peek_scrolled_rows` directly from ghostty.
    last_captured_lb_abs: Option<u64>,
    /// Daruda-owned scroll position in rows back from the bottom. `0`
    /// pins the viewport to the live grid; positive values reveal rows
    /// from `line_buffer` above the live viewport. Replaces ghostty's
    /// internal `scroll_viewport` state — we no longer rely on its
    /// scrollback for navigation; ghostty's retained scrollback is limited
    /// to a tiny capture window (see `GHOSTTY_TRANSIENT_SCROLLBACK`).
    scroll_offset: u32,
    /// Augmented interval tree storing user-authored marks (annotations
    /// today; prompt regions and search hits may become future mark types).
    /// Lifecycle is
    /// wired through `capture_scrolled_out` (eviction), `resize` (column
    /// clamp), and the alt-screen toggle (visibility filter). See
    /// `crate::session::annotation_ops` for the public API.
    pub(in crate::session) interval_tree: IntervalTree<MarkPayload>,
}

/// Rows ghostty retains as a transient buffer so `capture_scrolled_out`
/// can read a row after the feed that scrolled it off the active area but
/// before the next feed evicts it from ghostty too. Daruda's own scrollback
/// lives in `LineBuffer`; ghostty's ring exists purely to bridge "row
/// scrolled out of the live grid" → "row copied into LineBuffer". Sized as
/// `max(GHOSTTY_TRANSIENT_SCROLLBACK, rows * 2)` in `new()` so taller
/// terminals always have at least a viewport's worth of headroom.
///
/// `capture_scrolled_out` tracks how many rows scrolled via a monotonic
/// tracked-pin counter (`Terminal::take_scrolled_rows`), so steady-state
/// retention is NOT bounded by this ring — `LineBuffer` grows to its own
/// `max_scrollback`. This ring only bounds a single oversized `feed()`:
///
/// LIMITATION: a single `feed()` that scrolls more rows than
/// `max(GHOSTTY_TRANSIENT_SCROLLBACK, rows * 2)` out of the active area
/// loses the oldest ones (they evict from ghostty before
/// `capture_scrolled_out` runs at the end of the feed). Sized to absorb a
/// typical interactive burst (tool output, a screenful of paste) in one
/// 16 ms frame; only a pathological bulk dump (`cat huge_file`) in a single
/// feed still drops its oldest lines. A future improvement: invoke
/// `capture_scrolled_out` from within ghostty's eviction hook so no row can
/// be evicted without being snapshotted first (iTerm2's synchronous model).
const GHOSTTY_TRANSIENT_SCROLLBACK: usize = 2048;

/// Logical-line absolute index inside the unified `LineBuffer` + grid
/// frame. Computed as
/// `LineBuffer::overflow() + LineBuffer::len() + Hard-EOL-count-above-cursor`
/// (see [`TerminalSession::current_abs_y_at_cursor`]). Wrap-blind: the
/// value survives viewport resize without re-projection.
///
/// **Distinct namespace** from `LineCoord::Viewport::abs_y`
/// (visual-row space, `overflow + visual_row`), even though both wrap
/// a `u64`. Mixing them — passing a visual-row anchor to
/// [`TerminalSession::abs_to_screen_row`], or storing a logical-line
/// abs as a `LineCoord::Viewport.abs_y` — silently mis-projects under
/// wrap-inflated widths. The newtype turns that mistake into a
/// compile error wherever this type is named on the seam, and the
/// shared-type sites that remain (the interval tree's `Viewport`
/// variant) keep their visual-row contract explicit in comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct LogicalLineAbs(pub u64);

impl LogicalLineAbs {
    /// Unwrap to the raw `u64`. Used at the rare arithmetic sites
    /// (mark abs differences, `saturating_sub` against `overflow`)
    /// where the newtype's monoid is insufficient.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for LogicalLineAbs {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl std::fmt::Display for LogicalLineAbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Classify a ghostty row's wrap kind as "ends a logical line" or
/// "soft-wraps into the next row". Defined as a free function with
/// an exhaustive `match` so a future [`ghostty_vt::WrapKind`] variant
/// makes both call sites a compile error instead of silently
/// counting the new variant as "not Hard". Sole interpretation of
/// the wrap-kind → logical-line-boundary mapping inside this crate.
fn is_hard_eol(kind: ghostty_vt::WrapKind) -> bool {
    match kind {
        ghostty_vt::WrapKind::Hard => true,
        ghostty_vt::WrapKind::Soft => false,
    }
}

impl TerminalSession {
    pub fn new(dims: TerminalDims, config: TerminalConfig) -> Result<Self, Error> {
        // Ghostty keeps a small ring so `capture_scrolled_out` can still
        // read evicted rows before the next feed drops them; daruda's
        // own scrollback lives in `line_buffer`.
        let ghostty_scrollback = GHOSTTY_TRANSIENT_SCROLLBACK.max((dims.rows as usize) * 2);
        let mut terminal = Terminal::with_scrollback(dims.cols, dims.rows, ghostty_scrollback)?;
        terminal.set_default_colors(config.default_fg, config.default_bg);
        if let Some(palette) = &config.palette {
            for (i, [r, g, b]) in palette.iter().enumerate() {
                let seq = crate::ansi::osc4_set_color(i as u8, *r, *g, *b);
                let _ = terminal.feed(seq.as_bytes());
            }
        }
        Ok(Self {
            dims,
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
            prompt_mark_seq: 0,
            alt_screen: false,
            screen_changed: false,
            prompt_arrived: false,
            pending_attention: None,
            pending_notification: None,
            command_started_at: None,
            pending_finished_command_elapsed: None,
            pending_clear_scrollback: false,
            line_buffer: LineBuffer::new(config.max_scrollback),
            last_captured_lb_abs: None,
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
        let cols = self.dims.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if row < lb_rows {
            self.line_buffer
                .position_for_visual_row(row, cols)
                .map(|(pos, _, _)| LineCoord::Buffered(pos))
        } else {
            // Viewport region. `LineCoord::Viewport.abs_y` lives in
            // **visual-row** space (`overflow + row`); a separate
            // coordinate system from [`PromptMark::abs_y`], which is now
            // logical-line based. Migrating `LineCoord::Viewport` is a
            // separate surgery — wrap-projection's predictive abs is
            // ill-defined for the interval-tree consumers — so do not
            // collapse the two spaces by reusing the mark-side
            // translator on this value.
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
    /// Current limitation: rows in the live ghostty viewport (`screen_row >=
    /// lb_rows`) always return `false` — ghostty_vt does not expose a
    /// wrap-continuation predicate for viewport rows in a way that can be
    /// consumed here without new FFI. The annotation-paint duplication bug
    /// (resize-wrap) is triggered by rows already captured in `LineBuffer`,
    /// so this limitation is acceptable for the current annotation support.
    pub fn is_wrap_continuation(&self, screen_row: u32) -> bool {
        let cols = self.dims.cols;
        let lb_rows = self.line_buffer.wrapped_row_count(cols);
        if screen_row >= lb_rows {
            // Viewport-region wrap continuations are not detected;
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
