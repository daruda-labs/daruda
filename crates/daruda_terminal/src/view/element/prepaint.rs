use super::super::box_drawing::{
    block_glyph_for_char, box_drawing_mask, box_drawing_quads_for_char, line_has_box_drawing,
    powerline_for_char,
};
use super::super::overlay::{
    flash_overlay_if_active, grid_row_to_screen_row, screen_row_to_visible,
};
use super::super::selection::block_selection_quads;
use super::super::state::MouseDragState;
use super::super::style::{
    CELL_STYLE_FLAG_BOLD, CELL_STYLE_FLAG_FAINT, CELL_STYLE_FLAG_ITALIC,
    CELL_STYLE_FLAG_STRIKETHROUGH, CELL_STYLE_FLAG_UNDERLINE, TextRunKey, color_for_key,
    cursor_color_for_background, hsla_from_rgb, text_run_for_key,
};
use super::super::text_metrics::{
    byte_index_for_column_in_line, cell_left_x_for_col, cursor_width_for_col, cursor_x_for_col,
    grid_right_x, shaped_pixel_range_for_cols,
};
use super::{PreeditPostShift, TerminalPrepaintState, TerminalTextElement};
use crate::ux::theme;
use gpui::{
    App, Bounds, PaintQuad, Pixels, SharedString, TextRun, UnderlineStyle, Window, fill, point, px,
    rgba, size,
};

