//! `LineBuffer::find_matches` and projection of logical-line matches onto
//! wrapped visual rows.

use super::*;

impl LineBuffer {
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
