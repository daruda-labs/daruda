//! What a draggable thumb scrolls.
//!
//! The thumb maps the pointer to an absolute offset; each scroll primitive
//! applies it its own way (a list re-anchors on an item, an editor clamps and
//! repaints itself), so the thumb stays independent of the handle type.

use gpui::{App, Entity, ListState, Pixels, ScrollHandle, UniformListScrollHandle};

use super::Axis;
use crate::InputState;

/// A region a scrollbar thumb can move.
pub trait ThumbTarget: 'static {
    /// Move the region along `axis` to `offset` (`-scrollable..=0`).
    fn scroll_to(&self, axis: Axis, offset: Pixels, cx: &mut App);
    /// The pointer grabbed the thumb.
    fn drag_started(&self) {}
    /// The pointer let go of the thumb.
    fn drag_ended(&self) {}
}

impl ThumbTarget for ScrollHandle {
    fn scroll_to(&self, axis: Axis, offset: Pixels, _: &mut App) {
        self.set_offset(axis.replace(self.offset(), offset));
    }
}

impl ThumbTarget for UniformListScrollHandle {
    fn scroll_to(&self, axis: Axis, offset: Pixels, _: &mut App) {
        let state = self.0.borrow();
        let current = state.base_handle.offset();
        state.base_handle.set_offset(axis.replace(current, offset));
    }
}

impl ThumbTarget for ListState {
    fn scroll_to(&self, axis: Axis, offset: Pixels, _: &mut App) {
        let current = self.scroll_px_offset_for_scrollbar();
        self.set_offset_from_scrollbar(axis.replace(current, offset));
    }

    // The list freezes its content height for the drag, so items measured
    // mid-drag do not pull the thumb out from under the pointer.
    fn drag_started(&self) {
        self.scrollbar_drag_started();
    }

    fn drag_ended(&self) {
        self.scrollbar_drag_ended();
    }
}

impl ThumbTarget for Entity<InputState> {
    fn scroll_to(&self, axis: Axis, offset: Pixels, cx: &mut App) {
        // `set_scroll_offset` owns the clamp and the repaint; writing the
        // editor's scroll handle directly would skip both.
        self.update(cx, |state, cx| {
            let current = state.scroll_handle().offset();
            state.set_scroll_offset(axis.replace(current, offset), cx);
        });
    }
}
