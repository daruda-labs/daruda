//! Drag-selection autoscroll for the agent-chat pane.
//!
//! While the user drag-selects text inside one conversation block (a
//! `crate::ui::markdown` / `selectable_text` → gpui_component `TextView`) and
//! the cursor leaves the pane vertically, this polls `window.mouse_position()`
//! on a timer and scrolls the virtualized `list` so the selection extends into
//! the off-screen part of *that* block. Text selection is confined to a single
//! block, so scrolling is clamped at the selected block's top/bottom edges.
//!
//! Mirrors the terminal's `start_autoscroll` / `autoscroll_poll_with_pos`
//! (`daruda_terminal/src/view/mouse.rs`): a 50 ms `background_executor().timer`
//! loop that reads `window.mouse_position()` (works even when the cursor is
//! outside the window frame *and* held stationary — why polling beats
//! mouse-move events), scrolls, then re-extends the selection endpoint.
//!
//! The pure per-tick step calculation lives in [`autoscroll_step`], unit-tested
//! with plain numbers; the polling loop / real mouse position / real scroll are
//! GPUI-runtime and stay untested (same split the terminal uses).

use std::time::Duration;

use gpui::{Context, MouseButton, MouseMoveEvent, Window, px};

use super::view::AgentChatView;
use crate::ui::theme;

/// One auto-scroll polling step, in pixels. Returns the distance to scroll the
/// list this tick (positive = toward the end / content up, negative = toward
/// the start / content down, `0` = no scroll). Kept a free function of plain
/// numbers so it is trivially unit-testable without any GPUI state.
///
/// - Inside the viewport (`list_top..=list_bottom`) → `0`.
/// - Below the bottom edge → scroll down, magnitude proportional to the
///   overshoot (rounded up to whole `step_granularity_px` steps), clamped to
///   `max_step_px`.
/// - Above the top edge → the same, negated.
/// - Boundary clamp: the selected block cannot be scrolled past its own edges,
///   so if the block's end is already visible we don't scroll further down, and
///   if the block's start is already visible we don't scroll further up.
///
/// `step_granularity_px` is the velocity-stepping granularity — the overshoot
/// is rounded up to whole multiples of it. It is the agent-chat font size (not a
/// true text line height), so a slightly-outside cursor advances by one glyph-row
/// increment per tick rather than a single pixel.
pub(in crate::workspace) fn autoscroll_step(
    mouse_y: f32,
    list_top: f32,
    list_bottom: f32,
    step_granularity_px: f32,
    block_top: f32,
    block_bottom: f32,
    max_step_px: f32,
) -> i32 {
    // Guard against a zero / negative granularity (div-by-zero) and a negative
    // cap. A degenerate cap disables scrolling entirely.
    let step_granularity_px = step_granularity_px.max(1.0);
    let max_step_px = max_step_px.max(0.0);

    if mouse_y > list_bottom {
        // Scroll toward the block's tail (reveal content below the viewport).
        if block_bottom <= list_bottom {
            return 0; // block end already fully revealed — nothing more below
        }
        magnitude(mouse_y - list_bottom, step_granularity_px, max_step_px)
    } else if mouse_y < list_top {
        // Scroll toward the block's head (reveal content above the viewport).
        if block_top >= list_top {
            return 0; // block start already fully revealed — nothing more above
        }
        -magnitude(list_top - mouse_y, step_granularity_px, max_step_px)
    } else {
        0 // cursor inside the viewport
    }
}

/// Overshoot distance (px, `>= 0`) → per-tick scroll magnitude (px, `>= 0`):
/// round the overshoot up to whole `step_granularity_px` steps, then clamp to
/// `max_step_px`.
fn magnitude(overshoot: f32, step_granularity_px: f32, max_step_px: f32) -> i32 {
    let stepped = (overshoot / step_granularity_px).ceil() * step_granularity_px;
    stepped.min(max_step_px).round() as i32
}

