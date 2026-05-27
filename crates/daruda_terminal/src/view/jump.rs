use gpui::{Context, Window};

use super::TerminalView;

/// Resolved target of a `Cmd+Shift+Up/Down` press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PromptJump {
    pub(crate) row: u32,
    pub(crate) wrapped: bool,
}

/// Compute the next prompt mark to focus when Cmd+Shift+Up/Down is
/// pressed. Accepts the previously-focused mark as an absolute
/// screen row so the cursor survives mark eviction.
pub(crate) fn next_prompt_index(
    sorted_starts: &[u32],
    previous_row: Option<u32>,
    viewport_top: u32,
    forward: bool,
) -> Option<PromptJump> {
    if sorted_starts.is_empty() {
        return None;
    }
    let len = sorted_starts.len();
    if let Some(prev_row) = previous_row
        && let Some(prev) = sorted_starts.iter().position(|&r| r == prev_row)
    {
        let (idx, wrapped) = if forward {
            let next = (prev + 1) % len;
            (next, next == 0 && prev == len - 1)
        } else {
            let next = (prev + len - 1) % len;
            (next, prev == 0 && next == len - 1)
        };
        return Some(PromptJump {
            row: sorted_starts[idx],
            wrapped,
        });
    }
    if forward {
        match sorted_starts.iter().position(|&r| r >= viewport_top) {
            Some(idx) => Some(PromptJump {
                row: sorted_starts[idx],
                wrapped: false,
            }),
            None => Some(PromptJump {
                row: sorted_starts[0],
                wrapped: true,
            }),
        }
    } else {
        match sorted_starts.iter().rposition(|&r| r < viewport_top) {
            Some(idx) => Some(PromptJump {
                row: sorted_starts[idx],
                wrapped: false,
            }),
            None => Some(PromptJump {
                row: sorted_starts[len - 1],
                wrapped: true,
            }),
        }
    }
}

impl TerminalView {
    pub(super) fn scroll_to_screen_row(&mut self, target_row: u32) {
        let viewport_top = self.session.viewport_row_offset();
        let viewport_rows = self.session.rows() as u32;
        if target_row >= viewport_top && target_row < viewport_top + viewport_rows {
            return;
        }
        let desired_top = target_row.saturating_sub(viewport_rows / 2);
        self.scroll_viewport_to_top(desired_top);
    }

    pub(super) fn scroll_screen_row_to_top(&mut self, target_row: u32) {
        self.scroll_viewport_to_top(target_row);
    }

    fn scroll_viewport_to_top(&mut self, desired_top: u32) {
        let viewport_top = self.session.viewport_row_offset();
        let delta = desired_top as i64 - viewport_top as i64;
        if delta == 0 {
            return;
        }
        let clamped = delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let _ = self.session.scroll_viewport(clamped);
        self.sync_viewport_scroll_tracking();
    }

    pub fn jump_to_prompt(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::session::PromptMarkKind;
        let previous = self.state.focused_prompt_row;
        self.state.focused_prompt_row =
            self.jump_to_mark(PromptMarkKind::PromptStart, previous, forward, window, cx);
    }

    pub fn jump_to_command(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::session::PromptMarkKind;
        let previous = self.state.focused_command_row;
        self.state.focused_command_row = self.jump_to_mark(
            PromptMarkKind::CommandExecuted,
            previous,
            forward,
            window,
            cx,
        );
    }

    fn jump_to_mark(
        &mut self,
        kind: crate::session::PromptMarkKind,
        previous_row: Option<u32>,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u32> {
        let viewport_top = self.session.viewport_row_offset();
        // Translate each mark's abs_y to a current-frame screen row.
        // Marks whose row has been evicted from `LineBuffer` drop out
        // here — the picker only walks rows that are still reachable.
        let mut rows: Vec<u32> = self
            .session
            .prompt_marks()
            .iter()
            .filter(|m| m.kind == kind)
            .filter_map(|m| self.session.abs_to_screen_row(m.abs_y))
            .collect();
        // `next_prompt_index` expects ascending order. Capture order
        // tracks `abs_y`, which is monotonic, so the translated rows
        // are already sorted — but sort defensively in case a future
        // change introduces a non-monotonic path.
        rows.sort_unstable();

        let jump = next_prompt_index(&rows, previous_row, viewport_top, forward)?;
        match self.session.prompt_jump_scroll_mode() {
            crate::config::PromptJumpScroll::AlwaysTop => {
                self.scroll_screen_row_to_top(jump.row);
            }
            crate::config::PromptJumpScroll::LeaveInPlace => {
                self.scroll_to_screen_row(jump.row);
            }
        }
        // Lock the viewport so PTY output doesn't snap back to the bottom
        // while the user is reading the jumped-to prompt or command block.
        self.state
            .viewport_lock
            .lock(self.session.viewport_top_abs_y());
        if jump.wrapped {
            self.schedule_prompt_jump_flash(window, cx);
        }
        self.state.pending_refresh = true;
        cx.notify();
        Some(jump.row)
    }

    fn schedule_prompt_jump_flash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let flash = crate::ux::strings::PROMPT_JUMP_FLASH;
        self.state.prompt_jump_flash_until = Some(std::time::Instant::now() + flash);
        let entity = cx.entity().downgrade();
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(flash).await;
                cx.update(|_, cx| {
                    if let Some(view) = entity.upgrade() {
                        view.update(cx, |_, cx| cx.notify());
                    }
                })
                .ok();
            })
            .detach();
    }
}
