use gpui::{
    Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollDelta, ScrollWheelEvent, Window, point, px,
};

use super::selection::ScreenPos;
use super::url::url_at_column_in_line;
use super::{ByteSelection, TerminalView};
use gpui::ClipboardItem;

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Normal (X10/X11) mouse report: `ESC [ M <button+32> <col_enc> <row_enc>`.
///
/// `button` is the raw button value (0=left press, 1=middle press, 2=right
/// press, 3=release).  Modifier bits are added by the caller the same way as
/// for SGR.  When `utf8` is true and a 0-indexed coordinate exceeds 94,
/// the coordinate is encoded as a 2-byte UTF-8 sequence matching Alacritty's
/// `normal_mouse_report`.  Returns an empty `Vec` when the position falls
/// outside the encodable range (> 222 without UTF-8, > 2014 with).
///
/// `col` and `row` are **1-indexed** (as returned by `mouse_position_to_cell`).
pub(crate) fn normal_mouse_sequence(button: u8, col: u16, row: u16, utf8: bool) -> Vec<u8> {
    let col0 = col.saturating_sub(1);
    let row0 = row.saturating_sub(1);
    let max0: u16 = if utf8 { 2014 } else { 222 };
    if col0 > max0 || row0 > max0 {
        return Vec::new();
    }
    let mut msg = vec![b'\x1b', b'[', b'M', button.saturating_add(32)];
    let encode = |pos0: u16| -> Vec<u8> {
        let v = 32usize + 1 + pos0 as usize;
        if utf8 && pos0 >= 95 {
            vec![(0xC0 + v / 64) as u8, (0x80 + (v & 63)) as u8]
        } else {
            vec![v as u8]
        }
    };
    msg.extend(encode(col0));
    msg.extend(encode(row0));
    msg
}

pub(crate) fn sgr_mouse_button_value(
    base_button: u8,
    motion: bool,
    shift: bool,
    alt: bool,
    control: bool,
) -> u8 {
    let mut value = base_button;
    if motion {
        value = value.saturating_add(32);
    }
    if shift {
        value = value.saturating_add(4);
    }
    if alt {
        value = value.saturating_add(8);
    }
    if control {
        value = value.saturating_add(16);
    }
    value
}

pub(crate) fn sgr_mouse_sequence(button_value: u8, col: u16, row: u16, pressed: bool) -> String {
    crate::ansi::sgr_mouse_report(button_value, col, row, pressed)
}

pub(super) fn window_position_to_local(
    last_bounds: Option<Bounds<Pixels>>,
    position: gpui::Point<Pixels>,
) -> gpui::Point<Pixels> {
    let origin = last_bounds
        .map(|bounds| bounds.origin)
        .unwrap_or_else(|| point(px(0.0), px(0.0)));
    point(position.x - origin.x, position.y - origin.y)
}

// ---------------------------------------------------------------------------
// TerminalView — mouse & scroll
// ---------------------------------------------------------------------------

