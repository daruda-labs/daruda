use gpui::{Context, Window};

use super::TerminalView;
use super::state::PendingRefresh;

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
        let previous = self.state.focused_prompt;
        self.state.focused_prompt =
            self.jump_to_mark(PromptMarkKind::PromptStart, previous, forward, window, cx);
    }

    pub fn jump_to_command(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        use crate::session::PromptMarkKind;
        let previous = self.state.focused_command;
        self.state.focused_command = self.jump_to_mark(
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
        previous_seq: Option<u64>,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let viewport_top = self.session.viewport_row_offset();
        // Translate the stored focus identity (`PromptMark::seq`) into a
        // current-frame screen row so [`next_prompt_index`] — which works
        // in screen-row space — can step relative to it. A focused mark
        // that has been wiped (`\x1b[3J` mirror in
        // `clear_line_buffer_and_drop_history_marks`) or evicted from
        // `LineBuffer` collapses to `None`, triggering the fresh-anchor
        // fallback inside `next_prompt_index`.
        let previous_row = previous_seq.and_then(|seq| {
            self.session
                .prompt_marks()
                .iter()
                .find(|m| m.kind == kind && m.seq == seq)
                .and_then(|m| self.session.abs_to_screen_row(m.abs_y))
        });
        // Pair each candidate's current screen row with its identity so
        // the picker's chosen row can be mapped back to a `seq` for
        // storage. Marks whose row has been evicted from `LineBuffer`
        // drop out here — the picker only walks rows that are still
        // reachable.
        let candidates: Vec<(u32, u64)> = self
            .session
            .prompt_marks()
            .iter()
            .filter(|m| m.kind == kind)
            .filter_map(|m| {
                self.session
                    .abs_to_screen_row(m.abs_y)
                    .map(|row| (row, m.seq))
            })
            .collect();
        // `next_prompt_index` expects ascending order. Capture order
        // tracks `abs_y`, which is monotonic, and `abs_to_screen_row` is
        // order-preserving (subtracts a fixed `overflow`), so candidates
        // are already sorted by construction. Assert the invariant in
        // debug instead of paying an O(n log n) sort in release — if a
        // future change introduces a non-monotonic path, tests will
        // panic here loudly.
        debug_assert!(
            candidates.windows(2).all(|w| w[0].0 <= w[1].0),
            "candidates must be sorted by screen row (abs_y monotonic + abs_to_screen_row order-preserving)",
        );
        let rows: Vec<u32> = candidates.iter().map(|&(row, _)| row).collect();

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
        // A prompt/command jump only shifts the viewport window; the
        // selection's absolute ScreenPos anchors stay valid, so preserve it.
        self.state.pending_refresh = PendingRefresh::Preserve;
        cx.notify();
        // Map the chosen screen row back to its mark identity. Screen
        // rows are unique per mark within a frame (`abs_to_screen_row`
        // is injective on live `abs_y`), so this lookup is unambiguous.
        candidates
            .into_iter()
            .find(|&(row, _)| row == jump.row)
            .map(|(_, seq)| seq)
    }

    fn schedule_prompt_jump_flash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let flash = crate::ux::strings::PROMPT_JUMP_FLASH;
        self.state.flash.prompt_jump = Some(std::time::Instant::now() + flash);
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