impl AgentChatView {
    /// Spawn the drag-selection autoscroll poll. Called on every left
    /// mouse-down inside the list container; the task self-terminates on the
    /// first tick where no block is mid-drag (mouse-up cleared the selection),
    /// so a plain click just spawns a one-tick no-op. Replace-and-cancel: any
    /// prior (completed or stale) task is dropped here, before the next drag
    /// begins — only one drag runs at a time, so this is never dropping the
    /// currently-executing task from within itself.
    ///
    /// `step_granularity_px` is captured now (while `window`/`cx` are available)
    /// so the async task uses the agent-chat font metric at drag-start rather
    /// than a value that could shift mid-drag.
    pub(in crate::workspace) fn start_selection_autoscroll(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // App-owned drag lifetime: set on the always-painted list container's
        // mouse-down so loop termination never depends on the selected child
        // block staying painted (see `end_selection_drag`).
        self.selection_drag_active = true;
        let entity = cx.entity().downgrade();
        let step_granularity_px = theme::agent_chat_font_size(cx).max(1.0);
        let poll = Duration::from_millis(theme::AGENT_CHAT_AUTOSCROLL_POLL_MS);
        self.autoscroll_task = Some(window.spawn(cx, async move |cx| {
            loop {
                cx.background_executor().timer(poll).await;
                let keep_going = cx
                    .update(|window, cx| {
                        let mouse = window.mouse_position();
                        entity
                            .upgrade()
                            .map(|e| {
                                e.update(cx, |v, cx| {
                                    v.autoscroll_tick(mouse, step_granularity_px, cx)
                                })
                            })
                            .unwrap_or(false)
                    })
                    // SILENT-OK: drag ended by window/entity teardown — the loop
                    // exits, which is exactly the intended terminal outcome.
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    /// End the drag-selection: clear the app-owned drag signal and drop the
    /// poll task. The single end point for the drag lifetime (parallels the
    /// terminal's `end_mouse_drag`), so any future drag-state addition can't be
    /// silently missed by one of the two release paths. Invoked from the
    /// always-painted list container's mouse-up (in-bounds release) and its
    /// mouse-move (release-outside-window re-entry — see `on_selection_drag_move`).
    pub(in crate::workspace) fn end_selection_drag(&mut self) {
        self.selection_drag_active = false;
        self.autoscroll_task = None;
    }

    /// List-container mouse-move handler. Mirrors the terminal's `on_mouse_move`
    /// re-entry check (`mouse.rs`): if a drag is still marked active but the left
    /// button is no longer held, the button was released off-window where the
    /// container's `on_mouse_up` never fires (`hitbox.is_hovered` is false out of
    /// bounds). Treat re-entry without the button as an implicit mouse-up and end
    /// the drag, so the poll terminates even for an off-window release.
    pub(in crate::workspace) fn on_selection_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _cx: &mut Context<Self>,
    ) {
        if self.selection_drag_active && event.pressed_button != Some(MouseButton::Left) {
            self.end_selection_drag();
        }
    }

    /// One poll tick. Returns `false` to stop the loop, `true` to keep polling.
    /// Scrolls the list and extends the live selection while the cursor is
    /// outside the viewport and the selected block has off-screen text in that
    /// direction.
    fn autoscroll_tick(
        &mut self,
        mouse: gpui::Point<gpui::Pixels>,
        step_granularity_px: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        // Primary termination authority: the app-owned drag signal, cleared by
        // `end_selection_drag` on mouse release. Independent of the selected
        // block's paint lifetime, so an unmounted-mid-drag block can't strand
        // the loop the way a stale `active_text_selection` slot would.
        if !self.selection_drag_active {
            return false;
        }
        // Secondary early-exit: no live selection to extend.
        let Some(handle) = crate::ui::active_text_selection(cx) else {
            return false;
        };
        // Bounds not captured yet (list not painted) → keep polling, no-op.
        let Some(lb) = self.list_bounds else {
            return true;
        };
        let block = handle.block_bounds(cx);
        // The active selection belongs to a block in a different pane — this
        // pane must not scroll for it. Keep polling in case focus returns.
        if !block.intersects(&lb) {
            return true;
        }
        let mouse_y = f32::from(mouse.y);
        let list_top = f32::from(lb.top());
        let list_bottom = f32::from(lb.bottom());
        let step = autoscroll_step(
            mouse_y,
            list_top,
            list_bottom,
            step_granularity_px,
            f32::from(block.top()),
            f32::from(block.bottom()),
            theme::AGENT_CHAT_AUTOSCROLL_MAX_STEP_PX,
        );
        if step != 0 {
            self.list_state.scroll_by(px(step as f32));
        }
        // Extend whenever the cursor is beyond the viewport edge — including the
        // settle tick where the boundary clamp made `step == 0` (block edge just
        // revealed). `scroll_by` moved the offset this frame but the block's
        // `TextViewState.bounds` only refresh on the next paint, so extending
        // against the just-refreshed bounds each tick lets the selection catch up
        // to (and finally reach) the true dragged edge instead of settling up to
        // `max_step` short. Inside the viewport the block's own move handler owns
        // extension, so we don't touch it there.
        let cursor_outside = mouse_y > list_bottom || mouse_y < list_top;
        if cursor_outside {
            handle.extend_to(mouse, cx);
        }
        if step != 0 || cursor_outside {
            cx.notify();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed viewport / block geometry for the step tests. Viewport spans
    // y = 100..200; the selected block spans y = 50..300 (extends both above
    // and below the viewport), so neither boundary clamp fires unless asked.
    const TOP: f32 = 100.0;
    const BOTTOM: f32 = 200.0;
    const BLOCK_TOP: f32 = 50.0;
    const BLOCK_BOTTOM: f32 = 300.0;
    const STEP_GRANULARITY: f32 = 16.0;
    const MAX: f32 = 48.0;

    fn step(mouse_y: f32, block_top: f32, block_bottom: f32) -> i32 {
        autoscroll_step(
            mouse_y,
            TOP,
            BOTTOM,
            STEP_GRANULARITY,
            block_top,
            block_bottom,
            MAX,
        )
    }

    #[test]
    fn inside_viewport_does_not_scroll() {
        assert_eq!(step(TOP, BLOCK_TOP, BLOCK_BOTTOM), 0);
        assert_eq!(step(150.0, BLOCK_TOP, BLOCK_BOTTOM), 0);
        assert_eq!(step(BOTTOM, BLOCK_TOP, BLOCK_BOTTOM), 0);
    }

    #[test]
    fn below_bottom_scrolls_down_positive() {
        // 8px past the bottom → rounds up to one line (16px).
        assert_eq!(step(BOTTOM + 8.0, BLOCK_TOP, BLOCK_BOTTOM), 16);
        // Exactly one line past → still one line.
        assert_eq!(step(BOTTOM + 16.0, BLOCK_TOP, BLOCK_BOTTOM), 16);
        // Just over one line past → two lines (32px).
        assert_eq!(step(BOTTOM + 17.0, BLOCK_TOP, BLOCK_BOTTOM), 32);
    }

    #[test]
    fn above_top_scrolls_up_negative() {
        assert_eq!(step(TOP - 8.0, BLOCK_TOP, BLOCK_BOTTOM), -16);
        assert_eq!(step(TOP - 17.0, BLOCK_TOP, BLOCK_BOTTOM), -32);
    }

    #[test]
    fn magnitude_is_capped_at_max_step() {
        // Far past the bottom → clamped to MAX (48px), not the raw overshoot.
        assert_eq!(step(BOTTOM + 1000.0, BLOCK_TOP, BLOCK_BOTTOM), 48);
        assert_eq!(step(TOP - 1000.0, BLOCK_TOP, BLOCK_BOTTOM), -48);
    }

    #[test]
    fn clamps_at_block_bottom_when_end_visible() {
        // Cursor below the viewport, but the block ends inside the viewport
        // (nothing more of this block below) → no scroll.
        let block_bottom_visible = BOTTOM - 10.0;
        assert_eq!(step(BOTTOM + 40.0, BLOCK_TOP, block_bottom_visible), 0);
        // Block ending exactly at the bottom edge is also fully revealed.
        assert_eq!(step(BOTTOM + 40.0, BLOCK_TOP, BOTTOM), 0);
    }

    #[test]
    fn clamps_at_block_top_when_start_visible() {
        // Cursor above the viewport, but the block starts inside the viewport
        // (nothing more of this block above) → no scroll.
        let block_top_visible = TOP + 10.0;
        assert_eq!(step(TOP - 40.0, block_top_visible, BLOCK_BOTTOM), 0);
        // Block starting exactly at the top edge is also fully revealed.
        assert_eq!(step(TOP - 40.0, TOP, BLOCK_BOTTOM), 0);
    }

    #[test]
    fn block_extending_past_edge_still_scrolls() {
        // Block bottom just past the viewport bottom → still has content below,
        // so a below-bottom cursor scrolls.
        assert!(step(BOTTOM + 40.0, BLOCK_TOP, BOTTOM + 1.0) > 0);
        // Block top just above the viewport top → still has content above.
        assert!(step(TOP - 40.0, TOP - 1.0, BLOCK_BOTTOM) < 0);
    }

    #[test]
    fn zero_step_granularity_does_not_panic() {
        // Degenerate granularity is floored to 1.0 — no div-by-zero / NaN.
        let s = autoscroll_step(
            BOTTOM + 40.0,
            TOP,
            BOTTOM,
            0.0,
            BLOCK_TOP,
            BLOCK_BOTTOM,
            MAX,
        );
        assert!(s > 0 && s <= MAX as i32);
    }
}
