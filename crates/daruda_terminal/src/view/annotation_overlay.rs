//! Annotation overlay paint integration.
//!
//! Two pieces coexist here:
//!
//! 1. [`compute_rect`] — pure logic that picks where the floating box
//!    sits relative to the annotated row. Takes only cell-grid `u16`
//!    inputs and returns an [`OverlayRect`] describing the placement.
//!    GPUI-free and the only piece exercised by unit tests.
//!
//! 2. [`TerminalTextElement::build_annotation_quads`] — wraps
//!    `compute_rect` with the renderer-side concerns: pulls the
//!    in-viewport annotations from the session, resolves each row to
//!    a visible viewport offset, converts cells to pixels via the
//!    shaper-aware metrics, and emits the per-annotation quads (background
//!    fill + 1px border tint) **and** a `ShapedLine` for the first text
//!    line rendered inside the box using [`crate::ux::theme::ANNOTATION_TEXT`].
//!
//! Layout heuristic (per design §8):
//! - **Inline** when there is enough right-margin on the annotated row
//!   for the box to fit without overlapping text. The first line of
//!   the payload is rendered immediately past the row's last glyph
//!   with a 1-cell gap and a 1-cell trailing padding.
//! - **NextLineAbove** otherwise. The box paints on the row directly
//!   above the annotation, anchored to column 0, with width clamped to
//!   the viewport. This is a deliberate paint-time decision: at row 0
//!   with insufficient right-margin we still emit a `NextLineAbove`
//!   rect — the paint pass clips above the viewport bound and the user
//!   sees the bottom edge of the box pinned to the first visible row.

use gpui::{
    App, Bounds, Font, PaintQuad, Pixels, SharedString, TextRun, Window, fill, point, px, size,
};

use super::element::TerminalTextElement;
use super::text_metrics::shaped_pixel_range_for_cols;
use crate::ux::theme;

/// One floating annotation box positioned on the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverlayRect {
    /// Anchor mode picked by the layout heuristic — informational only,
    /// the paint code reads the geometry fields verbatim.
    pub placement: OverlayPlacement,
    /// 0-indexed column the left edge of the box sits at.
    pub x_cells: u16,
    /// 0-indexed visual row the top edge sits at.
    pub y_rows: u16,
    /// Width in cells (includes the 1-cell trailing padding).
    pub w_cells: u16,
    /// Height in rows (SP-1: always 1 — only the first text line is
    /// surfaced inline; future SPs may stack additional rows).
    pub h_rows: u16,
}

/// Layout mode picked by [`compute_rect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayPlacement {
    /// Box paints on the same row as the annotated text, past the
    /// row's last glyph.
    Inline,
    /// Box paints on the row directly above the annotated row.
    NextLineAbove,
}

/// Trailing padding inside the inline box so the text doesn't kiss
/// the border. Single cell on the right of the first line's content.
const INLINE_TRAILING_PAD_CELLS: u16 = 2;

/// Gap between the row's last glyph and the box's left edge.
const INLINE_LEADING_GAP_CELLS: u16 = 1;

