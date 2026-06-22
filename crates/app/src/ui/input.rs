//! Text input factory — wrapper over `gpui_component::input`. Works
//! for both single-line (modal default) and multi-line states (e.g.
//! the bottom-dock terminal input): the inner `Input` picks the mode
//! from `InputState.mode`, and the wrapper's `items_stretch` lets a
//! multi-line `h_auto` body grow to the parent cell's height while
//! leaving single-line inputs unchanged (single-line inner carries an
//! explicit `input_h(size)`).
//!
//! daruda inputs sit on `MODAL_INPUT_BG`, not `gpui_component`'s
//! default `theme.background`. Plain `input` keeps the inner `Input`'s
//! own chrome (bg + border + 1px accent focus border) and just
//! overrides the bg via `.bg()`. The `*_with_action` variants host the
//! action in an outer div, so they keep the inner `Input` borderless
//! and draw the bg / border / focus-within accent on that div.
//!
//! Tab participation is the same `XxxTabSpec` polymorphism used by
//! `checkbox` / `radio` / `select`: pass an `isize` to slot the input
//! into the modal's cycle, or `()` to skip it (read-only / display-
//! only inputs). `Input` has no `tab_stop` builder, so the `()` impl
//! mutates the `InputState`'s `FocusHandle` directly — hence the `cx`
//! argument.
//!
//! ```ignore
//! let state = cx.new(|cx| crate::ui::InputState::new(window, cx).placeholder("Name"));
//! parent.child(crate::ui::input(&state, cx, 0));   // cycles at index 0
//! parent.child(crate::ui::input(&state, cx, ())); // mouse-only
//! ```

use crate::ui::theme as d;
use gpui::{
    App, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    Styled as _, div, px,
};
use gpui_component::Sizable as _;

pub use gpui_component::input::{Input, InputEvent, InputState};

/// Tab-participation specifier for [`input`]. Mirrors `CheckboxTabSpec`
/// / `RadioTabSpec` / `SelectTabSpec` — `isize` slots the input at
/// that cycle index, `()` removes it from the cycle.
pub trait InputTabSpec {
    fn apply(self, state: &Entity<InputState>, cx: &App, input: Input) -> Input;
}

impl InputTabSpec for isize {
    fn apply(self, _state: &Entity<InputState>, _cx: &App, input: Input) -> Input {
        input.tab_index(self)
    }
}

impl InputTabSpec for () {
    fn apply(self, state: &Entity<InputState>, cx: &App, input: Input) -> Input {
        // `Input` has no `tab_stop` builder; mutate the underlying
        // `FocusHandle` so the focus map registers this input as
        // non-tab-stoppable.
        let _ = state.read(cx).focus_handle(cx).tab_stop(false);
        input
    }
}

/// Construct a daruda-toned single-line input bound to `state`.
///
/// `tab` decides Tab cycle participation (`isize` cycles at that
/// index, `()` skips). The inner `Input` paints its own chrome; `.bg()`
/// overrides the surface to `MODAL_INPUT_BG`.
///
/// `.w_full()` is baked in: the inner `gpui_component::Input` sizes
/// to its parent via `size_full`, so a content-sized wrapper would
/// collapse the input to zero width inside a `flex_row` parent
/// (where the cross-axis stretch only fills vertically). Callers
/// that need a narrower input can place this inside a sized
/// container (`div().w(px(N))....child(input(...))`).
pub fn input<T: InputTabSpec>(state: &Entity<InputState>, cx: &App, tab: T) -> impl IntoElement {
    // gpui draws its own chrome (bg + hairline + 1px accent focus
    // border); `.bg()` overrides the surface to `modal_input_bg`.
    let inner = Input::new(state)
        .small()
        .bordered(true)
        .w_full()
        .bg(d::current(cx).modal_input_bg);
    tab.apply(state, cx, inner)
}

