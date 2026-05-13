use super::{TerminalPrepaintState, TerminalTextElement};
use gpui::{App, Bounds, ElementInputHandler, Pixels, Window};

impl TerminalTextElement {
    pub(super) fn do_paint(
        &mut self,
        bounds: Bounds<Pixels>,
        prepaint: &mut TerminalPrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.view.update(cx, |view, _cx| {
            view.state.last_bounds = Some(bounds);
        });

        let focus_handle = { self.view.read(cx).focus_handle.clone() };
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );

        window.paint_layer(bounds, |window| {
            for quad in prepaint.background_quads.drain(..) {
                window.paint_quad(quad);
            }

            for quad in prepaint.prompt_marks.drain(..) {
                window.paint_quad(quad);
            }

            for quad in prepaint.search_quads.drain(..) {
                window.paint_quad(quad);
            }

            for quad in prepaint.selection_quads.drain(..) {
                window.paint_quad(quad);
            }

            let origin = bounds.origin;
            for (row, line) in prepaint.shaped_lines.iter().enumerate() {
                let y = origin.y + prepaint.line_height * row as f32;
                let _ = line.paint(
                    gpui::point(origin.x, y),
                    prepaint.line_height,
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }

            for quad in prepaint.box_drawing_quads.drain(..) {
                window.paint_quad(quad);
            }

            if prepaint.marked_text_background.is_some()
                || prepaint.marked_text.is_some()
                || prepaint.preedit_post_shift.is_some()
            {
                window.paint_layer(bounds, |window| {
                    if let Some(shift) = prepaint.preedit_post_shift.as_ref() {
                        window.paint_quad(shift.erase.clone());
                    }
                    if let Some(bg) = prepaint.marked_text_background.take() {
                        window.paint_quad(bg);
                    }
                    if let Some(shift) = prepaint.preedit_post_shift.as_ref() {
                        let _ = shift.post_shape.paint(
                            shift.post_origin,
                            prepaint.line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                    if let Some((line, origin)) = prepaint.marked_text.as_ref() {
                        let _ = line.paint(
                            *origin,
                            prepaint.line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                });
            }

            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }

            if let Some(scrollbar) = prepaint.scrollbar.take() {
                window.paint_quad(scrollbar);
            }

            for tick in prepaint.search_scrollbar_ticks.drain(..) {
                window.paint_quad(tick);
            }

            if let Some(hover) = prepaint.hover_underline.take() {
                window.paint_quad(hover);
            }

            if let Some(bell) = prepaint.bell_flash.take() {
                window.paint_quad(bell);
            }

            if let Some(flash) = prepaint.prompt_jump_flash.take() {
                window.paint_quad(flash);
            }
        });
    }
}
