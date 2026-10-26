//! Scroll-wheel handling: mouse reporting, alternate scroll, and viewport
//! scrolling.

use super::*;

impl TerminalView {
    pub(in crate::view) fn on_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A new physical gesture clears the momentum-suppression window so
        // intentional scrolling after PTY input locks the viewport normally.
        if matches!(event.touch_phase, TouchPhase::Started) {
            self.state.suppress_scroll_lock_until = None;
        }

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

        if self.input.is_some() && !event.modifiers.shift && self.session.mouse_reporting_enabled()
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
            self.send_mouse_report(MouseReport::Press(button_value), col, row, steps);
            return;
        }

        let offset_before = self.session.viewport_row_offset();
        let _ = self.session.scroll_viewport(delta_lines);

        if self.session.viewport_row_offset() != offset_before {
            self.sync_viewport_scroll_tracking();
            self.apply_side_effects(cx);
            // Suppress re-locking for pixel-delta (trackpad) events that
            // arrive during a momentum burst following PTY input: the input
            // snapped the viewport to the bottom and inertia must not
            // immediately re-pin it there.  Lines-delta (physical wheel)
            // and new gestures (TouchPhase::Started clears the flag) are
            // never suppressed.
            let is_momentum = matches!(event.delta, ScrollDelta::Pixels(_))
                && self
                    .state
                    .suppress_scroll_lock_until
                    .is_some_and(|until| std::time::Instant::now() < until);
            if is_momentum {
                self.schedule_viewport_refresh(cx);
            } else {
                self.reanchor_and_refresh(cx);
            }
        }

        // While the left button is held (a text-selection drag), extend the
        // selection active endpoint to the row now under the cursor so scrolling
        // during a drag extends the selection — matching iTerm2 / Alacritty.
        if matches!(self.state.mouse_drag, MouseDragState::TextSelection { .. })
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
}
