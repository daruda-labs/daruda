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
//! overrides the bg via `.bg()`. [`input_with_action`] hosts the action
//! in an outer div, so it keeps the inner `Input` borderless and draws
//! the bg / border / focus-within accent on that div.
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

pub use gpui_component::input::{
    CompletionProvider, HistoryDir, Input, InputEvent, InputState, Rope, RopeExt,
};

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

/// Chrome-cell input variant that hosts an action element beside the
/// text on the right. Same daruda tokens as [`input`] (`MODAL_INPUT_BG`
/// / border / radius) so the two read as the same widget family. Used
/// by the bottom-dock terminal input, where the Submit button lives
/// inside the chrome.
///
/// The action sits in its own full-height column to the right of the
/// text (not floating over it), so the text uses the cell's full height
/// at every dock size — no bottom strip is reserved. The earlier
/// floating variant reserved a full-width bottom band for a button that
/// only occupied the bottom-right corner, blanking ~1.5 lines of text on
/// short docks; this layout avoids that.
///
/// `action` is any [`IntoElement`] — typically `crate::ui::button_*`
/// for a single button, but a `DropdownButton` split or a small
/// `flex_row` of multiple buttons works just as well. Variant /
/// disabled / on_click choices stay at the call site.
///
/// Caller controls width via the surrounding container; the wrapper
/// commits only to chrome + the row layout.
/// Max-rows value passed to [`input_with_action`]. When the input state
/// was built with [`InputState::auto_grow`], this cap is already baked
/// into the state; this argument merely documents the call site's
/// intent without duplicating the state-side value.
///
/// Pass `None` to keep the classic `h_full` fill behaviour (the input
/// scrolls within whatever height its parent allocates). Pass `Some(n)`
/// to switch to content-driven growth: the editor expands row-by-row up
/// to `n` rows and scrolls internally beyond that. The outer container
/// must be resized independently (e.g. via `set_bottom_dock_row_preset`
/// on `InputEvent::Change`) to match the desired growth.
pub enum InputGrowMode {
    /// Classic mode: editor fills the available height and scrolls.
    Fill,
    /// Auto-grow mode: editor grows with content up to `max_rows`.
    AutoGrow { max_rows: usize },
}

pub fn input_with_action<T: InputTabSpec>(
    state: &Entity<InputState>,
    action: impl IntoElement,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    input_with_action_grow(state, action, cx, tab, InputGrowMode::Fill)
}

/// Like [`input_with_action`] but lets the caller choose the growth
/// mode. Used by the bottom-dock terminal input to switch to content-
/// driven auto-grow.
pub fn input_with_action_grow<T: InputTabSpec>(
    state: &Entity<InputState>,
    action: impl IntoElement,
    cx: &App,
    tab: T,
    grow_mode: InputGrowMode,
) -> impl IntoElement {
    // In `Fill` mode `h_full()` fills the text column's (definite) height
    // and scrolls internally instead of auto-growing past it. The row is
    // left at the default cross-axis `stretch`, so the `flex_1` text column
    // inherits the row's full height — that definite height is what
    // `h_full`'s `relative(1.)` resolves against. Single-line states ignore
    // it.
    //
    // In `AutoGrow` mode the inner editor self-sizes to N lines (already
    // configured via `InputState::auto_grow`); `h_full` is omitted so the
    // column shrinks to the editor's natural height instead of stretching to
    // fill the parent.
    // Disable the size-derived inner padding (`input_px`/`input_py`) on the
    // Input itself so the text column wrapper can apply the DESIGN.md spec:
    // `padding: sm md (8px 12px)` — `INPUT_TEXTAREA_PAD_X` horizontal,
    // `INPUT_TEXTAREA_PAD_Y` vertical. Without `input_padding(false)` the
    // `Small` defaults (px=8, py=2) would contradict the spec.
    //
    // In `AutoGrow` mode the `max_rows` field is now live: it is forwarded
    // to `InputState::set_auto_grow` from the `InputEvent::Change` subscriber
    // on config reload (Issue C fix), so the state's internal cap stays in
    // sync after a live config change. The field is kept in the enum rather
    // than dropped so the render path doesn't need to re-read the state.
    let inner = match grow_mode {
        InputGrowMode::Fill => Input::new(state)
            .small()
            .appearance(false)
            .input_padding(false)
            .h_full(),
        InputGrowMode::AutoGrow { .. } => Input::new(state)
            .small()
            .appearance(false)
            .input_padding(false),
    };
    let inner = tab.apply(state, cx, inner);
    let t = d::current(cx);
    // Focus-within accent border.
    let focus_handle = state.read(cx).focus_handle(cx);
    div()
        .track_focus(&focus_handle)
        .flex()
        .flex_row()
        .w_full()
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.border)
        .in_focus(|s| s.border_color(d::PRIMARY))
        .rounded(px(d::RADIUS_MD))
        // Text region claims the row's free width and full height — `flex_1`
        // takes the free width, the row's `stretch` gives it the full height,
        // and the inner `h_full` editor scrolls within it. Padding follows
        // DESIGN.md TerminalInputDock spec: `sm md (8px 12px)`.
        .child(
            div()
                .flex_1()
                .flex()
                .min_w_0()
                .px(px(d::INPUT_TEXTAREA_PAD_X))
                .py(px(d::INPUT_TEXTAREA_PAD_Y))
                .child(inner),
        )
        // Action — beside the text on the right in its own full-height
        // column, bottom-aligned (composer feel). No vertical reservation is
        // taken from the text (the button sits in a separate column, not over
        // the text), so the text uses the cell's full height at every dock
        // size. On a 1-row dock the column is ~one line tall, so bottom and
        // center coincide.
        .child(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .justify_end()
                .pr(px(d::INPUT_PANEL_BUTTON_GAP))
                .pl(px(d::INPUT_PANEL_BUTTON_GAP))
                .pb(px(d::INPUT_PANEL_BUTTON_GAP))
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