/// Decide where the annotation overlay should paint.
///
/// `first_line_chars` is the codepoint count of the first text line in
/// the payload — the layout heuristic only considers the first line
/// (SP-1 inline overlays are single-row).
///
/// `line_text_end_col` is the last column (0-indexed) with text on the
/// annotated row. Used to decide whether there is enough right-margin
/// for the inline placement.
///
/// `viewport_cols` is the terminal grid width in cells.
///
/// `target_row` is the 0-indexed visual row of the annotated text.
///
/// Returns `None` when the payload is empty (`first_line_chars == 0`)
/// — an empty box is visually meaningless and the caller can skip the
/// quad emission entirely.
pub(crate) fn compute_rect(
    first_line_chars: u16,
    line_text_end_col: u16,
    viewport_cols: u16,
    target_row: u16,
) -> Option<OverlayRect> {
    // Defensive: an annotation with no text has no overlay to paint.
    // Returning `None` here lets the caller skip the per-row work
    // entirely rather than emit a zero-width quad that the paint stack
    // would still walk past.
    if first_line_chars == 0 {
        return None;
    }

    // Required cells = text width + leading gap + trailing pad. Saturate
    // so a pathologically long line on a narrow viewport degrades into
    // the NextLineAbove branch instead of panicking on overflow.
    let required = first_line_chars
        .saturating_add(INLINE_LEADING_GAP_CELLS)
        .saturating_add(INLINE_TRAILING_PAD_CELLS);
    let right_margin = viewport_cols.saturating_sub(line_text_end_col);

    if right_margin >= required {
        // Inline placement — leave one blank cell between the row's
        // last glyph and the box's left edge.
        let x = line_text_end_col.saturating_add(INLINE_LEADING_GAP_CELLS);
        let w = first_line_chars.saturating_add(INLINE_TRAILING_PAD_CELLS);
        return Some(OverlayRect {
            placement: OverlayPlacement::Inline,
            x_cells: x,
            y_rows: target_row,
            w_cells: w,
            h_rows: 1,
        });
    }

    // NextLineAbove fallback — anchor at column 0 of the row above
    // (or row 0 when the annotation is on the top viewport row; the
    // paint pass clips beyond the viewport's top edge).
    let y = target_row.saturating_sub(1);
    let w = first_line_chars
        .saturating_add(INLINE_TRAILING_PAD_CELLS)
        .min(viewport_cols);
    Some(OverlayRect {
        placement: OverlayPlacement::NextLineAbove,
        x_cells: 0,
        y_rows: y,
        w_cells: w,
        h_rows: 1,
    })
}

