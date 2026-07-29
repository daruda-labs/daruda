use crate::session::{LineBufferPosition, TerminalSession};

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
/// `selection::Anchor { Point, Side }`. `row` is an **absolute** screen
/// row (scrollback + viewport), 1-indexed — widened to `u32` so block
/// selections can extend into `LineBuffer` scrollback past the viewport
/// top (iTerm2 `iTermSubSelection.absRange` parity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellAnchor {
    pub(super) col: u16,
    pub(super) row: u32,
    pub(super) side: Side,
}

impl CellAnchor {
    pub(super) fn new(col: u16, row: u32, side: Side) -> Self {
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
///
/// The returned `row` is **viewport-relative** (1-indexed within the
/// visible rows). Callers that need an absolute screen row — block
/// selections that survive scroll into `LineBuffer` — add the session's
/// `viewport_row_offset` at the call site (see `cell_anchor_at`).
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
    let row = raw_row.clamp(1, rows as i32) as u32;
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
///
/// `top` and `bottom` are **absolute** screen rows (1-indexed, covering
/// scrollback + viewport) so block selections persist into `LineBuffer`
/// scrollback (iTerm2 `iTermSubSelection.absRange + columnWindow`
/// parity). `left` and `right` are 1-indexed column coordinates within
/// the viewport's cell grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BlockRect {
    pub(super) top: u32,
    pub(super) bottom: u32,
    pub(super) left: u16,
    pub(super) right: u16,
}

/// Two-mode anchor that backs every [`ScreenPos`].
///
/// When a position lies in scrollback at selection start we stash a
/// stable [`LineBufferPosition`] plus the cumulative cell column within
/// that logical line — both invariants under viewport resize. When the
/// position lies in the live viewport we keep the current-frame
/// `(screen_row, byte)` pair, which is re-clamped against the new
/// viewport on resize. Mirrors iTerm2's `LineBufferPosition`
/// `{absolutePosition, yOffset, extendsToEndOfLine}` design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PosAnchor {
    Scrollback {
        pos: LineBufferPosition,
        /// Cumulative cell column within the logical line at the
        /// start of the visual row (`sub_row * cell_cols` modulo
        /// wide-char wrap rules), plus the within-row column. The
        /// pair is invariant under width change because the cells of
        /// the logical line are width-stable.
        cell_col: u16,
    },
    Viewport {
        screen_row: u32,
        byte: usize,
    },
}

impl Default for PosAnchor {
    fn default() -> Self {
        Self::Viewport {
            screen_row: 0,
            byte: 0,
        }
    }
}

/// Selection endpoint. Either a width-invariant `LineBufferPosition`
/// anchor (for scrollback-resident endpoints captured at selection
/// start) or a current-frame `(screen_row, byte)` pair (for
/// viewport-resident endpoints). Call [`Self::resolve`] to project to
/// the current frame after any state change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ScreenPos {
    pub(super) anchor: PosAnchor,
}

impl ScreenPos {
    /// Construct an anchor at the cell `(abs_screen_row, cell_col)`.
    /// `cell_col` is **1-indexed** (1..=cols). Passing 0 panics in
    /// debug builds; in release it silently snaps to column 1 (the
    /// `saturating_sub(1)` and `byte_index_for_column_in_line` paths
    /// both treat 0 the same as 1). When the row is in scrollback
    /// (covered by `LineBuffer`) the returned anchor stores a
    /// width-invariant `LineBufferPosition`; otherwise it stores the
    /// current-frame `(screen_row, byte)` pair.
    pub(super) fn anchor_at(session: &TerminalSession, abs_screen_row: u32, cell_col: u16) -> Self {
        debug_assert!(cell_col >= 1, "cell_col is 1-indexed; got 0");
        let cell_cols = session.cols();
        let lb = session.line_buffer();
        let lb_rows = lb.wrapped_row_count(cell_cols);
        if abs_screen_row < lb_rows
            && cell_cols > 0
            && let Some((pos, _sub_row, sub_col_origin)) =
                lb.position_for_visual_row(abs_screen_row, cell_cols)
        {
            let within_row = cell_col.saturating_sub(1);
            let cumulative = sub_col_origin.saturating_add(within_row);
            return Self {
                anchor: PosAnchor::Scrollback {
                    pos,
                    cell_col: cumulative,
                },
            };
        }
        // Live viewport row — fall back to byte-offset storage.
        // SILENT-OK: anchor_at's only production caller
        // (`mouse_position_to_screen_pos` in view/mouse.rs) has already
        // located `abs_screen_row` inside the unified frame before
        // delegating, so `dump_screen_row` only fails for rows the
        // caller manufactured out of bounds — in which case byte=0 is a
        // defensive fallback that the next `resolve()` re-clamps.
        let line = session.dump_screen_row(abs_screen_row).unwrap_or_default();
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let byte = text_metrics::byte_index_for_column_in_line(line, cell_col).min(line.len());
        Self {
            anchor: PosAnchor::Viewport {
                screen_row: abs_screen_row,
                byte,
            },
        }
    }

    /// Construct a viewport-resident anchor directly from a current-frame
    /// `(screen_row, byte)` pair. Used by mouse / drag code paths that
    /// already computed the byte from pixel metrics and don't need to
    /// re-derive it through `dump_screen_row`.
    pub(super) fn viewport(screen_row: u32, byte: usize) -> Self {
        Self {
            anchor: PosAnchor::Viewport { screen_row, byte },
        }
    }