impl TerminalView {
    pub(super) fn mouse_row_index(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> usize {
        let local_pos = self.mouse_position_to_local(position);
        let y = f32::from(local_pos.y);
        let cell_height = self
            .cell_layout(window)
            .map(|l| l.line_height)
            .unwrap_or(16.0);
        let row = (y / cell_height).floor() as i32;
        row.max(0) as usize
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.state.drag_row = None;

        if event.first_mouse {
            return;
        }

        // Scrollbar thumb drag — must be checked before the general
        // selection path so a click on the thumb doesn't start a
        // text selection.
        if event.button == MouseButton::Left {
            if let Some(thumb) = self.state.scrollbar_thumb_bounds {
                if thumb.contains(&event.position) {
                    let bounds = self.state.last_bounds.unwrap_or(thumb);
                    // Store the cursor's offset from the thumb top so the
                    // thumb doesn't jump on the first move event.
                    let cursor_y = f32::from(event.position.y) - f32::from(bounds.origin.y);
                    let thumb_top = f32::from(thumb.origin.y) - f32::from(bounds.origin.y);
                    let click_offset = cursor_y - thumb_top;
                    self.state.scrollbar_drag_start = Some(click_offset);
                    cx.notify();
                    return;
                }
                // Click on the track (outside thumb): jump to that fraction.
                let bounds = self.state.last_bounds.unwrap_or(thumb);
                let track_right = f32::from(bounds.right());
                let click_x = f32::from(event.position.x);
                if click_x
                    >= track_right
                        - (crate::ux::theme::TERMINAL_SCROLLBAR_W
                            + crate::ux::theme::TERMINAL_SCROLLBAR_MARGIN)
                    && click_x <= track_right
                {
                    let track_height = f32::from(bounds.size.height);
                    let click_y = f32::from(event.position.y) - f32::from(bounds.origin.y);
                    let fraction = (click_y / track_height).clamp(0.0, 1.0);
                    let total = self.session.total_rows();
                    let rows = self.session.rows() as u32;
                    let scrollable = total.saturating_sub(rows);
                    let offset = (fraction * scrollable as f32).round() as i32;
                    let current = self.session.viewport_row_offset() as i32;
                    let delta = offset - current;
                    if delta != 0 {
                        let _ = self.session.scroll_viewport(delta);
                        self.refresh_viewport();
                        cx.notify();
                    }
                    return;
                }
            }
        }

        // Commit any pending Hangul composition before the click is
        // routed. Otherwise a drag-select started mid-composition
        // anchors at a position that doesn't include the composed
        // glyph (the preedit only lives in the overlay, not the
        // selection's source bytes), and a Cmd+click hyperlink open
        // would leave a stale syllable inside the composer.
        self.flush_hangul(cx);

        // While the search overlay is open, any click outside the
        // search bar means "return to the terminal". The bar's own
        // interactive children stop propagation, and its panel root
        // swallows background clicks — so reaching this handler
        // implies the click landed on the terminal area.
        //
        // Keep `self.state.search` intact so the existing match
        // highlights stay rendered after the bar is hidden — the
        // user can still see where their hits are and reopen the
        // bar via Cmd+F to refine. Explicit dismissal (Esc or the
        // close button) still calls `on_search_close`, which also
        // clears the state.
        if self.state.search_overlay {
            self.state.search_overlay = false;
            cx.notify();
            return;
        }

        if event.button == MouseButton::Left && event.modifiers.platform {
            if let Some((col, row)) = self.mouse_position_to_cell(event.position, window) {
                if let Some(link) = self.session.hyperlink_at(col, row) {
                    cx.open_url(&link);
                    cx.write_to_clipboard(ClipboardItem::new_string(link));
                    return;
                }

                if let Some(line) = self
                    .state
                    .viewport_lines
                    .get(row.saturating_sub(1) as usize)
                    && let Some(url) = url_at_column_in_line(line, col)
                {
                    cx.open_url(&url);
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    return;
                }
            }

            if let Some(pos) = self.mouse_position_to_screen_pos(event.position, window)
                && let Some(url) = self.url_at_screen_pos(pos)
            {
                cx.open_url(&url);
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                return;
            }
        }

        // Shift+Right-click matches the iTerm2/kitty/Alacritty convention
        // for opening a host context menu. Fires unconditionally, including
        // under SGR mouse capture — the gesture is specifically intended to
        // escape PTY capture. Subscribers consume the emitted event to build
        // the actual menu, which the terminal crate does not own.
        if event.button == MouseButton::Right && event.modifiers.shift {
            let range = self.selection_single_line_range();
            cx.emit(super::TerminalViewEvent::ContextMenuRequested {
                position: event.position,
                range,
            });
            cx.stop_propagation();
            return;
        }

        // Double-click on an annotation overlay opens the edit dialog.
        // The hit test runs against the session's interval tree at the
        // clicked cell — when a mark covers that cell the click is
        // intercepted before the regular word-selection path kicks in.
        if event.button == MouseButton::Left
            && event.click_count == 2
            && let Some((col, row)) = self.mouse_position_to_cell(event.position, window)
        {
            let row0 = row.saturating_sub(1) as u32;
            let screen_row = self.session.viewport_row_offset().saturating_add(row0);
            if let Some(line_coord) = self.session.screen_row_to_line_coord(screen_row)
                && let Some((mark_id, _)) = self.session.annotation_at_point(line_coord, col)
            {
                cx.emit(super::TerminalViewEvent::AnnotationDoubleClicked { id: mark_id });
                cx.stop_propagation();
                return;
            }
        }

        if event.modifiers.shift || self.input.is_none() || !self.session.mouse_reporting_enabled()
        {
            if event.button == MouseButton::Left
                && let Some(pos) = self.mouse_position_to_screen_pos(event.position, window)
            {
                let vp_offset = self.session.viewport_row_offset();
                match event.click_count {
                    2 => {
                        let (start, end) = self.word_range_at(pos, vp_offset);
                        self.state.selection = Some(ByteSelection::linear(start, end));
                    }
                    3 => {
                        let (start, end) = self.line_range_at(pos, vp_offset);
                        self.state.selection = Some(ByteSelection::linear(start, end));
                    }
                    _ => {
                        // Alt (Option) → block mode. Cell anchor is the
                        // current cursor position under the click.
                        let mode = super::selection_mode_from_modifiers(event.modifiers.alt);
                        let selection = match mode {
                            super::SelectionMode::Linear => ByteSelection::linear(pos, pos),
                            super::SelectionMode::Block => {
                                // Use the Side-aware mapper so a
                                // click on the right half of cell C
                                // anchors the block at C+1's left
                                // edge (Alacritty rule).
                                let cell = self
                                    .cell_anchor_at(event.position, window)
                                    .unwrap_or(super::CellAnchor::new(1, 1, super::Side::Left));
                                ByteSelection::block(pos, cell)
                            }
                        };
                        self.state.selection = Some(selection);
                    }
                }
                cx.notify();
                self.start_autoscroll(window, cx);
            }
            return;
        }

        let Some((col, row)) = self.mouse_position_to_cell(event.position, window) else {
            return;
        };

        if let Some(input) = self.input.as_ref() {
            let base_button = match event.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };

            let button_value = sgr_mouse_button_value(
                base_button,
                false,
                false,
                event.modifiers.alt,
                event.modifiers.control,
            );
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
    }

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.drag_row = None;
        self.state.is_dragging = false;
        self.autoscroll_task = None;
        if event.button == MouseButton::Left && self.state.scrollbar_drag_start.is_some() {
            self.state.scrollbar_drag_start = None;
            cx.notify();
            return;
        }
        if event.modifiers.shift || self.input.is_none() || !self.session.mouse_reporting_enabled()
        {
            if let Some(selection) = self.state.selection {
                if selection.is_empty() {
                    self.state.selection = None;
                }
                cx.notify();
            }
            return;
        }

        // X10 mode (1000) is press-only. Do not send release events.
        if self.session.mouse_x10_only() {
            return;
        }

        let Some((col, row)) = self.mouse_position_to_cell(event.position, window) else {
            return;
        };

        if let Some(input) = self.input.as_ref() {
            let base_button = match event.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };

            let mods_value = sgr_mouse_button_value(
                0,
                false,
                false,
                event.modifiers.alt,
                event.modifiers.control,
            );
            if self.session.mouse_sgr_enabled() {
                let button_value = base_button + mods_value;
                let seq = sgr_mouse_sequence(button_value, col, row, false);
                input.send(seq.as_bytes());
            } else {
                // Normal mode: release is always encoded as button 3
                let release_value = 3u8 + mods_value;
                let seq = normal_mouse_sequence(
                    release_value,
                    col,
                    row,
                    self.session.mouse_utf8_enabled(),
                );
                if !seq.is_empty() {
                    input.send(&seq);
                }
            }
        }
    }

    pub(super) fn on_mouse_move(
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
        if let Some(click_offset) = self.state.scrollbar_drag_start {
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
                        let _ = self.session.scroll_viewport(delta);
                        self.refresh_viewport();
                        cx.notify();
                    }
                }
                return;
            } else {
                self.state.scrollbar_drag_start = None;
                cx.notify();
            }
        }

        self.update_hovered_url(event.position, event.modifiers.platform, window, cx);
        self.update_hovered_annotation(event.position, window, cx);

        // If we thought we were dragging but the left button is no longer
        // pressed, the button was released outside the window where
        // on_mouse_up never fires.  Treat re-entry without the button as
        // an implicit mouse-up.
        if self.state.is_dragging && event.pressed_button != Some(MouseButton::Left) {
            self.state.is_dragging = false;
            self.autoscroll_task = None;
            self.state.drag_row = None;
            if self.state.selection.map(|s| s.is_empty()).unwrap_or(false) {
                self.state.selection = None;
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
        self.state.drag_row = Some(drag_row);

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
    pub(super) fn start_autoscroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.is_dragging = true;
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
        if !self.state.is_dragging {
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

        let _ = self.session.scroll_viewport(vel);
        self.sync_viewport_scroll_tracking();

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
    pub(super) fn cell_anchor_at(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<super::CellAnchor> {
        let layout = self.cell_layout(window)?;
        let local = self.mouse_position_to_local(position);
        let mut anchor = super::pixel_to_cell_anchor(
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
    /// in SP-1 and the tree's `at_line` walk is O(matches).
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
                        super::url::url_range_at_column_in_line(line, col)?;
                    Some(super::HoveredUrl {
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

    pub(super) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dy_lines: f32 = match event.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / 16.0,
        };

        let delta_lines = (-dy_lines).round() as i32;
        if delta_lines == 0 {
            return;
        }

        // Alt-screen + alternate-scroll mode + no mouse reporting → send arrow
        // keys so wheel scroll works in vim/less/man.  Matches Alacritty's
        // ALT_SCREEN | ALTERNATE_SCROLL path.
        if self.session.is_alt_screen()
            && !self.session.mouse_reporting_enabled()
            && self.session.alternate_scroll_enabled()
        {
            if let Some(input) = self.input.as_ref() {
                let arrow = if delta_lines < 0 {
                    crate::ansi::CURSOR_UP
                } else {
                    crate::ansi::CURSOR_DOWN
                };
                let steps = delta_lines.unsigned_abs().min(10) as usize;
                for _ in 0..steps {
                    input.send(arrow);
                }
            }
            return;
        }

        if let Some(input) = self.input.as_ref()
            && !event.modifiers.shift
            && self.session.mouse_reporting_enabled()
        {
            let Some((col, row)) = self.mouse_position_to_cell(event.position, window) else {
                return;
            };

            let button = if delta_lines < 0 { 64u8 } else { 65u8 };
            let button_value = sgr_mouse_button_value(
                button,
                false,
                false,
                event.modifiers.alt,
                event.modifiers.control,
            );
            let steps = delta_lines.unsigned_abs().min(10);
            if self.session.mouse_sgr_enabled() {
                for _ in 0..steps {
                    let seq = sgr_mouse_sequence(button_value, col, row, true);
                    input.send(seq.as_bytes());
                }
            } else {
                for _ in 0..steps {
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

        let offset_before = self.session.viewport_row_offset();
        let _ = self.session.scroll_viewport(delta_lines);
        if self.session.viewport_row_offset() != offset_before {
            self.sync_viewport_scroll_tracking();
            self.apply_side_effects(cx);
            self.state
                .viewport_pin
                .pin(self.session.viewport_top_abs_y());
            self.schedule_viewport_refresh(cx);
        }

        // While the left button is held (is_dragging), extend the selection
        // active endpoint to the row now under the cursor so scrolling during
        // a drag extends the selection — matching iTerm2 / Alacritty behaviour.
        if self.state.is_dragging
            && self
                .state
                .selection
                .as_ref()
                .map(|s| !s.is_block())
                .unwrap_or(false)
            && let Some(new_pos) = self.mouse_position_to_screen_pos(event.position, window)
            && let Some(sel) = self.state.selection.as_mut()
        {
            sel.active = new_pos;
        }
    }

    /// Convert a window pixel position to an absolute `ScreenPos`.
    /// Returns `None` if the terminal has no rows or no valid cell metrics.
    pub(super) fn mouse_position_to_screen_pos(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<ScreenPos> {
        let rows = self.session.rows() as usize;
        if rows == 0 {
            return None;
        }

        let local_pos = self.mouse_position_to_local(position);
        let layout = self.cell_layout(window)?;
        let x = f32::from(local_pos.x);
        let y = f32::from(local_pos.y);
        let mut row_index = (y / layout.line_height).floor() as i32;
        if row_index < 0 {
            row_index = 0;
        }
        if row_index >= rows as i32 {
            row_index = rows as i32 - 1;
        }
        let row_index = row_index as usize;

        let vp_offset = self.session.viewport_row_offset();
        let screen_row = vp_offset + row_index as u32;

        let content_rows = self.state.viewport_lines.len();
        if row_index >= content_rows {
            // Click past all content rows — clamp to one-past-last-content.
            let last_len = self
                .state
                .viewport_lines
                .last()
                .map(|l| l.len() + 1)
                .unwrap_or(0);
            let last_row = vp_offset + content_rows.saturating_sub(1) as u32;
            return Some(ScreenPos::viewport(last_row, last_len));
        }

        let line_text = self
            .state
            .viewport_lines
            .get(row_index)
            .map(|s| s.as_str())
            .unwrap_or("");

        let byte = if let Some(Some(line)) = self.line_layouts.get(row_index) {
            let b = line.closest_index_for_x(px(x)).min(line.text.len());
            if b >= line.text.len() {
                let text_px_width = f32::from(line.x_for_index(line.text.len()));
                if x > text_px_width {
                    line_text.len() + 1
                } else {
                    b
                }
            } else {
                b
            }
        } else {
            let col_index = (x / layout.cell_width).floor() as i32 + 1;
            let col = col_index.max(1).min(self.session.cols() as i32) as u16;
            let b = super::text_metrics::byte_index_for_column_in_line(line_text, col)
                .min(line_text.len());
            if b >= line_text.len() {
                use unicode_width::UnicodeWidthStr as _;
                let display_cols = line_text.width();
                if col as usize > display_cols {
                    line_text.len() + 1
                } else {
                    b
                }
            } else {
                b
            }
        };

        // When the row lies in scrollback (covered by `LineBuffer`),
        // promote the anchor to a width-invariant `LineBufferPosition`
        // so the selection survives a subsequent resize. Viewport-resident
        // rows keep the byte-offset storage; they get re-clamped after a
        // resize via the normal `refresh_viewport_preserving_selection`
        // path.
        let cell_cols = self.session.cols();
        let lb_rows = self.session.line_buffer().wrapped_row_count(cell_cols);
        if screen_row < lb_rows {
            // Translate the row-local byte offset back to a 1-indexed
            // cell column so `anchor_at` can derive the cumulative
            // cell column within the logical line.
            let cell_col = super::text_metrics::column_for_byte_in_line(line_text, byte);
            return Some(ScreenPos::anchor_at(&self.session, screen_row, cell_col));
        }
        Some(ScreenPos::viewport(screen_row, byte))
    }

    /// URL at the given absolute screen position (viewport rows only).
    pub(super) fn url_at_screen_pos(&self, pos: ScreenPos) -> Option<String> {
        let (screen_row, byte) = pos.resolve(&self.session)?;
        let vp_offset = self.session.viewport_row_offset();
        let vp_row = screen_row.checked_sub(vp_offset)? as usize;
        let line = self.state.viewport_lines.get(vp_row)?;
        super::url::url_at_byte_index(line, byte.min(line.len().saturating_sub(1)))
    }

    pub(super) fn word_range_at(&self, pos: ScreenPos, vp_offset: u32) -> (ScreenPos, ScreenPos) {
        super::viewport::word_range_in_viewport(
            pos,
            &self.session,
            &self.state.viewport_lines,
            vp_offset,
        )
    }

    pub(super) fn line_range_at(&self, pos: ScreenPos, vp_offset: u32) -> (ScreenPos, ScreenPos) {
        super::viewport::line_range_in_viewport(
            pos,
            &self.session,
            &self.state.viewport_lines,
            vp_offset,
        )
    }

    pub(super) fn mouse_position_to_cell(
        &self,
        position: gpui::Point<Pixels>,
        window: &mut Window,
    ) -> Option<(u16, u16)> {
        let cols = self.session.cols();
        let rows = self.session.rows();

        let position = self.mouse_position_to_local(position);
        let layout = self.cell_layout(window)?;
        let x = f32::from(position.x);
        let y = f32::from(position.y);

        let mut col = (x / layout.cell_width).floor() as i32 + 1;
        let mut row = (y / layout.line_height).floor() as i32 + 1;

        if col < 1 {
            col = 1;
        }
        if row < 1 {
            row = 1;
        }
        if col > cols as i32 {
            col = cols as i32;
        }
        if row > rows as i32 {
            row = rows as i32;
        }

        Some((col as u16, row as u16))
    }

    pub(super) fn mouse_position_to_local(
        &self,
        position: gpui::Point<Pixels>,
    ) -> gpui::Point<Pixels> {
        window_position_to_local(self.state.last_bounds, position)
    }
}