impl TerminalTextElement {
    /// Build the paint quads and shaped text for every annotation visible
    /// in the current viewport.
    ///
    /// Per annotation the return value includes:
    /// - a filled background quad (switches to `ANNOTATION_BG_HOVER` when
    ///   the annotation is hovered),
    /// - four 1-pixel border strips, and
    /// - a `ShapedLine` for the first text line, paired with its pixel
    ///   origin, using [`theme::ANNOTATION_TEXT`] colour.
    ///
    /// Hidden behind a single in-viewport range query so the renderer
    /// never walks the whole interval-tree on every frame.
    ///
    /// # Prepaint / paint split
    ///
    /// Shaping happens here (prepaint phase). The paint phase calls
    /// `shaped_line.paint(origin, line_height, window, cx)` on the
    /// returned text entries — callers must not call `window.text_style()`
    /// from the paint phase (see root CLAUDE.md pitfall §8).
    #[allow(clippy::too_many_arguments)] // Render helpers need layout + shaping inputs; splitting wraps callers more than it saves.
    pub(crate) fn build_annotation_quads(
        &self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        cell_width: f32,
        shaped_lines: &[gpui::ShapedLine],
        window: &mut Window,
        run_font: &Font,
        font_size: Pixels,
        cx: &mut App,
    ) -> (Vec<PaintQuad>, Vec<(gpui::ShapedLine, gpui::Point<Pixels>)>) {
        let view = self.view.read(cx);
        let viewport_top = view.session.viewport_row_offset();
        let viewport_rows = view.session.rows() as u32;
        let viewport_cols = view.session.cols();

        let hovered = view.state.hovered_annotation;
        let origin = bounds.origin;
        let mut quads: Vec<PaintQuad> = Vec::new();
        let mut text_lines: Vec<(gpui::ShapedLine, gpui::Point<Pixels>)> = Vec::new();

        // Iterate every visible row and ask the session whether an
        // annotation lives there. The per-row cost is one map lookup
        // + one interval-tree at_line() walk; the alternative would be
        // a session-side `line_coord_to_screen_row` reverse mapping,
        // which has to special-case the buffered / viewport split and
        // would need its own resize regression coverage.
        for visible_row in 0..viewport_rows {
            let screen_row = viewport_top.saturating_add(visible_row);
            let Some(line_coord) = view.session.screen_row_to_line_coord(screen_row) else {
                continue;
            };
            // SP-1: paint annotation only on the head row of a wrapped logical
            // line. Continuations would otherwise produce duplicate boxes (one
            // per wrap segment) because LineBufferPosition is the logical-line
            // id shared by every wrapped row.
            if view.session.is_wrap_continuation(screen_row) {
                continue;
            }
            // `annotation_at_point` returns the first match; SP-1 marks
            // are single-line so one annotation per row is the rule.
            // Pass column 0 — the inline-overlay layout treats the row
            // as a whole rather than per-cell anchoring.
            let Some((mark_id, payload)) = view.session.annotation_at_point(line_coord, 0) else {
                continue;
            };
            let visible_row = visible_row as u16;

            // Strip everything past the first newline — the inline
            // overlay is a single-row affordance in SP-1 (per design
            // §8). Multi-line text on disk is fine; we just truncate
            // the display.
            let first_line = payload.text.lines().next().unwrap_or("");
            // Codepoint count — the renderer pads the box with one
            // cell on each side, so columns are what we need rather
            // than byte length. Treats every char as advance-1 (CJK
            // double-width would underflow but the rest of the box
            // gets clipped by the trailing-pad cell).
            let first_line_chars = first_line.chars().count().min(u16::MAX as usize) as u16;

            // Line-text-end column = column index past the last glyph
            // on the row. Looked up from the cached viewport-line
            // text — viewport_lines is indexed by visible row.
            let line_text_end_col = view
                .state
                .viewport_lines
                .get(visible_row as usize)
                .map(|line| {
                    use unicode_width::UnicodeWidthStr as _;
                    line.width().min(viewport_cols as usize) as u16
                })
                .unwrap_or(0);

            let Some(rect) = compute_rect(
                first_line_chars,
                line_text_end_col,
                viewport_cols,
                visible_row,
            ) else {
                continue;
            };

            // Cell → pixel mapping. Inline rects piggyback on the
            // shaper so wide-character widths line up with the text;
            // the NextLineAbove case anchors at column 0 so plain
            // grid arithmetic is sufficient.
            let (x_left, x_right) = match rect.placement {
                OverlayPlacement::Inline => {
                    // Convert cell columns (1-indexed shaper API) by
                    // mapping the box's first/last cells.
                    let start_col_1 = rect.x_cells.saturating_add(1);
                    let end_col_1 = rect.x_cells.saturating_add(rect.w_cells).max(start_col_1);
                    let shaped = shaped_lines.get(visible_row as usize);
                    let line_text = view.state.viewport_lines.get(visible_row as usize);
                    shaped
                        .zip(line_text)
                        .and_then(|(s, t)| {
                            shaped_pixel_range_for_cols(s, t, start_col_1, end_col_1)
                        })
                        .unwrap_or_else(|| {
                            let x_start = px(cell_width * rect.x_cells as f32);
                            let x_end = px(cell_width * (rect.x_cells + rect.w_cells) as f32);
                            (x_start, x_end)
                        })
                }
                OverlayPlacement::NextLineAbove => {
                    let x_start = px(cell_width * rect.x_cells as f32);
                    let x_end = px(cell_width * (rect.x_cells + rect.w_cells) as f32);
                    (x_start, x_end)
                }
            };

            let y_top = origin.y + line_height * rect.y_rows as f32;
            let y_bottom = y_top + line_height * rect.h_rows as f32;

            let bg = if hovered == Some(mark_id) {
                theme::ANNOTATION_BG_HOVER
            } else {
                theme::ANNOTATION_BG_PRIMARY
            };
            let border = theme::ANNOTATION_BORDER;

            // 1. Filled rectangle.
            let x_l = origin.x + x_left;
            let x_r = origin.x + x_right;
            quads.push(fill(
                Bounds::from_corners(point(x_l, y_top), point(x_r, y_bottom)),
                bg,
            ));

            // 2. Border (four thin strips). Painting four edges as
            // separate quads keeps us from needing a custom outlined
            // primitive in the GPUI surface; the corners overlap by
            // 1px which is invisible at the alpha we use.
            let border_w = px(theme::ANNOTATION_BORDER_W);
            // Top
            quads.push(fill(
                Bounds::new(point(x_l, y_top), size(x_r - x_l, border_w)),
                border,
            ));
            // Bottom
            quads.push(fill(
                Bounds::new(point(x_l, y_bottom - border_w), size(x_r - x_l, border_w)),
                border,
            ));
            // Left
            quads.push(fill(
                Bounds::new(point(x_l, y_top), size(border_w, y_bottom - y_top)),
                border,
            ));
            // Right
            quads.push(fill(
                Bounds::new(
                    point(x_r - border_w, y_top),
                    size(border_w, y_bottom - y_top),
                ),
                border,
            ));

            // 3. First-line text inside the box.
            // The box width accounts for INLINE_LEADING_GAP_CELLS on the
            // left and INLINE_TRAILING_PAD_CELLS on the right (see
            // `compute_rect`). Text origin is inset by one cell from the
            // box's left edge so glyphs don't kiss the border.
            if !first_line.is_empty() {
                let text_x = x_l + px(cell_width);
                let text_origin = point(text_x, y_top);
                let text_run = TextRun {
                    len: first_line.len(),
                    font: run_font.clone(),
                    color: theme::ANNOTATION_TEXT,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let shaped = window.text_system().shape_line(
                    SharedString::from(first_line.to_string()),
                    font_size,
                    &[text_run],
                    None,
                );
                text_lines.push((shaped, text_origin));
            }
        }

        (quads, text_lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Case 1: payload fits inline next to the row's text.
    #[test]
    fn inline_when_right_margin_is_sufficient() {
        // Row uses cols 0..=9 (10 cells). Viewport is 80 cells wide.
        // First-line text is 12 chars. Required = 12 + 1 (gap) + 2 (pad) = 15.
        // Right margin = 80 - 10 = 70 ≥ 15.
        let rect = compute_rect(12, 10, 80, 5).expect("rect emitted");
        assert_eq!(rect.placement, OverlayPlacement::Inline);
        assert_eq!(rect.x_cells, 11, "inline starts one cell past end_col");
        assert_eq!(rect.y_rows, 5);
        assert_eq!(rect.w_cells, 14, "text + trailing pad");
        assert_eq!(rect.h_rows, 1);
    }

    /// Case 2: payload too wide for the right margin → row above.
    #[test]
    fn next_line_above_when_inline_does_not_fit() {
        // Row uses cols 0..=74 (75 cells). Viewport is 80 cells wide.
        // Required = 30 + 1 + 2 = 33. Right margin = 80 - 75 = 5 < 33.
        let rect = compute_rect(30, 75, 80, 4).expect("rect emitted");
        assert_eq!(rect.placement, OverlayPlacement::NextLineAbove);
        assert_eq!(rect.x_cells, 0);
        assert_eq!(
            rect.y_rows, 3,
            "next-line-above sits one row above the annotation"
        );
        assert_eq!(rect.w_cells, 32, "text + trailing pad, clamped to viewport");
    }

    /// Case 3: annotation on the topmost visible row with insufficient
    /// margin. The paint pass deliberately accepts the overlap with the
    /// area above the viewport — `compute_rect` returns row 0 so the
    /// quad clips against the viewport's top edge.
    #[test]
    fn target_row_zero_with_no_margin_stays_at_row_zero() {
        let rect = compute_rect(30, 75, 80, 0).expect("rect emitted");
        assert_eq!(rect.placement, OverlayPlacement::NextLineAbove);
        assert_eq!(
            rect.y_rows, 0,
            "saturating_sub keeps the row at 0; paint clips the top"
        );
    }

    /// Case 4: an empty payload should produce no overlay. Skipping the
    /// quad emission is cleaner than painting a width-2 stub box for
    /// padding alone — the user would see a yellow blob with no text.
    #[test]
    fn empty_payload_returns_none() {
        assert!(compute_rect(0, 10, 80, 5).is_none());
    }
}