/// Chrome-cell input variant that hosts an action element inline,
/// right-aligned along the bottom edge of the cell. Same daruda
/// tokens as [`input`] (`MODAL_INPUT_BG` / border / radius) so the
/// two read as the same widget family. Used by the bottom-dock
/// terminal input where the Submit button lives inside the chrome
/// instead of beside it.
///
/// `action` is any [`IntoElement`] — typically `crate::ui::button_*`
/// for a single button, but a `DropdownButton` split or a small
/// `flex_row` of multiple buttons works just as well. Variant /
/// disabled / on_click choices stay at the call site.
///
/// Height is left to the caller — wrap with `div().flex_1().min_h(...)`
/// (or any sized container) when the cell should grow with its
/// parent. The wrapper itself only commits to chrome + layout.
pub fn input_with_action<T: InputTabSpec>(
    state: &Entity<InputState>,
    action: impl IntoElement,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    let inner = Input::new(state).small().appearance(false);
    let inner = tab.apply(state, cx, inner);
    let t = d::current(cx);
    // Inner Input is borderless, so the focus-within accent border
    // lives on this outer div (track_focus + in_focus). The inner Input
    // track_focus'es the same handle; the duplicate collapses to one
    // tab stop by FocusId — don't add a `tab_index` here (real 2nd stop).
    let focus_handle = state.read(cx).focus_handle(cx);
    div()
        .track_focus(&focus_handle)
        .flex()
        .flex_col()
        .w_full()
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.border)
        .in_focus(|s| s.border_color(d::PRIMARY))
        .rounded(px(d::MODAL_BUTTON_RADIUS))
        // Text region fills the top — `flex_1` claims the vertical
        // space above the action bar; the inner flex propagates
        // stretch to the `Input`'s multi-line `h_auto` body.
        .child(div().flex_1().flex().child(inner))
        // Action bar — right-aligned along the bottom.
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .justify_end()
                .items_center()
                .px(px(d::INPUT_PANEL_BUTTON_GAP))
                .pb(px(d::INPUT_PANEL_BUTTON_GAP))
                .child(action),
        )
}

/// Inline (single-row) chrome-cell input variant. Same daruda tokens
/// as [`input`] / [`input_with_action`] (`MODAL_INPUT_BG` / border /
/// radius) so the three read as the same widget family, but the
/// layout is a single horizontal row — the input fills the available
/// width while `action` sits right-aligned beside it on the same line.
/// Used by the bottom-dock terminal input when the dock height is
/// compacted to the 1-row preset, where the vertical layout used by
/// [`input_with_action`] would clip both the text region and the
/// inline Submit button.
///
/// Caller controls width via the surrounding container; the wrapper
/// commits only to chrome + the row layout.
pub fn input_with_action_inline<T: InputTabSpec>(
    state: &Entity<InputState>,
    action: impl IntoElement,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    let inner = Input::new(state).small().appearance(false);
    let inner = tab.apply(state, cx, inner);
    let t = d::current(cx);
    // Focus-within accent border, same as `input_with_action`.
    let focus_handle = state.read(cx).focus_handle(cx);
    div()
        .track_focus(&focus_handle)
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.border)
        .in_focus(|s| s.border_color(d::PRIMARY))
        .rounded(px(d::MODAL_BUTTON_RADIUS))
        // Text region claims the row's free width — `flex_1` lets the
        // input stretch while the action chip on the right keeps its
        // intrinsic width.
        .child(div().flex_1().flex().min_w_0().child(inner))
        // Action — right-aligned on the same row, with a small gap to
        // the input and the chrome edge.
        .child(
            div()
                .flex_none()
                .flex()
                .flex_row()
                .items_center()
                .pr(px(d::INPUT_PANEL_BUTTON_GAP))
                .pl(px(d::INPUT_PANEL_BUTTON_GAP))
                .child(action),
        )
}

#[cfg(test)]
mod tests {
    use super::InputState;
    use crate::test_support::init_gpui_component;
    use gpui::TestAppContext;

    /// A single-line input must drop newlines on `set_value`. A stored
    /// newline routes into the single-line layout path, where gpui's
    /// `shape_line` panics ("text argument should not contain newlines").
    /// Regression: opening the file-viewer editor search (Cmd+F) over a
    /// multi-line selection pre-filled the single-line search input with
    /// the selection, newlines and all.
    #[gpui::test]
    fn single_line_set_value_strips_newlines(cx: &mut TestAppContext) {
        init_gpui_component(cx);
        let window = cx.add_window(InputState::new);
        window
            .update(cx, |state, window, cx| {
                state.set_value("first\nsecond\nthird", window, cx);
                assert_eq!(state.value().as_ref(), "firstsecondthird");
                assert!(!state.value().contains('\n'));
            })
            .unwrap();
    }

    /// A multi-line input keeps newlines — the strip is single-line only.
    #[gpui::test]
    fn multi_line_set_value_keeps_newlines(cx: &mut TestAppContext) {
        init_gpui_component(cx);
        let window = cx.add_window(|window, cx| InputState::new(window, cx).multi_line(true));
        window
            .update(cx, |state, window, cx| {
                state.set_value("first\nsecond", window, cx);
                assert_eq!(state.value().as_ref(), "first\nsecond");
            })
            .unwrap();
    }
}
