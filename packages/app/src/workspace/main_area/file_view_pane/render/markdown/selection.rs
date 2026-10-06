//! Block-level click/drag selection for the Markdown preview.
//!
//! A link and a block selection compete for the same press, so the decision
//! waits: the press is queued here, and either the link consumes it on
//! mouse-up or the block commits it.

use gpui::{App, Context, Global, MouseButton, MouseDownEvent, MouseMoveEvent, prelude::*};

use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::CharSelection;

#[derive(Clone, Copy)]
pub(super) struct PendingBlockSelection {
    block_idx: usize,
    shift: bool,
}

/// Input state that must survive the repaint `InteractiveText` requests on
/// mouse-down. A link consumes the queued block selection on mouse-up.
#[derive(Default)]
struct MarkdownPointerState {
    pressed_button: Option<MouseButton>,
    pending_block_selection: Option<PendingBlockSelection>,
}

impl Global for MarkdownPointerState {}

pub(super) fn record_markdown_mouse_button(button: MouseButton, cx: &mut App) {
    cx.default_global::<MarkdownPointerState>().pressed_button = Some(button);
}

pub(super) fn take_markdown_mouse_button(cx: &mut App) -> Option<MouseButton> {
    cx.default_global::<MarkdownPointerState>()
        .pressed_button
        .take()
}

pub(super) fn queue_block_selection(block_idx: usize, shift: bool, cx: &mut App) {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection = Some(PendingBlockSelection { block_idx, shift });
}

pub(super) fn take_pending_block_selection(cx: &mut App) -> Option<PendingBlockSelection> {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection
        .take()
}

pub(super) fn take_pending_block_selection_for_drag(
    active_block_idx: usize,
    cx: &mut App,
) -> Option<PendingBlockSelection> {
    let pointer = cx.default_global::<MarkdownPointerState>();
    if pointer
        .pending_block_selection
        .is_some_and(|pending| pending.block_idx != active_block_idx)
    {
        pointer.pending_block_selection.take()
    } else {
        None
    }
}

pub(super) fn cancel_pending_block_selection(cx: &mut App) {
    cx.default_global::<MarkdownPointerState>()
        .pending_block_selection = None;
}

/// Returns true when `block_idx` falls within the char-selection row range.
/// Used only by the Markdown preview block-level selection.
pub(super) fn is_block_selected(char_selection: Option<&CharSelection>, block_idx: usize) -> bool {
    let Some(sel) = char_selection else {
        return false;
    };
    let (start, end) = sel.ordered();
    block_idx >= start.row && block_idx <= end.row
}

/// Attach block-level click/drag selection handlers to a Markdown block div.
/// Selection waits until mouse-up or until the pointer enters another block,
/// giving a link in the original block a chance to consume an ordinary click.
pub(super) fn block_with_selection(
    block_div: gpui::Div,
    block_idx: usize,
    cx: &mut Context<Workspace>,
) -> gpui::Div {
    let down_handler = cx.listener(move |_this, ev: &MouseDownEvent, _window, cx| {
        queue_block_selection(block_idx, ev.modifiers.shift, cx);
    });
    let move_handler = cx.listener(move |this, ev: &MouseMoveEvent, _window, cx| {
        let left_pressed = ev.pressed_button == Some(MouseButton::Left);
        let pending = if left_pressed {
            take_pending_block_selection_for_drag(block_idx, cx)
        } else {
            cancel_pending_block_selection(cx);
            None
        };
        if let Some(fv) = this.focused_file_view_mut() {
            let mut changed = false;
            if let Some(pending) = pending {
                fv.handle_block_mouse_down(pending.block_idx, pending.shift);
                changed = true;
            }
            changed |= fv.handle_block_mouse_move(block_idx, left_pressed);
            if changed {
                cx.notify();
            }
        }
    });
    let up_handler = cx.listener(move |this, _ev, _window, cx| {
        let pending = take_pending_block_selection(cx);
        if let Some(fv) = this.focused_file_view_mut() {
            let mut changed = false;
            if let Some(pending) = pending {
                fv.handle_block_mouse_down(pending.block_idx, pending.shift);
                if !pending.shift && pending.block_idx != block_idx {
                    fv.handle_block_mouse_move(block_idx, true);
                }
                changed = true;
            }
            changed |= fv.end_selection_drag();
            if changed {
                cx.notify();
            }
        }
    });
    block_div
        .cursor_default()
        .on_mouse_down(MouseButton::Left, down_handler)
        .on_mouse_up(MouseButton::Left, up_handler)
        .on_mouse_move(move_handler)
}
