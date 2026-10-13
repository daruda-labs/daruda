use gpui::{
    Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollDelta, ScrollWheelEvent, TouchPhase, Window, point, px,
};

use super::selection::ScreenPos;
use super::state::MouseDragState;
use super::{ByteSelection, TerminalView};
use gpui::ClipboardItem;

mod hit_test;
mod motion;
mod scroll;

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
    inset_x: f32,
    inset_y: f32,
) -> gpui::Point<Pixels> {
    let origin = last_bounds
        .map(|bounds| bounds.origin)
        .unwrap_or_else(|| point(px(0.0), px(0.0)));
    // Subtract the pane inset alongside the element origin so pixel↔cell
    // math lines up with the inset paint origin. A click inside the
    // left/top inset margin would go negative; clamp to 0 so it maps to
    // the first cell instead of underflowing the grid column/row.
    point(
        (position.x - origin.x - px(inset_x)).max(px(0.0)),
        (position.y - origin.y - px(inset_y)).max(px(0.0)),
    )
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
        self.state.mouse_drag = MouseDragState::None;

        if event.first_mouse {
            return;
        }

        // Scrollbar thumb drag — must be checked before the general
        // selection path so a click on the thumb doesn't start a
        // text selection.
        if event.button == MouseButton::Left
            && let Some(thumb) = self.state.scrollbar_thumb_bounds
        {
            if thumb.contains(&event.position) {
                let bounds = self.state.last_bounds.unwrap_or(thumb);
                // Store the cursor's offset from the thumb top so the
                // thumb doesn't jump on the first move event.
                let cursor_y = f32::from(event.position.y) - f32::from(bounds.origin.y);
                let thumb_top = f32::from(thumb.origin.y) - f32::from(bounds.origin.y);
                let click_offset = cursor_y - thumb_top;
                self.state.mouse_drag = MouseDragState::ScrollbarDrag {
                    offset: click_offset,
                };
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
                    // Same settle path as the wheel: reanchor (unlock at
                    // the live edge) + selection-preserving repaint. Using
                    // `refresh_viewport` here would clear the selection and
                    // strand the hovered-URL underline, unlike every other
                    // user-scroll path.
                    self.scroll_viewport_and_sync(delta);
                    self.reanchor_and_refresh(cx);
                }
                return;
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
            if let Some((col, row)) = self.mouse_position_to_cell(event.position, window)
                && let Some(link) = self.link_at_cell(col, row)
                && link.openable
            {
                cx.open_url(&link.url);
                cx.write_to_clipboard(ClipboardItem::new_string(link.url));
                return;
            }

            if let Some(pos) = self.mouse_position_to_screen_pos(event.position, window)
                && let Some(url) = self.url_at_screen_pos(pos)
            {
                cx.open_url(&url);
                cx.write_to_clipboard(ClipboardItem::new_string(url));
                return;
            }
        }

        // A plain right-click opens the host context menu when mouse
        // reporting is off (a normal shell prompt) or there is no PTY input
        // sender. Shift stays the unconditional escape hatch — the
        // iTerm2/kitty/Alacritty/zed convention — so the menu is still
        // reachable while an app (vim, tmux) holds SGR mouse capture.
        // Subscribers consume the emitted event to build the actual menu,
        // which the terminal crate does not own.
        if event.button == MouseButton::Right
            && (event.modifiers.shift
                || self.input.is_none()
                || !self.session.mouse_reporting_enabled())
        {
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
        let was_scrollbar = matches!(self.state.mouse_drag, MouseDragState::ScrollbarDrag { .. });
        self.end_mouse_drag();
        if event.button == MouseButton::Left && was_scrollbar {
            cx.notify();
            return;
        }
        if event.modifiers.shift || self.input.is_none() || !self.session.mouse_reporting_enabled()
        {
            if let Some(selection) = self.state.selection {
                if selection.is_empty() {
                    self.state.selection = None;
                } else if event.button == MouseButton::Left {
                    self.copy_settled_selection(cx);
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
}
