use super::text_metrics;

/// Selection geometry: linear stream (reading order) vs. rectangular
/// block (iTerm2 "Box selection" / Alacritty `SelectionType::Block`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SelectionMode {
    #[default]
    Linear,
    Block,
}

/// Which side of a cell the mouse landed on. Alacritty's
/// `selection::Side`: used as a tie-breaker so clicking on the right
/// half of a cell snaps the selection boundary to *after* that cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// 1-indexed cell coordinate with Side. Mirrors Alacritty's
/// `selection::Anchor { Point, Side }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellAnchor {
    pub(super) col: u16,
    pub(super) row: u16,
    pub(super) side: Side,
}

impl CellAnchor {
    pub(super) fn new(col: u16, row: u16, side: Side) -> Self {
        Self { col, row, side }
    }
}

/// Pure helper: given the x offset inside a single cell (0 ≤ x < cell_width),
/// decide which half of the cell the cursor is on.
pub(super) fn cell_side(x_within_cell: f32, cell_width: f32) -> Side {
    if x_within_cell > cell_width * 0.5 {
        Side::Right
    } else {
        Side::Left
    }
}

/// Pure helper: map a viewport-local pixel position to a `CellAnchor`.
/// Clamps the column/row to the grid's valid range.
pub(super) fn pixel_to_cell_anchor(
    x: f32,
    y: f32,
    cell_width: f32,
    cell_height: f32,
    cols: u16,
    rows: u16,
) -> CellAnchor {
    if cols == 0 || rows == 0 || cell_width <= 0.0 || cell_height <= 0.0 {
        return CellAnchor::new(1, 1, Side::Left);
    }
    let x = x.max(0.0);
    let y = y.max(0.0);
    let raw_col = (x / cell_width).floor() as i32 + 1;
    let raw_row = (y / cell_height).floor() as i32 + 1;
    let col = raw_col.clamp(1, cols as i32) as u16;
    let row = raw_row.clamp(1, rows as i32) as u16;
    let x_within = x - cell_width * (col - 1) as f32;
    let clamped_x_within = x_within.max(0.0).min(cell_width);
    let side = cell_side(clamped_x_within, cell_width);
    CellAnchor::new(col, row, side)
}

/// Normalize a pair of `CellAnchor`s into an inclusive `BlockRect`,
/// applying Alacritty's `range_simple` Side-based boundary rules.
pub(super) fn block_rect_from_anchors(anchor: CellAnchor, active: CellAnchor) -> Option<BlockRect> {
    let (a, b) = if (anchor.row, anchor.col) <= (active.row, active.col) {
        (anchor, active)
    } else {
        (active, anchor)
    };

    let top = a.row.min(b.row);
    let bottom = a.row.max(b.row);
    let (left_anchor, right_anchor) = if a.col <= b.col { (a, b) } else { (b, a) };
    let mut left = left_anchor.col;
    let mut right = right_anchor.col;

    if left < right {
        if right_anchor.side == Side::Left {
            right = right.saturating_sub(1);
        }
        if left_anchor.side == Side::Right {
            left = left.saturating_add(1);
        }
        if left > right {
            return None;
        }
    }

    Some(BlockRect {
        top,
        bottom,
        left,
        right,
    })
}

/// Normalized block rectangle — inclusive on all four sides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BlockRect {
    pub(super) top: u16,
    pub(super) bottom: u16,
    pub(super) left: u16,
    pub(super) right: u16,
}

/// Absolute screen-space position: row is the ghostty screen row
/// (viewport_row_offset + viewport_row), byte is the offset within
/// that row's text.  Storing positions in screen space rather than as
/// a flat viewport-relative byte offset means the selection survives
/// both partial viewport repaints and user scrolling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ScreenPos {
    /// Absolute screen row: `session.viewport_row_offset() + viewport_row`.
    pub(super) screen_row: u32,
    /// Byte offset within that row's text.
    pub(super) byte: usize,
}

