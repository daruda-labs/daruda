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

mod find_context;

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
    /// `TerminalSession::clear_line_buffer_and_shift_marks`.
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

    /// Total visual-row count when wrapped at `cell_cols`. Each logical
    /// line contributes at least one row (an empty line still occupies
    /// one visible row).
    pub fn wrapped_row_count(&self, cell_cols: u16) -> u32 {
        if cell_cols == 0 {
            return self.lines.len() as u32;
        }
        let mut total: u32 = 0;
        for line in &self.lines {
            total = total.saturating_add(line.rows_at_width(cell_cols));
        }
        total
    }

    /// Visual rows occupied by the prefix `lines[0..idx]` when wrapped at
    /// `cell_cols`. `idx == 0` returns `0`; `idx >= len()` returns the
    /// full [`Self::wrapped_row_count`]. Same wrap rule as
    /// [`Self::wrapped_row_count`] so the two stay in sync.
    ///
    /// Used by [`crate::session::TerminalSession::abs_to_screen_row`] to
    /// project a logical-line abs that lands inside `LineBuffer` (line
    /// index `idx`) onto its first visual row — keeping
    /// `LogicalLine::rows_at_width` private to the line-buffer module.
    pub fn visual_rows_through(&self, idx: usize, cell_cols: u16) -> u32 {
        if cell_cols == 0 {
            // Mirror `wrapped_row_count`'s zero-cols fallback: every line
            // contributes one row, capped at the prefix length.
            return idx.min(self.lines.len()) as u32;
        }
        let bound = idx.min(self.lines.len());
        let mut total: u32 = 0;
        for line in self.lines.iter().take(bound) {
            total = total.saturating_add(line.rows_at_width(cell_cols));
        }
        total
    }

    /// Translate a flat visual-row index `y` (over the whole wrapped
    /// buffer) into `(logical_line_idx, sub_row_within_line)`.
    ///
    /// Returns `None` if `y >= wrapped_row_count(cell_cols)` or
    /// `cell_cols == 0`. The walk mirrors `wrapped_row_count` so the
    /// two stay in sync.
    pub fn locate_visual_row(&self, y: u32, cell_cols: u16) -> Option<(usize, usize)> {
        if cell_cols == 0 {
            return None;
        }
        let mut accum: u32 = 0;
        for (idx, line) in self.lines.iter().enumerate() {
            let rows = line.rows_at_width(cell_cols);
            let next = accum.saturating_add(rows);
            if y < next {
                let sub = (y - accum) as usize;
                return Some((idx, sub));
            }
            accum = next;
        }
        None
    }

    /// Resolve a viewport-resident mark's `abs_y` (defined as
    /// `overflow + visual_row` at mark-creation time) into a stable
    /// [`LineBufferPosition`] once the row has scrolled into this
    /// buffer. Returns `None` if `abs_y` no longer maps to a live row
    /// — either because it sits in the active grid still (`>=
    /// overflow + wrapped_row_count`) or has been evicted (`< overflow`).
    ///
    /// Single source of truth for the viewport→buffered rebind that
    /// `TerminalSession::capture_scrolled_out` runs at end-of-feed; pulls
    /// the math out of the call site so future variant changes have one
    /// place to update.
    pub fn rebind_viewport_abs(&self, abs_y: u64, cell_cols: u16) -> Option<LineBufferPosition> {
        let overflow = self.overflow;
        let lb_rows = self.wrapped_row_count(cell_cols) as u64;
        if abs_y < overflow || abs_y >= overflow + lb_rows {
            return None;
        }
        let row_in_lb = (abs_y - overflow) as u32;
        self.position_for_visual_row(row_in_lb, cell_cols)
            .map(|(pos, _, _)| pos)
    }

    /// Promote a visual-row index `y` (at the current `cell_cols`) to a
    /// stable [`LineBufferPosition`] plus the wrap-local `(sub_row,
    /// sub_col_origin)` pair.
    ///
    /// - `sub_row` is which wrapped row within the logical line `y`
    ///   lands on (0-indexed).
    /// - `sub_col_origin` is the **cumulative cell column** within the
    ///   logical line at the *start* of the visual row — i.e. the cells
    ///   consumed by sub_rows `0..sub_row`. Callers add their own
    ///   within-row cell column to recover a width-invariant anchor.
    ///
    /// The inverse [`Self::coordinate_for_position`] takes the same
    /// "cumulative cell column within the logical line" so the two
    /// round-trip across resize.
    ///
    /// Returns `None` when `y` is out of range or `cell_cols == 0`.
    pub fn position_for_visual_row(
        &self,
        y: u32,
        cell_cols: u16,
    ) -> Option<(LineBufferPosition, u32, u16)> {
        let (idx, sub) = self.locate_visual_row(y, cell_cols)?;
        let pos = self.position_at(idx)?;
        let line = self.lines.get(idx)?;
        let sub_col_origin = cells_consumed_before_sub_row(line, cell_cols, sub);
        Some((pos, sub as u32, sub_col_origin))
    }

    /// Inverse of [`Self::position_for_visual_row`]. Given a stable
    /// position and a **cumulative cell column within the logical
    /// line** (`sub_col`), return the wrapped `(visual_y,
    /// visual_col_within_row)` at the current `cell_cols`.
    ///
    /// `visual_y` is a flat row index over the entire wrapped buffer
    /// (the same coordinate space as [`Self::locate_visual_row`]).
    /// `visual_col_within_row` is the cell column inside the wrapped
    /// row (0-indexed; in `[0, cell_cols]` — `cell_cols` itself when
    /// `sub_col` lands exactly on the wrap boundary).
    ///
    /// Returns `None` when the position has been evicted or `cell_cols
    /// == 0`.
    pub fn coordinate_for_position(
        &self,
        pos: &LineBufferPosition,
        cell_cols: u16,
        sub_col: u16,
    ) -> Option<(u32, u16)> {
        if cell_cols == 0 {
            return None;
        }
        if pos.abs_index < self.overflow {
            return None;
        }
        // Checked narrowing — same rationale as `deref`.
        let local = usize::try_from(pos.abs_index - self.overflow).ok()?;
        if local >= self.lines.len() {
            return None;
        }
        // Sum visual rows of every prior logical line — same walk as
        // wrapped_row_count, just bounded.
        let mut visual_y: u32 = 0;
        for line in self.lines.iter().take(local) {
            visual_y = visual_y.saturating_add(line.rows_at_width(cell_cols));
        }
        let line = &self.lines[local];
        let (sub_row, col_in_row) = locate_sub_col_in_line(line, cell_cols, sub_col);
        Some((visual_y.saturating_add(sub_row), col_in_row))
    }

    /// Run [`FindContext`] over the buffer and project each match into
    /// `(visual_row, start_col, end_col)` triples at the given
    /// `cell_cols`. Columns are 1-indexed inclusive (matching the
    /// `dump_viewport_row_style_runs` convention). One triple per
    /// visual row spanned by a match — cross-line / wrap-spanning hits
    /// expand into multiple triples in stream order.
    ///
    /// Returns an empty vector when `cell_cols == 0` or the needle is
    /// empty / invalid.
    pub fn find_matches(
        &self,
        needle: &str,
        opts: FindOptions,
        cell_cols: u16,
    ) -> Vec<(u32, u16, u16)> {
        if cell_cols == 0 {
            return Vec::new();
        }
        let mut out: Vec<(u32, u16, u16)> = Vec::new();
        let mut ctx = FindContext::new(needle, opts);
        while let Some(m) = ctx.next_match(self) {
            self.project_match_into(&m, cell_cols, &mut out);
        }
        out
    }

    fn project_match_into(
        &self,
        m: &FindMatchRange,
        cell_cols: u16,
        out: &mut Vec<(u32, u16, u16)>,
    ) {
        for line_idx in m.start_line..=m.end_line {
            let Some(line) = self.lines.get(line_idx) else {
                continue;
            };
            let Some(pos) = self.position_at(line_idx) else {
                continue;
            };
            let byte_start = if line_idx == m.start_line {
                m.start_byte
            } else {
                0
            };
            let byte_end = if line_idx == m.end_line {
                m.end_byte
            } else {
                line.text.len()
            };
            let (cell_col_start, cell_col_end) =
                byte_range_to_cell_cols(line, byte_start, byte_end);
            let Some((y_start, col_start)) =
                self.coordinate_for_position(&pos, cell_cols, cell_col_start)
            else {
                continue;
            };
            let Some((y_end, col_end)) =
                self.coordinate_for_position(&pos, cell_cols, cell_col_end)
            else {
                continue;
            };
            for y in y_start..=y_end {
                let s = if y == y_start { col_start } else { 0 };
                let e = if y == y_end { col_end } else { cell_cols };
                if s >= e {
                    continue;
                }
                // 1-indexed inclusive: cell `s` (0-indexed) maps to
                // `start_col = s + 1`; cell `e - 1` (the last covered
                // cell) maps to `end_col = e`.
                out.push((y, s.saturating_add(1), e));
            }
        }
    }

    /// Walk forward from logical line `start_idx`, yielding visual rows
    /// wrapped at `cell_cols`. Returns up to `rows` rows of plain text.
    ///
    /// CJK-aware: a 2-cell char that would straddle the right margin
    /// pushes onto the next visual row and the previous row's effective
    /// width is `cell_cols - 1`.
    ///
    /// Degenerate case: when `cell_cols == 1` a 2-cell character still
    /// has no row wide enough to hold it; the implementation places it
    /// on its own row (overflowing the nominal width by 1) rather than
    /// dropping it. Callers that render this output are expected to clip
    /// or substitute as appropriate.
    pub fn wrap_visible(&self, start_idx: usize, rows: usize, cell_cols: u16) -> Vec<String> {
        self.walk_visual_rows(start_idx, rows, cell_cols, |row_text, _runs| row_text)
    }

    /// Same as [`Self::wrap_visible`] but each row carries the style
    /// runs that cover it, rebased to a row-local column origin.
    pub fn wrap_visible_with_styles(
        &self,
        start_idx: usize,
        rows: usize,
        cell_cols: u16,
    ) -> Vec<(String, Vec<StyleRun>)> {
        self.walk_visual_rows(start_idx, rows, cell_cols, |row_text, runs| {
            (row_text, runs)
        })
    }

    /// Read one visual row at flat index `y` (over the wrapped buffer) as
    /// text. Returns `None` if `y >= wrapped_row_count(cell_cols)`. O(n) in
    /// the line's cell count (one logical line walk), independent of total
    /// buffer size.
    pub fn visual_row(&self, y: u32, cell_cols: u16) -> Option<String> {
        let (idx, sub) = self.locate_visual_row(y, cell_cols)?;
        let line = self.lines.get(idx)?;
        let mut sub_row = 0usize;
        let mut row_text = String::new();
        let mut row_width: u16 = 0;
        for cell in &line.cells {
            let cw = cell.width as u16;
            if cw == 0 {
                row_text.push(cell.ch);
                continue;
            }
            if row_width.saturating_add(cw) > cell_cols && !row_text.is_empty() {
                if sub_row == sub {
                    return Some(row_text);
                }
                sub_row += 1;
                row_text.clear();
                row_width = 0;
            }
            row_text.push(cell.ch);
            row_width = row_width.saturating_add(cw);
        }
        if sub_row == sub { Some(row_text) } else { None }
    }

    /// Same as [`Self::visual_row`] but returns style runs alongside the text.
    pub fn visual_row_with_styles(
        &self,
        y: u32,
        cell_cols: u16,
    ) -> Option<(String, Vec<StyleRun>)> {
        let (idx, sub) = self.locate_visual_row(y, cell_cols)?;
        let line = self.lines.get(idx)?;
        let mut sub_row = 0usize;
        let mut row_text = String::new();
        let mut row_cells: Vec<&LbCell> = Vec::new();
        let mut row_width: u16 = 0;
        for cell in &line.cells {
            let cw = cell.width as u16;
            if cw == 0 {
                row_text.push(cell.ch);
                row_cells.push(cell);
                continue;
            }
            if row_width.saturating_add(cw) > cell_cols && !row_cells.is_empty() {
                if sub_row == sub {
                    let runs = build_runs_for_row(&row_cells);
                    return Some((row_text, runs));
                }
                sub_row += 1;
                row_text.clear();
                row_cells.clear();
                row_width = 0;
            }
            row_text.push(cell.ch);
            row_cells.push(cell);
            row_width = row_width.saturating_add(cw);
        }
        if sub_row == sub {
            let runs = build_runs_for_row(&row_cells);
            Some((row_text, runs))
        } else {
            None
        }
    }

    /// Shared cell-walker for [`Self::wrap_visible`] /
    /// [`Self::wrap_visible_with_styles`]. The closure receives the row's
    /// joined text plus its rebased style runs.
    fn walk_visual_rows<T>(
        &self,
        start_idx: usize,
        rows: usize,
        cell_cols: u16,
        mut emit: impl FnMut(String, Vec<StyleRun>) -> T,
    ) -> Vec<T> {
        let mut out: Vec<T> = Vec::with_capacity(rows);
        if rows == 0 || cell_cols == 0 {
            return out;
        }

        for line in self.lines.iter().skip(start_idx) {
            if out.len() >= rows {
                break;
            }

            if line.cells.is_empty() {
                out.push(emit(String::new(), Vec::new()));
                continue;
            }

            let mut row_text = String::new();
            let mut row_cells: Vec<&LbCell> = Vec::new();
            let mut row_width: u16 = 0;

            for cell in &line.cells {
                let cw = cell.width as u16;
                if cw == 0 {
                    row_text.push(cell.ch);
                    row_cells.push(cell);
                    continue;
                }
                if row_width.saturating_add(cw) > cell_cols && !row_cells.is_empty() {
                    // Flush current row; the unused tail cell at the
                    // right edge is implicitly blank. Guard against the
                    // empty-row case (e.g. cell_cols=1 with a wide char)
                    // so we don't emit a leading blank row before placing
                    // the overflowing cell on its own row.
                    let runs = build_runs_for_row(&row_cells);
                    out.push(emit(std::mem::take(&mut row_text), runs));
                    row_cells.clear();
                    row_width = 0;
                    if out.len() >= rows {
                        return out;
                    }
                }
                row_text.push(cell.ch);
                row_cells.push(cell);
                row_width = row_width.saturating_add(cw);
            }

            if !row_cells.is_empty() {
                let runs = build_runs_for_row(&row_cells);
                out.push(emit(row_text, runs));
            }
        }

        out
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

/// RLE-encode `cells` into row-local [`StyleRun`]s. Adjacent cells with
/// identical `(fg, bg, flags)` collapse into one run. `start_col` /
/// `end_col` are 1-indexed inclusive cell columns — `start_col = 1`
/// covers the first cell — matching ghostty's
/// `dump_viewport_row_style_runs` convention so downstream consumers
/// (search highlight, background quads, byte_index_for_column_in_line)
/// see the same shape regardless of source.
fn build_runs_for_row(cells: &[&LbCell]) -> Vec<StyleRun> {
    let mut runs: Vec<StyleRun> = Vec::new();
    let mut col: u16 = 1;
    for cell in cells {
        let w = cell.width.max(1) as u16;
        let end = col.saturating_add(w).saturating_sub(1);
        match runs.last_mut() {
            Some(last) if last.fg == cell.fg && last.bg == cell.bg && last.flags == cell.flags => {
                last.end_col = end;
            }
            _ => runs.push(StyleRun {
                start_col: col,
                end_col: end,
                fg: cell.fg,
                bg: cell.bg,
                flags: cell.flags,
            }),
        }
        col = col.saturating_add(w);
    }
    runs
}

/// Sum the cell widths of every cell consumed by sub-rows `0..sub_row`
/// when `line` is wrapped at `cell_cols`. Matches the wrap walk in
/// `wrap_visible` so the result is the cumulative cell column at the
/// start of sub-row `sub_row`. Returns 0 when `sub_row == 0`.
///
/// Wide chars that straddle a boundary push to the next row; the
/// previous row's effective width is `cell_cols - 1`. Zero-width cells
/// (e.g. combining marks) advance neither column nor cell count.
fn cells_consumed_before_sub_row(line: &LogicalLine, cell_cols: u16, sub_row: usize) -> u16 {
    if sub_row == 0 || cell_cols == 0 {
        return 0;
    }
    let mut cumulative: u32 = 0;
    let mut row_width: u16 = 0;
    let mut row_cells_present = false;
    let mut current_sub_row: usize = 0;
    for cell in &line.cells {
        let cw = cell.width as u16;
        if cw == 0 {
            // Combining marks: tracked but contribute no cell width.
            continue;
        }
        if row_width.saturating_add(cw) > cell_cols && row_cells_present {
            current_sub_row += 1;
            if current_sub_row == sub_row {
                return cumulative.min(u16::MAX as u32) as u16;
            }
            row_width = 0;
            // `row_cells_present` is unconditionally reset to `true`
            // below — no need to clear it here.
        }
        cumulative = cumulative.saturating_add(cw as u32);
        row_width = row_width.saturating_add(cw);
        row_cells_present = true;
    }
    cumulative.min(u16::MAX as u32) as u16
}

/// Walk `line` at `cell_cols` and locate which `(sub_row, col_in_row)`
/// the cumulative-cell-column `sub_col` lands on. Mirrors
/// `wrap_visible`'s wrap rule so the result round-trips with
/// `cells_consumed_before_sub_row`.
///
/// `col_in_row` is 0-indexed and may equal `cell_cols` when `sub_col`
/// lands exactly on a wrap boundary (i.e. one past the rightmost cell
/// of the previous visual row).
fn locate_sub_col_in_line(line: &LogicalLine, cell_cols: u16, sub_col: u16) -> (u32, u16) {
    let mut consumed: u32 = 0;
    let mut row_width: u16 = 0;
    let mut row_cells_present = false;
    let mut sub_row: u32 = 0;
    let target = sub_col as u32;
    for cell in &line.cells {
        let cw = cell.width as u16;
        if cw == 0 {
            continue;
        }
        if row_width.saturating_add(cw) > cell_cols && row_cells_present {
            sub_row = sub_row.saturating_add(1);
            row_width = 0;
            // `row_cells_present` is unconditionally reset to `true`
            // when we accept this cell below; if we return early at
            // the next branch, the flag is no longer consulted.
        }
        if consumed.saturating_add(cw as u32) > target {
            // Target lands inside this cell — col_in_row is the
            // column at the start of this cell (within the row).
            return (sub_row, row_width);
        }
        consumed = consumed.saturating_add(cw as u32);
        row_width = row_width.saturating_add(cw);
        row_cells_present = true;
    }
    // Past the last printable cell — clamp to "after last cell on the
    // last sub_row".
    (sub_row, row_width)
}

/// Translate a `[byte_start, byte_end)` range within `line.text` into
/// the cumulative cell-column range within the logical line. Walks
/// `line.cells` once; widths are summed using each cell's `width`
/// field (CJK / emoji cells contribute 2 each). Bytes are aligned by
/// char boundary: `byte_pos >= byte_start` advances `start_col` to the
/// current cumulative width; the same rule applies for `byte_end`.
///
/// Both returned columns are 0-indexed cumulative widths within the
/// logical line — feed them into [`LineBuffer::coordinate_for_position`]
/// to map back to wrapped `(visual_y, visual_col)`.
fn byte_range_to_cell_cols(line: &LogicalLine, byte_start: usize, byte_end: usize) -> (u16, u16) {
    let mut byte_pos: usize = 0;
    let mut cell_col: u16 = 0;
    let mut start_col: Option<u16> = if byte_start == 0 { Some(0) } else { None };
    let mut end_col: Option<u16> = if byte_end == 0 { Some(0) } else { None };
    for cell in &line.cells {
        if start_col.is_none() && byte_pos >= byte_start {
            start_col = Some(cell_col);
        }
        if end_col.is_none() && byte_pos >= byte_end {
            end_col = Some(cell_col);
        }
        if start_col.is_some() && end_col.is_some() {
            break;
        }
        byte_pos = byte_pos.saturating_add(cell.ch.len_utf8());
        cell_col = cell_col.saturating_add(cell.width as u16);
    }
    (start_col.unwrap_or(cell_col), end_col.unwrap_or(cell_col))
}

#[cfg(test)]
mod tests;
