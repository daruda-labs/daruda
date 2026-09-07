//! Picker row — the visible half of a "search and pick" overlay: one
//! selectable row's chrome, plus the row that stands in for an empty
//! result list. The invisible half is
//! `crate::workspace::command::picker::PickerState`.
//!
//! Four things are owned here rather than left to the call site,
//! because applying half of any of them is a visible bug: the
//! focus-accent reservation, the label truncation, the trailing
//! column's refusal to shrink, and the row's height — which
//! `theme::PALETTE_MAX_HEIGHT` is derived from, so a row that measured
//! itself would let the list clip a row the keyboard can still reach.

use gpui::{
    AnyElement, App, IntoElement, MouseButton, MouseDownEvent, SharedString, Window, div,
    prelude::*, px,
};

use crate::ui::theme;

/// One selectable row. `trailing` is the optional right-hand column —
/// the command palette's shortcut, the flow picker's tag.
pub fn picker_row(
    is_focused: bool,
    label: SharedString,
    trailing: Option<AnyElement>,
    on_pick: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            on_pick(window, cx);
        })
        .hover(|d| d.bg(t.palette_focused_bg))
        .flex()
        .flex_row()
        // The row is exactly `theme::PALETTE_ROW_H` tall, and
        // `PALETTE_MAX_HEIGHT` is that times `PALETTE_MAX_VISIBLE`. An
        // explicit line box is what makes that a fact; `flex_none` is
        // what stops the list's own `max_h` shrinking it back.
        .flex_none()
        .line_height(px(theme::PALETTE_ROW_LINE_H))
        .items_center()
        .w_full()
        .px(px(theme::PALETTE_ENTRY_PAD_X))
        .py(px(theme::PALETTE_ENTRY_PAD_Y))
        .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
        // Reserve the same-width transparent border on unfocused rows so
        // the label does not shift when the accent rule appears — same
        // idiom as the lane rows in the left dock.
        .border_l(px(theme::PALETTE_FOCUS_BORDER_W))
        .border_color(theme::TRANSPARENT)
        .when(is_focused, |d| {
            d.bg(t.palette_focused_bg)
                .text_color(t.text_primary)
                .border_color(theme::PRIMARY)
        })
        .when(!is_focused, |d| d.text_color(t.text_body))
        // `min_w_0` is what lets the slot shrink below its content width;
        // without it the row lays out at max-content, overflows the popup,
        // and `truncate` never gets a narrower box to ellipsize into — the
        // label is hard-clipped mid-glyph instead.
        .child(div().flex_1().min_w_0().truncate().child(label))
        // `flex_none` because the label slot is `flex_1`: without it
        // taffy's default `flex_shrink: 1.0` squeezes the trailing column
        // instead of ellipsizing the label it sits beside.
        .children(trailing.map(|el| div().flex_none().child(el)))
}

/// Stand-in row for "nothing matched". Taller vertical padding than a
/// real row so a one-line message does not read as a selectable entry.
pub fn picker_empty(message: SharedString, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .px(px(theme::PALETTE_ENTRY_PAD_X))
        .py(px(theme::PALETTE_EMPTY_PAD_Y))
        // The same transparent accent reservation every row carries, so
        // the message starts on the label's left edge instead of jumping
        // 2px left as a query narrows to nothing.
        .border_l(px(theme::PALETTE_FOCUS_BORDER_W))
        .border_color(theme::TRANSPARENT)
        .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
        .text_color(t.text_subtle)
        .child(message)
}