/// Selection state.  Stores endpoints as `ScreenPos` (absolute screen
/// coordinates) so the selection persists through viewport repaints and
/// user scrolling.  Block mode additionally tracks cell-grid anchors for
/// the rectangular highlight.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ByteSelection {
    pub(super) anchor: ScreenPos,
    pub(super) active: ScreenPos,
    pub(super) mode: SelectionMode,
    pub(super) block_anchor: Option<CellAnchor>,
    pub(super) block_active: Option<CellAnchor>,
}

impl ByteSelection {
    pub(super) fn linear(anchor: ScreenPos, active: ScreenPos) -> Self {
        Self {
            anchor,
            active,
            mode: SelectionMode::Linear,
            block_anchor: None,
            block_active: None,
        }
    }

    pub(super) fn block(anchor: ScreenPos, cell_anchor: CellAnchor) -> Self {
        Self {
            anchor,
            active: anchor,
            mode: SelectionMode::Block,
            block_anchor: Some(cell_anchor),
            block_active: Some(cell_anchor),
        }
    }

    /// Returns the endpoints in reading order: `(start, end)` where
    /// `start.screen_row ≤ end.screen_row` (or equal row, `start.byte ≤ end.byte`).
    pub(super) fn normalized(self) -> (ScreenPos, ScreenPos) {
        if (self.anchor.screen_row, self.anchor.byte) <= (self.active.screen_row, self.active.byte)
        {
            (self.anchor, self.active)
        } else {
            (self.active, self.anchor)
        }
    }

    /// True when the selection covers no content (anchor == active).
    pub(super) fn is_empty(self) -> bool {
        self.anchor == self.active
    }

    pub(super) fn is_block(self) -> bool {
        matches!(self.mode, SelectionMode::Block)
    }

    pub(super) fn block_rect(self) -> Option<BlockRect> {
        let a = self.block_anchor?;
        let b = self.block_active?;
        block_rect_from_anchors(a, b)
    }
}

/// Pure helper — Alt (Option on macOS) switches to Block selection.
pub(super) fn selection_mode_from_modifiers(alt: bool) -> SelectionMode {
    if alt {
        SelectionMode::Block
    } else {
        SelectionMode::Linear
    }
}

/// One pixel rectangle produced by a Block selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlockQuadRect {
    pub(super) x1: f32,
    pub(super) y1: f32,
    pub(super) x2: f32,
    pub(super) y2: f32,
}

/// Slices each viewport line to the block's column range and joins
/// rows with `\n` — the copy payload for Block-mode selection.
pub(super) fn block_copy_text(lines: &[String], rect: BlockRect) -> String {
    use text_metrics::byte_index_for_column_in_line;
    let mut buf = String::new();
    for row in rect.top..=rect.bottom {
        if row > rect.top {
            buf.push('\n');
        }
        let row_idx = row.saturating_sub(1) as usize;
        let Some(line) = lines.get(row_idx) else {
            continue;
        };
        let byte_start = byte_index_for_column_in_line(line, rect.left).min(line.len());
        let byte_end =
            byte_index_for_column_in_line(line, rect.right.saturating_add(1)).min(line.len());
        if byte_end > byte_start {
            buf.push_str(&line[byte_start..byte_end]);
        }
    }
    buf
}

/// Convert a `BlockRect` into per-row pixel rectangles.
pub(super) fn block_selection_quads(
    rect: BlockRect,
    origin_x: f32,
    origin_y: f32,
    cell_width: f32,
    line_height: f32,
) -> Vec<BlockQuadRect> {
    let left_col = rect.left.saturating_sub(1) as f32;
    let right_col_exclusive = rect.right as f32;
    let x1 = origin_x + cell_width * left_col;
    let x2 = origin_x + cell_width * right_col_exclusive;
    let mut quads = Vec::with_capacity((rect.bottom - rect.top + 1) as usize);
    for row in rect.top..=rect.bottom {
        let row_idx = row.saturating_sub(1) as f32;
        let y1 = origin_y + line_height * row_idx;
        let y2 = y1 + line_height;
        quads.push(BlockQuadRect { x1, y1, x2, y2 });
    }
    quads
}
