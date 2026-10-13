//! iTerm2-style logical-line scrollback buffer.
//!
//! Stores logical lines (cells joined across soft-wraps) with per-cell style
//! and hyperlink ID. Wrap to a target cell-width is computed lazily by
//! [`LineBuffer::wrap_visible`] so the buffer is immutable across viewport
//! resize.
//!
//! ## State machine
//!
//! Each [`LogicalLine`] carries an [`EolKind`]:
//! - [`EolKind::Hard`] — terminated by a hard newline; subsequent
//!   [`LineBuffer::append`] calls start a new logical line.
//! - [`EolKind::Soft`] / [`EolKind::Dwc`] — partial: subsequent appends extend
//!   the same logical line in place. `Dwc` records that ghostty inserted a
//!   `DWC_SKIP` at the right edge because a 2-cell character would have
//!   straddled the boundary.
//!
//! ## Stable references
//!
//! [`LineBufferPosition`] holds an absolute index (`overflow + idx`) so a
//! caller can hold onto a position across appends. If the line has been
//! evicted by the ring, [`LineBuffer::deref`] returns `None`.

use std::collections::VecDeque;
use std::num::NonZeroU16;

use ghostty_vt::{Rgb, StyleRun};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

mod find;
mod find_context;
mod wrap;

pub use find_context::{FindContext, FindOptions, MatchRange as FindMatchRange};

/// Default foreground used when no [`StyleRun`] covers a cell's column.
const DEFAULT_FG: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};

/// Default background used when no [`StyleRun`] covers a cell's column.
const DEFAULT_BG: Rgb = Rgb { r: 0, g: 0, b: 0 };

/// Default unicode-width fallback for characters that
/// [`UnicodeWidthChar::width`] reports as `None` (control chars).
const DEFAULT_CELL_WIDTH: u8 = 1;

/// How a logical line was terminated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EolKind {
    /// Hard newline. Next append starts a new logical line.
    Hard,
    /// Soft wrap. Next append extends this line.
    Soft,
    /// Right edge held a `DWC_SKIP` because a 2-cell character would have
    /// straddled the boundary. Next append extends this line.
    Dwc,
}

/// One cell of a logical line — character, display width, style, and
/// optional OSC 8 hyperlink ID.
#[derive(Clone, Debug)]
pub struct LbCell {
    pub ch: char,
    pub width: u8,
    pub fg: Rgb,
    pub bg: Rgb,
    /// Mirrors StyleRun::flags from ghostty_vt (bold/italic/underline/etc.)
    pub flags: u8,
    pub url_id: Option<NonZeroU16>,
}

/// A logical line — the sequence of cells joined across any soft-wraps,
/// the eol that terminated (or last extended) it, and a `text` field kept
/// in sync with `cells` for callers that want a quick string view.
#[derive(Clone, Debug)]
pub struct LogicalLine {
    pub cells: Vec<LbCell>,
    pub eol: EolKind,
    pub text: String,
    /// Wall-clock instant the line was first pushed into the buffer.
    /// Preserved across in-place extension (soft / DWC tail). Mirrors
    /// iTerm2's `LineBlockMetadata.lineMetadata` per-line timestamps.
    pub created_at: std::time::SystemTime,
    /// Cached `(visual_rows, width)` for the most recent `rows_at_width`
    /// query. `Cell` keeps `&self` callable from immutable methods so
    /// `wrapped_row_count` / `locate_visual_row` remain `&self`. Mirrors
    /// iTerm2's `cached_numlines` / `cached_numlines_width`.
    cached_wrap: std::cell::Cell<Option<(u32, u16)>>,
}

