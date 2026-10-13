//! Mouse-move handling: drag selection, autoscroll while dragging, and
//! URL / annotation hover tracking.

use super::*;

impl TerminalView {
    pub(in crate::view) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.shift && self.input.is_some() && self.session.mouse_reporting_enabled()
        {
            let send_motion = if self.session.mouse_any_event_enabled() {
                true
            } else if self.session.mouse_button_event_enabled() {
                event.pressed_button.is_some()
            } else {
                false
            };

            if send_motion {
                let Some((col, row)) = self.mouse_position_to_cell(event.position, window) else {
                    return;
                };

                let base_button = match event.pressed_button {
                    Some(MouseButton::Left) => 0,
                    Some(MouseButton::Middle) => 1,
                    Some(MouseButton::Right) => 2,
                    Some(_) => 3,
                    None => 3,
                };

                let button_value = sgr_mouse_button_value(
                    base_button,
                    true,
                    false,
                    event.modifiers.alt,
                    event.modifiers.control,
                );
                if let Some(input) = self.input.as_ref() {
                    if self.session.mouse_sgr_enabled() {
                        let seq = sgr_mouse_sequence(button_value, col, row, true);
                        input.send(seq.as_bytes());
                    } else {
                        let seq = normal_mouse_sequence(
                            button_value,
                            col,
                            row,
                            self.session.mouse_utf8_enabled(),
                        );
                        if !seq.is_empty() {
                            input.send(&seq);
                        }
                    }
                }
                return;
            }
        }

        // Scrollbar thumb drag.
        if let MouseDragState::ScrollbarDrag {
            offset: click_offset,
        } = self.state.mouse_drag
        {
            if event.pressed_button == Some(MouseButton::Left) {
                if let Some(bounds) = self.state.last_bounds {
                    let track_height = f32::from(bounds.size.height);
                    let current_y = f32::from(event.position.y) - f32::from(bounds.origin.y);
                    let total = self.session.total_rows();
                    let rows = self.session.rows() as u32;
                    // Recompute thumb height to derive the usable track range.
                    let thumb_ratio = rows as f32 / total as f32;
                    let thumb_height = (track_height * thumb_ratio)
                        .max(crate::ux::theme::TERMINAL_SCROLLBAR_THUMB_MIN_H);
                    // Maintain the offset between the cursor and the thumb top
                    // so the thumb doesn't jump on the first move.
                    let new_thumb_top =
                        (current_y - click_offset).clamp(0.0, track_height - thumb_height);
                    let usable = track_height - thumb_height;
                    let fraction = if usable > 0.0 {
                        (new_thumb_top / usable).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let scrollable = total.saturating_sub(rows);
                    let offset = (fraction * scrollable as f32).round() as i32;
                    let current = self.session.viewport_row_offset() as i32;
                    let delta = offset - current;
                    if delta != 0 {
                        // Same settle path as the wheel: reanchor (unlock at
                        // the live edge) + selection-preserving repaint. Using
                        // `refresh_viewport` here would clear the selection and
                        // strand the hovered-URL underline, unlike every other
                        // user-scroll path.
                        self.scroll_viewport_and_sync(delta);
                        self.reanchor_and_refresh(cx);
                    }
                }
                return;
            } else {
                self.state.mouse_drag = MouseDragState::None;
                cx.notify();
            }
        }

        self.update_hovered_url(event.position, event.modifiers.platform, window, cx);
        self.update_hovered_annotation(event.position, window, cx);

        // If we thought we were dragging but the left button is no longer
        // pressed, the button was released outside the window where
        // on_mouse_up never fires.  Treat re-entry without the button as
        // an implicit mouse-up.
        if matches!(self.state.mouse_drag, MouseDragState::TextSelection { .. })
            && event.pressed_button != Some(MouseButton::Left)
        {
            self.end_mouse_drag();
            match self.state.selection {
                Some(sel) if sel.is_empty() => self.state.selection = None,
                // A drag let go outside the pane still settles a selection.
                Some(_) => self.copy_settled_selection(cx),
                None => {}
            }
            cx.notify();
            return;
        }

        if !event.dragging() {
            return;
        }

        if self.state.selection.is_none() {
            return;
        }

        let drag_row = self.mouse_row_index(event.position, window);
        if let MouseDragState::TextSelection { row } = &mut self.state.mouse_drag {
            *row = drag_row;
        }

        let Some(screen_pos) = self.mouse_position_to_screen_pos(event.position, window) else {
            return;
        };

        // For Block mode we also need the Side-aware cell anchor
        // under the cursor so the rectangle can extend into trailing
        // blank space past the row's last glyph and apply Alacritty's
        // range_simple Side trim. Linear mode ignores it.
        let cell_anchor_under_cursor = self.cell_anchor_at(event.position, window);

        if let Some(selection) = self.state.selection.as_mut() {
            let mut changed = false;
            if selection.active != screen_pos {
                selection.active = screen_pos;
                changed = true;
            }
            if selection.is_block() && selection.block_active != cell_anchor_under_cursor {
                selection.block_active = cell_anchor_under_cursor;
                changed = true;
            }
            if changed {
                cx.notify();
            }
        }
    }

