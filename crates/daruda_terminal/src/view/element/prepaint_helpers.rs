//! Per-section prepaint helpers for `TerminalTextElement`.
//!
//! Each method emits one category of paint quads (background, search
//! highlights, prompt-mark gutter, scrollbar match ticks, URL hover
//! underline) so the main `prepaint` body can read top-down without
//! interleaving five different concerns.
//!
//! All helpers go through the row's shaped layout when wide / CJK /
//! emoji glyphs are present, falling back to grid arithmetic only on
//! the first frame when `line_layouts` has not been populated yet.
//! See the root CLAUDE.md §7 (column → pixel must go through
//! shaping) for the rationale.

use ghostty_vt::Rgb;
use gpui::{App, Bounds, PaintQuad, Pixels, fill, point, px, rgba, size};

use super::super::overlay::screen_row_to_visible;
use super::super::text_metrics::shaped_pixel_range_for_cols;
use super::TerminalTextElement;
use crate::ux::theme;

impl TerminalTextElement {
    pub(super) fn build_background_quads(
        &self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        cell_width: f32,
        default_bg: Rgb,
        cx: &mut App,
    ) -> Vec<PaintQuad> {
        let origin = bounds.origin;
        let mut quads: Vec<PaintQuad> = Vec::new();
        let view = self.view.read(cx);

        // Single-owner background model:
        //   1. Base fill — full bounds at default_bg × background_alpha.
        //      Covers default-bg cells, the sub-row gap at the bottom, and
        //      any area not mapped to a viewport row.
        //   2. Per-cell overrides — non-default cells only, painted at 0xFF
        //      so they are always opaque on top of the base fill.
        // The TerminalView root div carries no `.bg()` so this function is
        // the single source of truth for all terminal background rendering.
        let alpha_u8 = (view.state.background_alpha * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        let base_color = rgba(
            (u32::from(default_bg.r) << 24)
                | (u32::from(default_bg.g) << 16)
                | (u32::from(default_bg.b) << 8)
                | u32::from(alpha_u8),
        );
        quads.push(fill(
            Bounds::from_corners(
                point(origin.x, bounds.top()),
                point(bounds.right(), bounds.bottom()),
            ),
            base_color,
        ));

        for (row, runs) in view.state.viewport_style_runs.iter().enumerate() {
            let y = origin.y + line_height * row as f32;
            let shaped = view.line_layouts.get(row).and_then(|l| l.as_ref());
            let line_text = view.state.viewport_lines.get(row).map(|s| s.as_str());
            for span in crate::view::bg_merge::merge_bg_runs(runs, default_bg) {
                let (x_start, x_end) =
                    bg_pixel_range(shaped, line_text, span.start_col, span.end_col, cell_width);
                let x = origin.x + x_start;
                let w = x_end - x_start;
                let color = rgba(
                    (u32::from(span.bg.r) << 24)
                        | (u32::from(span.bg.g) << 16)
                        | (u32::from(span.bg.b) << 8)
                        | 0xFF,
                );
                quads.push(fill(Bounds::new(point(x, y), size(w, line_height)), color));
            }
        }

        quads
    }

    pub(super) fn build_search_quads(
        &self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        shaped_lines: &[gpui::ShapedLine],
        cx: &mut App,
    ) -> Vec<PaintQuad> {
        let view = self.view.read(cx);
        let state = &view.state.search;
        if state.matches.is_empty() {
            return Vec::new();
        }
        let viewport_top = view.session.viewport_row_offset();
        let viewport_rows = view.session.rows() as u32;
        let origin = bounds.origin;
        let mut quads = Vec::with_capacity(state.matches.len());
        for (idx, m) in state.matches.iter().enumerate() {
            let Some(visible) = screen_row_to_visible(m.row, viewport_top, viewport_rows) else {
                continue;
            };
            let visible_row_idx = visible as usize;
            let (Some(line_text), Some(shaped)) = (
                view.state.viewport_lines.get(visible_row_idx),
                shaped_lines.get(visible_row_idx),
            ) else {
                continue;
            };
            let Some((x_start, x_end)) =
                shaped_pixel_range_for_cols(shaped, line_text, m.start_col, m.end_col)
            else {
                continue;
            };
            let is_focused = state.focused == Some(idx);
            let color = if is_focused {
                theme::PROMPT_MARK_FOCUSED_OTHER
            } else {
                theme::PROMPT_MARK_OTHER
            };
            let y = origin.y + line_height * visible_row_idx as f32;
            quads.push(fill(
                Bounds::from_corners(
                    point(origin.x + x_start, y),
                    point(origin.x + x_end, y + line_height),
                ),
                color,
            ));
        }
        quads
    }

    pub(super) fn build_prompt_marks(
        &self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        cx: &mut App,
    ) -> Vec<PaintQuad> {
        let view = self.view.read(cx);
        let rows = view.session.rows() as u32;
        let viewport_top = view.session.viewport_row_offset();
        let mut out: Vec<PaintQuad> = Vec::new();
        let origin = bounds.origin;
        let strip_w = px(theme::PROMPT_MARK_STRIP_W);
        let focused_prompt = view.state.focused_prompt;
        let focused_command = view.state.focused_command;

        for mark in view.session.prompt_marks().iter().rev().take(64) {
            // Translate the mark's absolute Y to a current screen row;
            // marks whose row was evicted from `LineBuffer` skip the
            // paint (their gutter band would point at the wrong row).
            let Some(screen_row) = view.session.abs_to_screen_row(mark.abs_y) else {
                continue;
            };
            let Some(visible) = screen_row_to_visible(screen_row, viewport_top, rows) else {
                continue;
            };
            let visible_row = visible as f32;
            // Compare by mark identity (`seq`), not by screen row.
            // A `\x1b[3J` mirror in `clear_line_buffer_and_shift_marks`
            // can drop wiped marks, and re-flow / scroll moves the
            // surviving marks' screen rows; row-based comparison would
            // drift off the focused mark across either. `seq` is the
            // position-independent identity that never reuses.
            let is_focused = Some(mark.seq) == focused_prompt || Some(mark.seq) == focused_command;
            let color = match (mark.kind, mark.exit_code) {
                (crate::session::PromptMarkKind::CommandFinished, Some(code)) if code != 0 => {
                    theme::PROMPT_MARK_FOCUSED_PROMPT
                }
                (crate::session::PromptMarkKind::PromptStart, _) => theme::PROMPT_MARK_PROMPT_START,
                (crate::session::PromptMarkKind::CommandFinished, _) => {
                    theme::PROMPT_MARK_COMMAND_FINISHED
                }
                (crate::session::PromptMarkKind::CommandExecuted, _) => {
                    theme::PROMPT_MARK_COMMAND_EXECUTED
                }
                _ => continue,
            };
            let (color, width) = if is_focused {
                (
                    gpui::Hsla {
                        l: (color.l + 0.15).min(0.95),
                        a: 1.0,
                        ..color
                    },
                    strip_w * 2.0,
                )
            } else {
                (color, strip_w)
            };
            let y = origin.y + line_height * visible_row;
            out.push(fill(
                Bounds::new(point(origin.x, y), size(width, line_height)),
                color,
            ));
        }
        out
    }

    /// Stack of horizontal ticks aligned with the scrollbar track so
    /// the user can see at a glance where else in the scrollback
    /// search hits live. Mirrors iTerm2 / VS Code minimap markers.
    /// Returns an empty vec when there's no scrollback to scroll
    /// over (no scrollbar would be drawn either).
    pub(super) fn build_search_scrollbar_ticks(
        &self,
        bounds: Bounds<Pixels>,
        cx: &mut App,
    ) -> Vec<PaintQuad> {
        let view = self.view.read(cx);
        if view.state.search.matches.is_empty() {
            return Vec::new();
        }
        let total_rows = view.session.total_rows();
        let viewport_rows = view.session.rows() as u32;
        if total_rows <= viewport_rows {
            return Vec::new();
        }
        let track_height = f32::from(bounds.size.height);
        let scrollbar_width = 6.0_f32;
        let margin = 2.0_f32;
        let track_x = bounds.right() - px(scrollbar_width + margin);
        let mut quads = Vec::with_capacity(view.state.search.matches.len());
        let focused_row = view
            .state
            .search
            .focused
            .and_then(|i| view.state.search.matches.get(i).map(|m| m.row));
        for m in view.state.search.matches.iter() {
            let frac = m.row as f32 / total_rows.saturating_sub(1).max(1) as f32;
            let y = bounds.top() + px(frac * track_height);
            let is_focused = Some(m.row) == focused_row;
            let color = if is_focused {
                theme::PROMPT_MARK_TICK_FOCUSED
            } else {
                theme::PROMPT_MARK_TICK_DEFAULT
            };
            quads.push(fill(
                Bounds::new(
                    point(track_x, y),
                    size(px(scrollbar_width), px(theme::PROMPT_MARK_TICK_H)),
                ),
                color,
            ));
        }
        quads
    }

    pub(super) fn build_hover_underline(
        &self,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
        shaped_lines: &[gpui::ShapedLine],
        run_color: gpui::Hsla,
        cx: &mut App,
    ) -> Option<PaintQuad> {
        let view = self.view.read(cx);
        let hit = view.state.hovered_url.as_ref()?;
        let row = hit.row as usize;
        let line_text = view.state.viewport_lines.get(row)?;
        let shaped = shaped_lines.get(row)?;
        let (x_start, x_end) =
            shaped_pixel_range_for_cols(shaped, line_text, hit.start_col, hit.end_col)?;
        let origin = bounds.origin;
        let y_top = origin.y + line_height * hit.row as f32;
        let baseline = y_top + line_height - px(theme::CURSOR_UNDERLINE_OFFSET_Y);
        Some(fill(
            Bounds::from_corners(
                point(origin.x + x_start, baseline),
                point(
                    origin.x + x_end,
                    baseline + px(theme::CURSOR_UNDERLINE_END_OFFSET_Y),
                ),
            ),
            run_color,
        ))
    }
}

/// Resolve a background span to pixel x coordinates via the shaper when
/// available, falling back to grid arithmetic on the first frame before
/// `line_layouts` is populated.
///
/// Using grid arithmetic for multi-cell spans containing wide/CJK glyphs
/// drifts from the actual glyph position: GPUI drops `force_width` when
/// any glyph on the line is wide, so `cell_width * col` no longer tracks
/// the shaper's advance.  The shaper path is therefore used for all spans,
/// not just single-cell ones.  `shaped_pixel_range_for_cols` returns `None`
/// only when the shaped text is empty or the byte range collapses (e.g. the
/// right half of a wide char referenced as a lone col), at which point the
/// grid fallback is still correct.
fn bg_pixel_range(
    shaped: Option<&gpui::ShapedLine>,
    line_text: Option<&str>,
    start_col: u16,
    end_col: u16,
    cell_width: f32,
) -> (Pixels, Pixels) {
    let grid_x = |col_boundary: u16| px(cell_width * col_boundary.saturating_sub(1) as f32);
    if let (Some(shaped), Some(line_text)) = (shaped, line_text)
        && let Some(range) = shaped_pixel_range_for_cols(shaped, line_text, start_col, end_col)
    {
        return range;
    }
    (grid_x(start_col), grid_x(end_col.saturating_add(1)))
}