impl LogicalLine {
    /// Visual rows this line occupies when wrapped at `width`. Empty
    /// lines and `width == 0` both yield 1 (preserves the prior contract
    /// that every logical line contributes ≥1 visible row).
    ///
    /// Walks the cells using the same wrap rule as `wrap_visible` /
    /// `cells_consumed_before_sub_row` / `locate_sub_col_in_line`: a
    /// cell whose width would push past `width` starts a new row.
    /// Plain `div_ceil` would diverge from this for CJK content at
    /// odd-divisor widths (e.g. three 2-cell chars at `width=3` →
    /// `div_ceil(6, 3) = 2`, but the wrap walk produces 3 rows).
    fn rows_at_width(&self, width: u16) -> u32 {
        if let Some((rows, w)) = self.cached_wrap.get()
            && w == width
        {
            return rows;
        }
        let rows = if self.cells.is_empty() || width == 0 {
            1
        } else {
            let mut rows: u32 = 1;
            let mut row_width: u16 = 0;
            let mut row_has_cells = false;
            for cell in &self.cells {
                let cw = cell.width as u16;
                if cw == 0 {
                    // Zero-width cells (combining marks) attach to the
                    // preceding cell — neither advance the column nor
                    // start a new row.
                    continue;
                }
                if row_width.saturating_add(cw) > width && row_has_cells {
                    rows = rows.saturating_add(1);
                    row_width = cw;
                } else {
                    row_width = row_width.saturating_add(cw);
                }
                row_has_cells = true;
            }
            rows
        };
        self.cached_wrap.set(Some((rows, width)));
        rows
    }

    /// Drop the cached `(rows, width)` pair. Called on any mutation that
    /// changes `cells` (append/extend paths in `LineBuffer::append`).
    fn invalidate_wrap_cache(&self) {
        self.cached_wrap.set(None);
    }
}

/// Stable reference to a logical line. Survives appends; becomes
/// dangling (returns `None` from [`LineBuffer::deref`]) once the line
/// is evicted — either by ring overflow or by [`LineBuffer::clear`],
/// which absorbs the cleared count into `overflow` and so pushes every
/// outstanding position below the live range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineBufferPosition {
    pub abs_index: u64,
}

/// Why an [`LineBuffer::append_at_or_after`] call was refused. Producers
/// of capture deltas (notably `TerminalSession::capture_scrolled_out`)
/// inspect this to log the desync site instead of silently corrupting
/// the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendError {
    /// `target_abs` lies below the buffer's `next_append_abs()` — the
    /// producer is trying to push a row that the buffer already holds
    /// (or has already evicted). Indicates a double-count somewhere
    /// upstream; the safe response is to skip and keep going.
    AlreadyAppended { target: u64, next: u64 },
    /// `target_abs` lies above the buffer's `next_append_abs()` — the
    /// producer claims to have scrolled past rows it never delivered.
    /// The buffer refuses to insert placeholder lines; the caller
    /// either lost intermediate rows or its `target_abs` math is wrong.
    GapWouldOpen { target: u64, next: u64 },
}

impl std::fmt::Display for AppendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyAppended { target, next } => write!(
                f,
                "append_at_or_after({target}) refused: buffer already at next={next} (would double-append)",
            ),
            Self::GapWouldOpen { target, next } => write!(
                f,
                "append_at_or_after({target}) refused: buffer at next={next} (gap of {} rows)",
                target - next,
            ),
        }
    }
}

impl std::error::Error for AppendError {}

/// Bounded FIFO of [`LogicalLine`]s. Evicts the oldest line on overflow
/// and bumps an `overflow` counter so [`LineBufferPosition`] math stays
/// monotonic across the buffer's lifetime.
pub struct LineBuffer {
    lines: VecDeque<LogicalLine>,
    max_lines: usize,
    overflow: u64,
}