    /// Resolve to a current-frame `(screen_row, byte)` pair. Returns
    /// `None` only when the underlying scrollback line has been evicted
    /// (matches `LineBuffer::deref` semantics).
    ///
    /// For `Viewport` anchors the stored pair is returned as-is — the
    /// caller is expected to clamp against the current viewport
    /// downstream (e.g. `viewport_slice_screen` already tolerates byte
    /// offsets past the row's text).
    ///
    /// For `Scrollback` anchors we project through the current cell
    /// width: `coordinate_for_position` walks the logical line at the
    /// session's current `cols()` to recover the visual row and the
    /// within-row cell column; the within-row cell column is then
    /// translated to a byte offset against the freshly-dumped row text.
    pub(super) fn resolve(&self, session: &TerminalSession) -> Option<(u32, usize)> {
        match &self.anchor {
            PosAnchor::Viewport { screen_row, byte } => Some((*screen_row, *byte)),
            PosAnchor::Scrollback { pos, cell_col } => {
                let cell_cols = session.cols();
                let (visual_y, col_in_row) = session
                    .line_buffer()
                    .coordinate_for_position(pos, cell_cols, *cell_col)?;
                let line = session.dump_screen_row(visual_y).ok()?;
                let line = line.strip_suffix('\n').unwrap_or(&line);
                // `col_in_row` is 0-indexed; the byte-index helper wants
                // a 1-indexed cell column.
                let one_indexed = col_in_row.saturating_add(1);
                let byte =
                    text_metrics::byte_index_for_column_in_line(line, one_indexed).min(line.len());
                Some((visual_y, byte))
            }
        }
    }

    /// Current-frame screen row, used by callers that only care about
    /// the row (e.g. autoscroll, hit-tests). Returns `None` when the
    /// underlying scrollback line has been evicted.
    #[cfg(test)]
    pub(super) fn screen_row(&self, session: &TerminalSession) -> Option<u32> {
        self.resolve(session).map(|(row, _)| row)
    }

    /// Current-frame byte offset within the row, used by callers that
    /// only care about the byte coordinate. Returns `None` when the
    /// underlying scrollback line has been evicted.
    #[cfg(test)]
    pub(super) fn byte(&self, session: &TerminalSession) -> Option<usize> {
        self.resolve(session).map(|(_, byte)| byte)
    }
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
    /// `start.screen_row ≤ end.screen_row` (or equal row, `start.byte ≤
    /// end.byte`). Resolution requires a session because either endpoint
    /// may be a width-invariant scrollback anchor — they only project to
    /// screen coordinates against the current `cell_cols`.
    ///
    /// Returns `None` when either endpoint has been evicted from the
    /// `LineBuffer` (a previously-stored `PosAnchor::Scrollback` whose
    /// `LineBufferPosition` no longer resolves). The selection cannot
    /// be re-projected onto the current frame, so callers treat
    /// `None` as "no live selection" rather than collapsing the
    /// evicted endpoint to `(0, 0)` — which would expand the selection
    /// backwards to the start of the buffer.
    pub(super) fn normalized(self, session: &TerminalSession) -> Option<(ScreenPos, ScreenPos)> {
        let a = self.anchor.resolve(session)?;
        let b = self.active.resolve(session)?;
        if a <= b {
            Some((self.anchor, self.active))
        } else {
            Some((self.active, self.anchor))
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

/// Slices each row in the block's range to the column window and joins
/// rows with `\n` — the copy payload for Block-mode selection.
///
/// Rows are fetched via [`TerminalSession::dump_screen_row`] so the
/// block walks scrollback and viewport uniformly (Task 7: iTerm2
/// `iTermSubSelection.absRange + columnWindow` parity). A row that
/// fails to dump (e.g. evicted from `LineBuffer`) contributes an empty
/// line so the rectangle's geometry is preserved when pasted back.
///
/// `rect.top`, `rect.bottom`, `rect.left`, `rect.right` are 1-indexed.
pub(super) fn block_copy_text(rect: &BlockRect, session: &TerminalSession) -> String {
    use text_metrics::byte_index_for_column_in_line;
    let mut buf = String::new();
    for row in rect.top..=rect.bottom {
        if row > rect.top {
            buf.push('\n');
        }
        let abs_row = row.saturating_sub(1);
        let line = session.dump_screen_row(abs_row).unwrap_or_default();
        let line = line.strip_suffix('\n').unwrap_or(&line);
        if line.is_empty() {
            continue;
        }
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
///
/// `rect.top` / `rect.bottom` are **absolute** screen rows; rows that
/// lie above the visible viewport (i.e. still in scrollback above
/// `vp_top`) or at/below the viewport bottom are dropped — only the
/// portion intersecting the visible region produces paint quads.
/// `vp_top` is the session's `viewport_row_offset` and `vp_rows` its
/// `rows()`.
pub(super) fn block_selection_quads(
    rect: BlockRect,
    origin_x: f32,
    origin_y: f32,
    cell_width: f32,
    line_height: f32,
    vp_top: u32,
    vp_rows: u32,
) -> Vec<BlockQuadRect> {
    let left_col = rect.left.saturating_sub(1) as f32;
    let right_col_exclusive = rect.right as f32;
    let x1 = origin_x + cell_width * left_col;
    let x2 = origin_x + cell_width * right_col_exclusive;
    let mut quads = Vec::new();
    for row in rect.top..=rect.bottom {
        // `rect.top` / `rect.bottom` are 1-indexed absolute screen rows;
        // convert to 0-indexed for `screen_row_to_visible`.
        let abs_row = row.saturating_sub(1);
        let Some(visible_row) = super::overlay::screen_row_to_visible(abs_row, vp_top, vp_rows)
        else {
            continue;
        };
        let y1 = origin_y + line_height * visible_row as f32;
        let y2 = y1 + line_height;
        quads.push(BlockQuadRect { x1, y1, x2, y2 });
    }
    quads
}