    /// Clear all mouse-drag state (text-selection or scrollbar) and cancel any
    /// autoscroll polling. The single entry point for ending a drag, so new
    /// drag-state additions can't be silently missed by one of the two mouse-up
    /// paths.
    pub(super) fn end_mouse_drag(&mut self) {
        self.state.mouse_drag = MouseDragState::None;
        self.autoscroll_task = None;
    }

    /// Spawn the autoscroll polling task.  Called on every left-button
    /// mouse-down that creates a selection.  The task polls
    /// `window.mouse_position()` every 50 ms (which wraps
    /// `NSWindow.mouseLocationOutsideOfEventStream`) — it therefore
    /// works even when the cursor is outside the window frame where
    /// `on_mouse_move` stops firing.
    ///
    /// `cell_h` is captured here (while `window` is available) so the
    /// async task always uses the font metrics at drag-start time rather
    /// than a stale field that would not reflect a zoom or config change
    /// that happened before the drag began.
    pub(in crate::view) fn start_autoscroll(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.mouse_drag = MouseDragState::TextSelection { row: 0 };
        // Lock the viewport so PTY output does not snap back to the bottom
        // while the user is dragging to make a selection. Mirrors iTerm2's
        // selectionScrollWillStart which sets userScroll = YES unconditionally.
        self.state
            .viewport_lock
            .lock(self.session.viewport_top_abs_y());
        let entity = cx.entity().downgrade();
        let cell_h = self
            .cell_layout(window)
            .map(|l| l.line_height)
            .unwrap_or(16.0)
            .max(4.0);
        self.autoscroll_task = Some(window.spawn(cx, async move |cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let keep_going = cx
                    .update(|window, cx| {
                        let mouse_pos = window.mouse_position();
                        entity
                            .upgrade()
                            .map(|e| {
                                e.update(cx, |tv, cx| {
                                    tv.autoscroll_poll_with_pos(mouse_pos, cell_h, cx)
                                })
                            })
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    /// One auto-scroll polling step.  Scrolls the viewport when the
    /// cursor (supplied by the caller from `window.mouse_position()`)
    /// is outside the terminal bounds.  Returns `false` to signal the
    /// task loop to exit.
    fn autoscroll_poll_with_pos(
        &mut self,
        mouse_pos: gpui::Point<Pixels>,
        cell_h: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(self.state.mouse_drag, MouseDragState::TextSelection { .. }) {
            return false;
        }
        let Some(vb) = self.state.last_bounds else {
            return true;
        };
        let local_y = f32::from(mouse_pos.y) - f32::from(vb.origin.y);
        let viewport_h = f32::from(vb.size.height);

        let cell_h = cell_h.max(4.0);
        let vel: i32 = if local_y < 0.0 {
            -((-local_y / cell_h).ceil() as i32).min(3)
        } else if local_y > viewport_h {
            (((local_y - viewport_h) / cell_h).ceil() as i32).min(3)
        } else {
            return true;
        };

        self.scroll_viewport_and_sync(vel);
        // Keep the viewport lock anchored to the new position so PTY output
        // arriving during an autoscroll burst does not fight the autoscroll.
        // Unlock if autoscroll has dragged us back to the live edge.
        self.reanchor_viewport_lock();

        // Re-query the post-scroll geometry for the selection update —
        // `reanchor_viewport_lock` reads these internally but does not
        // expose them.
        let vp_offset = self.session.viewport_row_offset();
        let rows = self.session.rows() as u32;
        if let Some(sel) = self.state.selection.as_mut() {
            // Autoscroll extends the active endpoint to the topmost /
            // bottommost row of the new viewport. The previous byte
            // offset is no longer meaningful at the new row — drop to
            // column 0 of the new row in current-frame coordinates.
            let target_row = if vel < 0 {
                vp_offset
            } else {
                vp_offset + rows.saturating_sub(1)
            };
            sel.active = ScreenPos::viewport(target_row, 0);
        }

        self.schedule_viewport_refresh(cx);
        true
    }

    /// Side-aware pixel→cell mapper. Uses `cell_dimensions` so the
    /// view's configured `font_size` / spacing is the source of
    /// truth — not `window.text_style()`, which is unset during
    /// mouse events.
    ///
    /// `pixel_to_cell_anchor` returns a viewport-relative row;
    /// `viewport_row_offset` is added here so the returned anchor
    /// carries an **absolute** screen row (matching `BlockRect.{top,
    /// bottom}`'s post-Task-7 convention). When the click lands at
    /// viewport row 1 the absolute row is `vp_top + 1`, which can
    /// extend into `LineBuffer` scrollback once the viewport is
    /// scrolled back.
    pub(in crate::view) fn cell_anchor_at(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<super::super::CellAnchor> {
        let layout = self.cell_layout(window)?;
        let local = self.mouse_position_to_local(position);
        let mut anchor = super::super::pixel_to_cell_anchor(
            f32::from(local.x),
            f32::from(local.y),
            layout.cell_width,
            layout.line_height,
            self.session.cols(),
            self.session.rows(),
        );
        anchor.row = anchor
            .row
            .saturating_add(self.session.viewport_row_offset());
        Some(anchor)
    }

    /// Recompute `hovered_annotation` based on the current cursor
    /// position. The lookup goes straight through
    /// `TerminalSession::annotation_at_point` — no separate hit-test
    /// cache — since each visible row contains at most one annotation
    /// today and the tree's `at_line` walk is O(matches).
    fn update_hovered_annotation(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = self
            .mouse_position_to_cell(position, window)
            .and_then(|(col, row)| {
                // 1-indexed → 0-indexed visual row; convert to LineCoord.
                let row0 = row.saturating_sub(1) as u32;
                let screen_row = self.session.viewport_row_offset().saturating_add(row0);
                let line_coord = self.session.screen_row_to_line_coord(screen_row)?;
                self.session
                    .annotation_at_point(line_coord, col)
                    .map(|(id, _)| id)
            });
        // Route through the public setter so the no-op short-circuit
        // and `cx.notify()` live in exactly one place.
        self.set_hovered_annotation(next, cx);
    }

    /// Recompute `hovered_url` based on the current cursor position and
    /// whether the Cmd (platform) modifier is held. Mirrors Alacritty's
    /// `update_highlighted_hints` (input/mod.rs:1106).
    fn update_hovered_url(
        &mut self,
        position: gpui::Point<Pixels>,
        modifier_held: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = if modifier_held {
            self.mouse_position_to_cell(position, window)
                .and_then(|(col, row)| {
                    let row0 = row.saturating_sub(1);
                    let line = self.state.viewport_lines.get(row0 as usize)?;
                    let (start_col, end_col, url) =
                        super::super::url::url_range_at_column_in_line(line, col)?;
                    Some(super::super::HoveredUrl {
                        row: row0,
                        start_col,
                        end_col,
                        url: gpui::SharedString::from(url),
                    })
                })
        } else {
            None
        };

        if self.state.hovered_url != next {
            self.state.hovered_url = next;
            cx.notify();
        }
    }
}