impl LineBuffer {
    /// Construct an empty buffer that retains at most `max_lines`
    /// logical lines before evicting from the front.
    pub fn new(max_lines: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            max_lines,
            overflow: 0,
        }
    }

    /// Append `text` to the buffer. If the previous tail is partial
    /// ([`EolKind::Soft`] or [`EolKind::Dwc`]), the existing line is
    /// extended in place; otherwise a new logical line is pushed. The
    /// tail's [`EolKind`] is then set to `eol`.
    ///
    /// `runs` carry per-cell style. Column indices in `runs` are 1-based
    /// inclusive (`start_col = 1` covers the first cell), matching the
    /// row-local coordinates that
    /// `ghostty_vt::Terminal::dump_viewport_row_style_runs` returns. A
    /// 2-cell char spans `[col, col+1]`. Cells outside any run use the
    /// default fg/bg/flags.
    pub fn append(&mut self, text: &str, runs: &[StyleRun], eol: EolKind) {
        let extend = self.would_extend_partial(text, eol);

        if extend {
            // Safe: `extend` is true only when `lines.back()` is `Some`.
            let line = self.lines.back_mut().expect("partial tail exists");
            line.invalidate_wrap_cache();
            // `runs` are segment-relative (indexed from column 1 of the
            // incoming `text`), so use a fresh `seg_col` for style lookup
            // rather than the logical line's cumulative width.
            let mut seg_col: u16 = 1;
            for ch in text.chars() {
                let cell = build_cell(ch, seg_col, runs);
                seg_col = seg_col.saturating_add(cell.width as u16);
                line.text.push(ch);
                line.cells.push(cell);
            }
            line.eol = eol;
        } else {
            let mut line = LogicalLine {
                cells: Vec::with_capacity(text.len()),
                eol,
                text: String::with_capacity(text.len()),
                created_at: std::time::SystemTime::now(),
                cached_wrap: std::cell::Cell::new(None),
            };
            let mut col: u16 = 1;
            for ch in text.chars() {
                let cell = build_cell(ch, col, runs);
                col = col.saturating_add(cell.width as u16);
                line.text.push(ch);
                line.cells.push(cell);
            }
            self.lines.push_back(line);
            while self.lines.len() > self.max_lines {
                self.lines.pop_front();
                self.overflow = self.overflow.saturating_add(1);
            }
        }
    }

    /// Attach per-cell OSC 8 URL IDs to the tail logical line's cells.
    /// `url_ids[i]` corresponds to the i-th cell of the just-appended
    /// segment (right-aligned within the line if the line was extended).
    /// Zero entries are dropped (no hyperlink).
    ///
    /// Call after [`Self::append`]. No-op if the buffer is empty.
    pub fn attach_url_ids_to_tail(&mut self, url_ids: &[u16]) {
        let Some(line) = self.lines.back_mut() else {
            return;
        };
        let n = line.cells.len();
        let offset = n.saturating_sub(url_ids.len());
        for (i, &id) in url_ids.iter().enumerate() {
            let Some(cell) = line.cells.get_mut(offset + i) else {
                break;
            };
            cell.url_id = NonZeroU16::new(id);
        }
    }

    /// Flip a partial tail (`Soft` / `Dwc`) to `Hard`. No-op otherwise.
    pub fn seal_partial(&mut self) {
        if let Some(line) = self.lines.back_mut()
            && matches!(line.eol, EolKind::Soft | EolKind::Dwc)
        {
            line.eol = EolKind::Hard;
        }
    }

    /// Invalidate per-line wrap caches. Called when viewport width changes
    /// so subsequent `wrapped_row_count` / `locate_visual_row` queries
    /// recalculate from cells without stale cached wraps.
    pub fn invalidate_wrap_cache(&mut self) {
        for line in &self.lines {
            line.invalidate_wrap_cache();
        }
    }

    /// Drop every logical line. `overflow` is bumped by the cleared
    /// count so [`Self::next_append_abs`] is monotonic across the
    /// wipe — the new abs index a fresh append claims sits *past* the
    /// cleared range, never aliasing it.
    ///
    /// Mirrors iTerm2's `clear` + pre-flush pattern: iTerm2 flushes
    /// the grid into the LineBuffer before clearing, which extends
    /// `cumulativeScrollbackOverflow` (the wrap-aware caller's
    /// version of `overflow`) across the wiped content; marks above
    /// the wipe stay valid because the new overflow grew past them.
    /// Daruda's marks are logical-line indexed, so `overflow +=
    /// lines.len()` absorbs the wipe in full — no caller-side
    /// visual-row correction is required. The caller still needs to
    /// drop marks anchored inside the wiped logical range so they
    /// don't alias future appends; see
    /// `TerminalSession::clear_line_buffer_and_drop_history_marks`.
    pub fn clear(&mut self) {
        // `lines.len() as u64` cast is safe — `usize` is at most 64-bit
        // on every supported target. `saturating_add` is for the
        // 128-bit-cosmic-ray edge; `overflow` never realistically
        // approaches `u64::MAX`.
        let cleared = self.lines.len() as u64;
        self.lines.clear();
        self.overflow = self.overflow.saturating_add(cleared);
    }

    /// Number of logical lines currently retained (post-eviction).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `true` when no logical lines are retained.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Counts logical lines dropped by ring eviction since construction.
    /// Preserved across [`Self::clear`] so [`LineBufferPosition`]s issued
    /// before the clear remain invalidated rather than aliasing.
    pub fn overflow(&self) -> u64 {
        self.overflow
    }

    /// Absolute index the next [`Self::append`] would assign to a *new*
    /// logical line (i.e. when the tail is not partial). Equals
    /// `overflow() + len() as u64`. Soft-tail extension does not advance
    /// this value; an append-then-soft-tail observer should re-read.
    ///
    /// Mirrors iTerm2's `firstAbsoluteLineNumber + numberOfLines`
    /// accounting at the line layer. Lets callers express capture state
    /// in the buffer's own abs-y space — see
    /// `TerminalSession::capture_scrolled_out` — instead of in the
    /// producer's row-index space, where any reset (resize, alt-screen,
    /// wipe) would require bespoke synchronization.
    pub fn next_append_abs(&self) -> u64 {
        self.overflow + self.lines.len() as u64
    }

    /// Like [`Self::append`] but conditioned on the caller-asserted
    /// absolute index `target_abs`:
    ///
    /// - `target_abs == self.next_append_abs()` → behaves identically to
    ///   `append` (the happy path).
    /// - `target_abs < self.next_append_abs()` → returns
    ///   [`AppendError::AlreadyAppended`]; the call is a no-op. A
    ///   producer that over-reports its scroll delta (a phantom
    ///   `take_scrolled_rows` spike, a stale watermark) is surfaced
    ///   instead of silently double-appending the same row.
    /// - `target_abs > self.next_append_abs()` → returns
    ///   [`AppendError::GapWouldOpen`]; the call is a no-op. The buffer
    ///   refuses to invent placeholders for rows the producer claimed
    ///   to have scrolled past but never delivered.
    ///
    /// On `Ok`, returns the abs index a *new* line append would claim
    /// after this call. For a hard-EOL push that equals `target_abs + 1`;
    /// for a soft-tail extension (which folds into the existing tail) it
    /// equals `target_abs`. Callers iterating row-by-row should use the
    /// returned value as the `target_abs` of the next call.
    pub fn append_at_or_after(
        &mut self,
        target_abs: u64,
        text: &str,
        runs: &[StyleRun],
        eol: EolKind,
    ) -> Result<u64, AppendError> {
        let next = self.next_append_abs();
        if target_abs < next {
            return Err(AppendError::AlreadyAppended {
                target: target_abs,
                next,
            });
        }
        if target_abs > next {
            return Err(AppendError::GapWouldOpen {
                target: target_abs,
                next,
            });
        }
        // Soft-tail extension folds into the existing tail and does not
        // advance `next_append_abs`; a new push moves it forward by one,
        // capped at the eviction floor (eviction increments `overflow`
        // by the same amount it shrinks `len`, so `next_append_abs` is
        // monotonic regardless). The predicate must match `append`'s own
        // branch decision exactly — sharing one helper rather than two
        // parallel `matches!` arms keeps a future `EolKind` variant from
        // silently diverging the return value from the actual buffer state.
        let extends_partial = self.would_extend_partial(text, eol);
        self.append(text, runs, eol);
        Ok(if extends_partial {
            target_abs
        } else {
            target_abs + 1
        })
    }

    /// True when a subsequent [`Self::append`] / [`Self::append_at_or_after`]
    /// with `(text, eol)` would fold into the existing tail instead of
    /// pushing a new logical line. Sole source of truth for that
    /// decision — both append paths consult it so their behavior stays
    /// lockstep across future `EolKind` additions.
    ///
    /// iTerm2 `LineBlock.mm:864` treats an empty Hard append after a
    /// partial tail as a fresh logical line rather than silently sealing
    /// the previous one — preserves the distinction between "kernel
    /// printed a partial chunk" and "kernel terminated with a newline".
    fn would_extend_partial(&self, text: &str, eol: EolKind) -> bool {
        matches!(
            self.lines.back().map(|l| l.eol),
            Some(EolKind::Soft) | Some(EolKind::Dwc)
        ) && !(text.is_empty() && eol == EolKind::Hard)
    }

    /// Borrow the logical line at ring-local index `idx`, or `None` if
    /// out of range.
    pub fn get(&self, idx: usize) -> Option<&LogicalLine> {
        self.lines.get(idx)
    }

    /// Remove and return the newest logical line, or `None` when empty.
    /// The inverse of a single new-line [`Self::append`]: `overflow` is
    /// left untouched, so [`Self::next_append_abs`] (`overflow + len`)
    /// rewinds by exactly one. Used to undo tail lines that re-entered the
    /// live viewport after a widen reflow — see
    /// `TerminalSession::pop_verified_reentered_lines`. Callers querying
    /// wrapped row counts afterward must invalidate the wrap cache (the
    /// resize path already does so globally).
    pub fn pop_tail(&mut self) -> Option<LogicalLine> {
        self.lines.pop_back()
    }

    /// Absolute index of the newest retained logical line, or `None` when
    /// empty. Equals `next_append_abs() - 1` for a non-empty buffer; the
    /// stable [`LineBufferPosition`] basis for evicting marks anchored on
    /// a line about to be popped.
    pub fn last_abs(&self) -> Option<u64> {
        if self.lines.is_empty() {
            None
        } else {
            Some(self.next_append_abs() - 1)
        }
    }

    /// Stable [`LineBufferPosition`] of the oldest still-live line, or
    /// `None` if the buffer is empty. Delegates to `position_at(0)` so
    /// the equivalence is enforced by code rather than comment.
    pub fn min_position(&self) -> Option<LineBufferPosition> {
        self.position_at(0)
    }

    /// Issue a stable [`LineBufferPosition`] for ring-local index `idx`.
    /// Returns `None` if `idx` is out of range.
    pub fn position_at(&self, idx: usize) -> Option<LineBufferPosition> {
        if idx < self.lines.len() {
            Some(LineBufferPosition {
                abs_index: self.overflow + idx as u64,
            })
        } else {
            None
        }
    }

    /// Resolve a [`LineBufferPosition`] to a borrow, or `None` if the
    /// line has been evicted (by ring overflow or [`Self::clear`]).
    pub fn deref(&self, pos: &LineBufferPosition) -> Option<&LogicalLine> {
        if pos.abs_index >= self.overflow && pos.abs_index < self.overflow + self.lines.len() as u64
        {
            // `try_from` over `as usize` so the cast is checked on
            // 32-bit targets — the ring length fits in `usize` by
            // construction but `abs_index - overflow` is `u64`.
            let local = usize::try_from(pos.abs_index - self.overflow).ok()?;
            self.lines.get(local)
        } else {
            None
        }
    }
}

/// Build one cell from `ch` at column `col`. Style is looked up from
/// `runs` whose `[start_col, end_col]` range (inclusive, cell-based)
/// covers `col`; absent runs fall back to the module defaults.
fn build_cell(ch: char, col: u16, runs: &[StyleRun]) -> LbCell {
    let width = UnicodeWidthChar::width(ch)
        .map(|w| w as u8)
        .unwrap_or(DEFAULT_CELL_WIDTH);
    let style = runs
        .iter()
        .find(|r| col >= r.start_col && col <= r.end_col)
        .map(|r| (r.fg, r.bg, r.flags))
        .unwrap_or((DEFAULT_FG, DEFAULT_BG, 0));
    LbCell {
        ch,
        width,
        fg: style.0,
        bg: style.1,
        flags: style.2,
        url_id: None,
    }
}

#[cfg(test)]
mod tests;
