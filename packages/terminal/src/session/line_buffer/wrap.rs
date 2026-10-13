//! Wrapping logical lines to a cell width: visual-row counts, row/position
//! mapping, and styled row extraction.

use super::*;

impl LineBuffer {
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