impl TerminalTextElement {
    pub(super) fn do_prepaint(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> TerminalPrepaintState {
        let (font, font_size_pt, h_spacing, v_spacing, default_fg, layout) = {
            let v = self.view.read(cx);
            let layout = v.cell_layout(window);
            (
                v.state.font.clone(),
                v.state.font_size,
                v.state.horizontal_spacing,
                v.state.vertical_spacing,
                v.session.default_foreground(),
                layout,
            )
        };
        let font_size = px(font_size_pt);

        let mut style = window.text_style();
        style.font_family = font.family.clone();
        style.font_features = crate::default_terminal_font_features();
        style.font_fallbacks = font.fallbacks.clone();
        style.font_size = font_size.into();
        style.color = hsla_from_rgb(default_fg);

        let run_font = style.font();
        let run_color = style.color;

        let layout = layout.unwrap_or(super::super::layout::TerminalLayout {
            cell_width: 8.0 * h_spacing,
            line_height: f32::from(font_size) * 1.25 * v_spacing,
        });
        let cell_width_f = layout.cell_width;
        let line_height = px(layout.line_height);
        let cell_width = Some(px(cell_width_f));

        self.view.update(cx, |view, _cx| {
            if view.state.viewport_lines.is_empty() {
                view.line_layouts.clear();
                view.line_layout_key = None;
                return;
            }

            let cache_key = (
                font_size,
                line_height,
                px(cell_width_f),
                super::super::font_hash(&font),
            );
            if view.line_layout_key != Some(cache_key)
                || view.line_layouts.len() != view.state.viewport_lines.len()
            {
                view.line_layout_key = Some(cache_key);
                view.line_layouts = vec![None; view.state.viewport_lines.len()];
            }

            for (idx, line) in view.state.viewport_lines.iter().enumerate() {
                let Some(slot) = view.line_layouts.get_mut(idx) else {
                    continue;
                };

                if let Some(existing) = slot.as_ref()
                    && existing.text.as_str() == line.as_str()
                {
                    continue;
                }

                let text = SharedString::from(line.clone());
                let mut runs: Vec<TextRun> = Vec::new();

                if let Some(style_runs) = view.state.viewport_style_runs.get(idx)
                    && !style_runs.is_empty()
                {
                    let mut byte_pos = 0usize;
                    for style in style_runs.iter() {
                        let key = TextRunKey {
                            fg: style.fg,
                            flags: style.flags
                                & (CELL_STYLE_FLAG_BOLD
                                    | CELL_STYLE_FLAG_ITALIC
                                    | CELL_STYLE_FLAG_UNDERLINE
                                    | CELL_STYLE_FLAG_FAINT
                                    | CELL_STYLE_FLAG_STRIKETHROUGH),
                        };

                        let start = byte_index_for_column_in_line(text.as_str(), style.start_col)
                            .min(text.len());
                        let end = byte_index_for_column_in_line(
                            text.as_str(),
                            style.end_col.saturating_add(1),
                        )
                        .min(text.len());

                        if start > byte_pos {
                            runs.push(TextRun {
                                len: start.saturating_sub(byte_pos),
                                font: run_font.clone(),
                                color: run_color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            });
                            byte_pos = start;
                        }

                        if end > start {
                            runs.push(text_run_for_key(&run_font, key, end.saturating_sub(start)));
                            byte_pos = end;
                        }
                    }

                    if byte_pos < text.len() {
                        runs.push(TextRun {
                            len: text.len().saturating_sub(byte_pos),
                            font: run_font.clone(),
                            color: run_color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        });
                    }
                }

                if runs.is_empty() {
                    runs.push(TextRun {
                        len: text.len(),
                        font: run_font.clone(),
                        color: run_color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    });
                }

                let force_width = cell_width.and_then(|cell_width| {
                    use unicode_width::UnicodeWidthChar as _;
                    let has_wide = text.as_str().chars().any(|ch| ch.width().unwrap_or(0) > 1);
                    (!has_wide).then_some(cell_width)
                });
                let shaped = window
                    .text_system()
                    .shape_line(text, font_size, &runs, force_width);
                *slot = Some(shaped);
            }
        });

        let (
            default_bg,
            shaped_lines,
            selection,
            resolved_selection,
            vp_offset,
            marked_text,
            cursor_position,
        ) = {
            let view = self.view.read(cx);
            // Resolve scrollback/viewport anchors against the live session
            // once — the closure below treats the selection as already
            // projected to current-frame `(screen_row, byte)` pairs.
            let resolved = view.state.selection.and_then(|sel| {
                let (start, end) = sel.normalized(&view.session)?;
                let s = start.resolve(&view.session)?;
                let e = end.resolve(&view.session)?;
                Some((s, e))
            });
            (
                view.session.default_background(),
                view.line_layouts
                    .iter()
                    .map(|line| line.clone().unwrap_or_default())
                    .collect::<Vec<_>>(),
                view.state.selection,
                resolved,
                view.session.viewport_row_offset(),
                view.state.marked_text.clone(),
                view.session.cursor_position(),
            )
        };

        let background_quads =
            self.build_background_quads(bounds, line_height, cell_width_f, default_bg, cx);

        let (marked_text, marked_text_background, preedit_post_shift) = marked_text
            .and_then(|text| {
                if text.is_empty() {
                    return None;
                }
                let (col, grid_row) = cursor_position?;
                // Same scroll-offset dispatch as the cursor: map the live-grid
                // cursor row to its viewport row, skipping the preedit overlay
                // when the cursor has scrolled into history. `row` below is the
                // 1-indexed viewport row used by cell_left_x_for_col / row_index.
                let (total_rows, vp_rows, viewport_top) = {
                    let v = self.view.read(cx);
                    (
                        v.session.total_rows(),
                        v.session.rows() as u32,
                        v.session.viewport_row_offset(),
                    )
                };
                let visible_row = screen_row_to_visible(
                    grid_row_to_screen_row(grid_row, total_rows, vp_rows)?,
                    viewport_top,
                    vp_rows,
                )?;
                let row = (visible_row as u16).saturating_add(1);
                let cell_width = cell_width_f;

                let (origin_x, origin_y) = {
                    let view = self.view.read(cx);
                    let origin_x = cell_left_x_for_col(
                        &view.line_layouts,
                        &view.state.viewport_lines,
                        col,
                        row,
                        cell_width,
                        bounds.left(),
                    );
                    let origin_y = bounds.top() + line_height * (row.saturating_sub(1)) as f32;
                    (origin_x, origin_y)
                };
                let origin = point(origin_x, origin_y);

                let run = TextRun {
                    len: text.len(),
                    font: run_font.clone(),
                    color: run_color,
                    background_color: None,
                    underline: Some(UnderlineStyle {
                        color: Some(run_color),
                        thickness: px(theme::TERMINAL_UNDERLINE_THICKNESS),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                let shaped = window
                    .text_system()
                    .shape_line(text.clone(), font_size, &[run], None);

                let bg = default_bg;

                let cell_len = {
                    let advance = f32::from(shaped.width) / cell_width;
                    (advance.ceil() as usize).max(1)
                };

                let bg_width = {
                    let view = self.view.read(cx);
                    let row_index = row.saturating_sub(1) as usize;
                    let initial_end = col.saturating_add(cell_len as u16).saturating_sub(1);
                    let max_end = initial_end.saturating_add(4);
                    let underlying_span = view
                        .line_layouts
                        .get(row_index)
                        .and_then(|l| l.as_ref())
                        .zip(view.state.viewport_lines.get(row_index))
                        .and_then(|(shaped, line_text)| {
                            let mut end = initial_end;
                            loop {
                                if let Some(range) =
                                    shaped_pixel_range_for_cols(shaped, line_text, col, end)
                                {
                                    return Some(range);
                                }
                                if end >= max_end {
                                    return None;
                                }
                                end = end.saturating_add(1);
                            }
                        })
                        .map(|(x_start, x_end)| x_end - x_start);

                    let grid = px(cell_width * cell_len as f32);
                    let mut bg = shaped.width.max(grid);
                    if let Some(u) = underlying_span {
                        bg = bg.max(u);
                    }
                    bg
                };

                let marked_text_background = fill(
                    Bounds::new(origin, size(bg_width, line_height)),
                    rgba(
                        (u32::from(bg.r) << 24)
                            | (u32::from(bg.g) << 16)
                            | (u32::from(bg.b) << 8)
                            | 0xFF,
                    ),
                );

                let preedit_shift = shaped.width;
                let post_shift = {
                    let view = self.view.read(cx);
                    let row_index = row.saturating_sub(1) as usize;
                    view.state
                        .viewport_lines
                        .get(row_index)
                        .and_then(|line_text| {
                            let split_byte = byte_index_for_column_in_line(line_text, col);
                            let post_text = line_text[split_byte..].trim_end_matches(' ');
                            if post_text.is_empty() {
                                return None;
                            }
                            let post_run = TextRun {
                                len: post_text.len(),
                                font: run_font.clone(),
                                color: run_color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            let force_post = {
                                use unicode_width::UnicodeWidthChar as _;
                                let has_wide =
                                    post_text.chars().any(|ch| ch.width().unwrap_or(0) > 1);
                                (!has_wide).then_some(px(cell_width))
                            };
                            let post_shape = window.text_system().shape_line(
                                post_text.to_string().into(),
                                font_size,
                                &[post_run],
                                force_post,
                            );
                            let erase = fill(
                                Bounds::new(origin, size(bounds.right() - origin.x, line_height)),
                                rgba(
                                    (u32::from(default_bg.r) << 24)
                                        | (u32::from(default_bg.g) << 16)
                                        | (u32::from(default_bg.b) << 8)
                                        | 0xFF,
                                ),
                            );
                            Some(PreeditPostShift {
                                erase,
                                post_shape,
                                post_origin: point(origin.x + preedit_shift, origin.y),
                            })
                        })
                };

                Some(((shaped, origin), marked_text_background, post_shift))
            })
            .map(|(text, bg, shift)| (Some(text), Some(bg), shift))
            .unwrap_or((None, None, None));

        let cols = { self.view.read(cx).session.cols() };
        let grid_right = grid_right_x(bounds.left(), cell_width_f, cols);

        let base_cell_width = Some(cell_width_f);

        let selection_quads = selection
            .map(|sel| {
                let highlight = theme::TERMINAL_SEARCH_HIGHLIGHT;
                let mut quads: Vec<PaintQuad> = Vec::new();

                if sel.is_block() {
                    if let (Some(rect), Some(cell_width)) = (sel.block_rect(), base_cell_width) {
                        let vp_rows = { self.view.read(cx).session.rows() as u32 };
                        for q in block_selection_quads(
                            rect,
                            f32::from(bounds.left()),
                            f32::from(bounds.top()),
                            cell_width,
                            f32::from(line_height),
                            vp_offset,
                            vp_rows,
                        ) {
                            quads.push(fill(
                                Bounds::from_corners(
                                    point(px(q.x1), px(q.y1)),
                                    point(px(q.x2), px(q.y2)),
                                ),
                                highlight,
                            ));
                        }
                    }
                    return quads;
                }

                let Some(((start_row, start_byte), (end_row, end_byte))) = resolved_selection
                else {
                    return quads;
                };
                if (start_row, start_byte) == (end_row, end_byte) {
                    return quads;
                }

                for (row, line) in shaped_lines.iter().enumerate() {
                    let screen_row = vp_offset + row as u32;
                    if screen_row < start_row || screen_row > end_row {
                        continue;
                    }

                    let local_start = if screen_row == start_row {
                        start_byte
                    } else {
                        0
                    };
                    let local_end = if screen_row == end_row {
                        end_byte
                    } else {
                        line.text.len() + 1
                    };

                    let local_end_text = local_end.min(line.text.len());
                    let extends_past_text = local_end > line.text.len();

                    let x1 = line.x_for_index(local_start.min(line.text.len()));
                    let y1 = bounds.top() + line_height * row as f32;
                    let y2 = y1 + line_height;

                    if local_start < line.text.len() && local_end_text > local_start {
                        let x2 = line.x_for_index(local_end_text);
                        quads.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x1, y1),
                                point(bounds.left() + x2, y2),
                            ),
                            highlight,
                        ));
                    }

                    if extends_past_text {
                        let x_start = line.x_for_index(line.text.len());
                        quads.push(fill(
                            Bounds::from_corners(
                                point(bounds.left() + x_start, y1),
                                point(grid_right, y2),
                            ),
                            highlight,
                        ));
                    }
                }

                let content_rows = shaped_lines.len();
                let view = self.view.read(cx);
                let grid_rows = view.session.rows() as usize;
                let end_vp_row = end_row.checked_sub(vp_offset).map(|r| r as usize);
                if let (MouseDragState::TextSelection { row: drag_row }, Some(end_row)) =
                    (view.state.mouse_drag, end_vp_row)
                    && end_row >= content_rows
                    && drag_row >= content_rows
                    && content_rows < grid_rows
                {
                    for row in content_rows..=drag_row.min(grid_rows - 1) {
                        let y1 = bounds.top() + line_height * row as f32;
                        let y2 = y1 + line_height;
                        quads.push(fill(
                            Bounds::from_corners(point(bounds.left(), y1), point(grid_right, y2)),
                            highlight,
                        ));
                    }
                }

                quads
            })
            .unwrap_or_default();

        let box_drawing_quads = {
            let cell_width = cell_width_f;
            use unicode_width::UnicodeWidthChar as _;
            let default_fg = run_color;
            let mut quads = Vec::new();

            let view = self.view.read(cx);
            for (row, line) in view.state.viewport_lines.iter().enumerate() {
                if !line_has_box_drawing(line) {
                    continue;
                }
                let y = bounds.top() + line_height * row as f32;
                let runs = view
                    .state
                    .viewport_style_runs
                    .get(row)
                    .map(|v| v.as_slice());
                let mut run_idx: usize = 0;

                let mut col = 1usize;
                for ch in line.chars() {
                    let width = ch.width().unwrap_or(0);
                    if width == 0 {
                        continue;
                    }

                    if box_drawing_mask(ch).is_some()
                        || block_glyph_for_char(ch).is_some()
                        || powerline_for_char(ch).is_some()
                    {
                        let fg = runs
                            .and_then(|runs| {
                                while let Some(run) = runs.get(run_idx) {
                                    if (col as u16) <= run.end_col {
                                        break;
                                    }
                                    run_idx = run_idx.saturating_add(1);
                                }
                                runs.get(run_idx).and_then(|run| {
                                    (col as u16 >= run.start_col && (col as u16) <= run.end_col)
                                        .then_some(run)
                                })
                            })
                            .map(|run| {
                                let key = TextRunKey {
                                    fg: run.fg,
                                    flags: run.flags
                                        & (CELL_STYLE_FLAG_FAINT
                                            | CELL_STYLE_FLAG_BOLD
                                            | CELL_STYLE_FLAG_ITALIC
                                            | CELL_STYLE_FLAG_UNDERLINE
                                            | CELL_STYLE_FLAG_STRIKETHROUGH),
                                };
                                color_for_key(key)
                            })
                            .unwrap_or(default_fg);

                        let col_u16 = col as u16;
                        let (x_left, x_right) = shaped_lines
                            .get(row)
                            .zip(Some(line.as_str()))
                            .and_then(|(shaped, text)| {
                                shaped_pixel_range_for_cols(shaped, text, col_u16, col_u16)
                            })
                            .unwrap_or_else(|| {
                                let base = px(cell_width * (col.saturating_sub(1)) as f32);
                                (base, base + px(cell_width))
                            });
                        let x = bounds.left() + x_left;
                        let cell_bounds =
                            Bounds::new(point(x, y), size(x_right - x_left, line_height));
                        quads.extend(box_drawing_quads_for_char(
                            cell_bounds,
                            line_height,
                            cell_width,
                            fg,
                            ch,
                        ));
                    }

                    col = col.saturating_add(width);
                }
            }

            quads
        };

        let cursor = {
            let view = self.view.read(cx);
            let focused = view.focus_handle.is_focused(window);
            let has_marked = view.state.marked_text.is_some();
            let cursor_visible = view.session.cursor_visible();
            let search_overlay = view.state.search_overlay;

            if focused && !search_overlay && cursor_visible && !has_marked {
                Some((view.session.cursor_position(), view.session.cursor_style()))
            } else {
                None
            }
        }
        .and_then(|(pos, style_code)| {
            let (col, grid_row) = pos?;
            // Map the live-grid cursor row to its viewport row. When the
            // viewport is scrolled into history the cursor is off-screen, so
            // bail rather than paint it over scrollback — the same
            // scroll-offset dispatch as dump_viewport and the prompt/search
            // overlays. `row` below is the 1-indexed viewport row.
            let (background, total_rows, vp_rows, viewport_top) = {
                let v = self.view.read(cx);
                (
                    v.session.default_background(),
                    v.session.total_rows(),
                    v.session.rows() as u32,
                    v.session.viewport_row_offset(),
                )
            };
            let visible_row = screen_row_to_visible(
                grid_row_to_screen_row(grid_row, total_rows, vp_rows)?,
                viewport_top,
                vp_rows,
            )?;
            let row = (visible_row as u16).saturating_add(1);
            let cursor_color = cursor_color_for_background(background);
            let y = bounds.top() + line_height * (row.saturating_sub(1)) as f32;
            let row_index = row.saturating_sub(1) as usize;
            let line = shaped_lines.get(row_index)?;
            let x = cursor_x_for_col(line, col, bounds.left());
            let cursor_w = cursor_width_for_col(line, col, cell_width_f);

            let cursor_bounds = match style_code {
                3 | 4 => {
                    let underline_height = 2.0_f32;
                    Bounds::new(
                        point(x, y + line_height - px(underline_height)),
                        size(px(cursor_w), px(underline_height)),
                    )
                }
                5 | 6 => Bounds::new(point(x, y), size(px(theme::CURSOR_BAR_W), line_height)),
                _ => Bounds::new(point(x, y), size(px(cursor_w), line_height)),
            };

            Some(fill(cursor_bounds, cursor_color))
        });

        let flash = self.view.read(cx).state.flash;
        let bell_flash =
            flash_overlay_if_active(flash.bell, || fill(bounds, theme::BELL_FLASH_OVERLAY));

        let prompt_jump_flash = flash_overlay_if_active(flash.prompt_jump, || {
            let stripe = Bounds::new(
                bounds.origin,
                gpui::size(bounds.size.width, px(theme::PROMPT_JUMP_FLASH_STRIPE_H)),
            );
            fill(stripe, theme::PROMPT_JUMP_FLASH_STRIPE)
        });

        let scrollbar = {
            let (total_rows, viewport_rows, viewport_offset, is_dragging) = {
                let v = self.view.read(cx);
                (
                    v.session.total_rows(),
                    v.session.rows() as u32,
                    v.session.viewport_row_offset(),
                    matches!(v.state.mouse_drag, MouseDragState::ScrollbarDrag { .. }),
                )
            };
            let (quad, new_thumb_bounds) = if total_rows > viewport_rows {
                let track_height = f32::from(bounds.size.height);
                let thumb_ratio = viewport_rows as f32 / total_rows as f32;
                let thumb_height =
                    (track_height * thumb_ratio).max(theme::TERMINAL_SCROLLBAR_THUMB_MIN_H);
                let scrollable = total_rows - viewport_rows;
                let thumb_top = if scrollable > 0 {
                    let fraction = viewport_offset as f32 / scrollable as f32;
                    fraction * (track_height - thumb_height)
                } else {
                    track_height - thumb_height
                };

                let thumb_bounds = Bounds::new(
                    point(
                        bounds.right()
                            - px(theme::TERMINAL_SCROLLBAR_W + theme::TERMINAL_SCROLLBAR_MARGIN),
                        bounds.top() + px(thumb_top),
                    ),
                    size(px(theme::TERMINAL_SCROLLBAR_W), px(thumb_height)),
                );
                let thumb_color = if is_dragging {
                    theme::TERMINAL_SCROLLBAR_THUMB_ACTIVE
                } else {
                    theme::TERMINAL_SCROLLBAR_THUMB
                };
                (Some(fill(thumb_bounds, thumb_color)), Some(thumb_bounds))
            } else {
                (None, None)
            };
            self.view.update(cx, |v, _| {
                v.state.scrollbar_thumb_bounds = new_thumb_bounds;
            });
            quad
        };

        let search_quads = self.build_search_quads(bounds, line_height, &shaped_lines, cx);
        let prompt_marks = self.build_prompt_marks(bounds, line_height, cx);
        let hover_underline =
            self.build_hover_underline(bounds, line_height, &shaped_lines, run_color, cx);
        let search_scrollbar_ticks = self.build_search_scrollbar_ticks(bounds, cx);
        let (annotation_quads, annotation_text_lines) = self.build_annotation_quads(
            bounds,
            line_height,
            cell_width_f,
            &shaped_lines,
            window,
            &run_font,
            font_size,
            cx,
        );

        TerminalPrepaintState {
            line_height,
            shaped_lines,
            background_quads,
            selection_quads,
            box_drawing_quads,
            marked_text,
            marked_text_background,
            preedit_post_shift,
            cursor,
            scrollbar,
            bell_flash,
            prompt_jump_flash,
            hover_underline,
            prompt_marks,
            search_quads,
            search_scrollbar_ticks,
            annotation_quads,
            annotation_text_lines,
        }
    }
}
