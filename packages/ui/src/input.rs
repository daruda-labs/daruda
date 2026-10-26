//! Text input factory over `gpui_component::input`.
//!
//! Supports single-line and multi-line states with daruda modal chrome. Tab
//! participation mirrors checkbox/radio/select: pass an `isize` for a modal
//! cycle slot, or `()` to remove read-only/display-only inputs from Tab order.
//! `Input` lacks a `tab_stop` builder, so the `()` path mutates the state's
//! `FocusHandle` directly.

use crate::field_geometry::FieldGeometry;
use crate::theme as d;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Element as _, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use gpui_component::{Disableable as _, Sizable as _};

use gpui_component::input::Input;
pub use gpui_component::input::Paste as InputPaste;
pub use gpui_component::input::{
    CompletionProvider, HistoryDir, InputEvent, InputState, Rope, RopeExt, ScrollWheelBehavior,
};

/// Tab-participation specifier for [`input`].
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

/// Construct a daruda-toned input bound to `state`.
///
/// `.w_full()` is baked in: the inner `gpui_component::Input` sizes
/// to its parent via `size_full`, so a content-sized wrapper would
/// collapse the input to zero width inside a `flex_row` parent
/// (where the cross-axis stretch only fills vertically). Callers
/// that need a narrower input can place this inside a sized
/// container (`div().w(px(N))....child(input(...))`).
/// [`input`], on a surface the caller names.
///
/// For a pane that follows the terminal colour theme rather than the workspace
/// chrome: a box carrying `modal_input_bg` on such a surface is the one light
/// rectangle in a dark pane. The same reason [`crate::file_viewer_editor`]
/// takes the file-viewer background.
pub fn input_on<T: InputTabSpec>(
    state: &Entity<InputState>,
    surface: gpui::Hsla,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    field_input(state, surface, FieldGeometry::STANDARD, cx, tab)
}

fn field_input<T: InputTabSpec>(
    state: &Entity<InputState>,
    surface: gpui::Hsla,
    geometry: FieldGeometry,
    cx: &App,
    tab: T,
) -> Input {
    let inner = geometry.apply(
        Input::new(state)
            .disabled(state.read(cx).is_disabled())
            .small()
            .bordered(true)
            .w_full()
            .h_auto()
            .min_h(px(geometry.height))
            .bg(surface),
    );
    tab.apply(state, cx, inner)
}

/// Standard 28px field. Multi-line states grow with their row count.
pub fn input<T: InputTabSpec>(state: &Entity<InputState>, cx: &App, tab: T) -> impl IntoElement {
    input_on(state, d::current(cx).modal_input_bg, cx, tab)
}

/// Roomier 36px field for authoring forms; shares typography with [`input`].
pub fn form_input<T: InputTabSpec>(
    state: &Entity<InputState>,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    field_input(
        state,
        d::current(cx).modal_input_bg,
        FieldGeometry::FORM,
        cx,
        tab,
    )
}

/// Search fields use standard geometry and keep the caller's Tab policy.
pub fn search_input<T: InputTabSpec>(
    state: &Entity<InputState>,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    input(state, cx, tab)
}

/// A clear action in a reserved suffix slot, with the same outer field size.
pub fn search_with_clear(
    clear_id: &'static str,
    tooltip: SharedString,
    state: &Entity<InputState>,
    has_query: bool,
    on_clear: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let input = field_input(
        state,
        d::current(cx).modal_input_bg,
        FieldGeometry::STANDARD,
        cx,
        (),
    );
    if !has_query {
        return input;
    }
    let focus = state.read(cx).focus_handle(cx);
    let disabled = state.read(cx).is_disabled();
    input.suffix(
        div()
            .relative()
            .w(px(d::CONTROL_TARGET_SIZE))
            .h(px(d::INPUT_LINE_HEIGHT))
            .child(
                crate::button_icon(clear_id, crate::icons::CLOSE, cx)
                    .disabled(disabled)
                    .debug_selector(move || clear_id.into())
                    .tooltip(tooltip)
                    .absolute()
                    .top(px(d::SEARCH_CLEAR_OFFSET_Y))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        cx.stop_propagation();
                        on_clear(window, cx);
                        window.focus(&focus, cx);
                    }),
            ),
    )
}

/// Borderless search inside a pane-owned search toolbar.
pub fn search_input_on<T: InputTabSpec>(
    state: &Entity<InputState>,
    background: gpui::Hsla,
    foreground: gpui::Hsla,
    cx: &App,
    tab: T,
) -> impl IntoElement {
    field_input(state, background, FieldGeometry::STANDARD, cx, tab)
        .bordered(false)
        .py(px(FieldGeometry::STANDARD.pad_y + d::INPUT_BORDER_WIDTH))
        .text_color(foreground)
}

pub fn readonly_input(value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    readonly_field(value, FieldGeometry::STANDARD, cx)
}

pub fn readonly_form_input(value: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    readonly_field(value, FieldGeometry::FORM, cx)
}

fn readonly_field(
    value: impl Into<SharedString>,
    geometry: FieldGeometry,
    cx: &App,
) -> impl IntoElement {
    let t = d::current(cx);
    geometry
        .apply(div())
        .min_h(px(geometry.height))
        .rounded(px(d::MODAL_BUTTON_RADIUS))
        .bg(t.modal_input_bg)
        .border_1()
        .border_color(t.border)
        .text_color(t.text_primary)
        .child(value.into())
}

/// Chrome-cell input variant that hosts an action element (mode chip +
/// Submit/Stop button). Same daruda tokens as [`input`] (`MODAL_INPUT_BG`
/// / border / radius) so the two read as the same widget family. Used
/// by the bottom-dock terminal input.
///
/// In `AutoGrow` mode the layout is **stacked** (`flex_col`): the text
/// area occupies the full width on the top row, and the action row sits
/// on its own row below, right-aligned. This mirrors Zed's composer
/// design where text and buttons never share a horizontal row. The
/// earlier side-by-side layout clipped long lines behind the button
/// column; the stacked layout eliminates that overlap at any dock height.
///
/// In `Fill` mode the classic side-by-side layout is kept (`flex_row
/// [text|buttons]`) — this path has no live callers in the bottom dock
/// but is preserved for completeness.
///
/// `action` is any [`IntoElement`] — typically a `flex_row` of a mode
/// chip plus a `button_primary` / `button_danger`. Variant / disabled /
/// on_click choices stay at the call site.
///
/// Caller controls width via the surrounding container; the wrapper
/// commits only to chrome + layout.
pub enum InputGrowMode {
    /// Classic mode: editor fills the available height and scrolls.
    Fill,
    /// Auto-grow mode: editor grows with content. The auto-grow cap is
    /// baked into the [`InputState`] via [`InputState::auto_grow`] /
    /// [`InputState::set_auto_grow`] — `input_with_action_grow` reads
    /// the state-owned cap directly. The outer dock height is driven
    /// separately by `adapt_dock_to_input_lines` on `InputEvent::Change`.
    AutoGrow,
}

pub(crate) fn composer_input(state: &Entity<InputState>, cx: &App) -> Input {
    composer_input_sized(state, cx, d::UiMetrics::default())
}

pub(crate) fn composer_input_sized(
    state: &Entity<InputState>,
    cx: &App,
    metrics: d::UiMetrics,
) -> Input {
    Input::new(state)
        .small()
        .appearance(false)
        .input_padding(false)
        .disabled(state.read(cx).is_disabled())
        .text_size(px(metrics.text(d::FONT_SIZE_MD)))
        .line_height(px(metrics.row(d::INPUT_LINE_HEIGHT)))
}

pub fn input_with_action<T: InputTabSpec>(
    state: &Entity<InputState>,
    action: impl IntoElement,
    cx: &App,
    tab: T,
) -> AnyElement {
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
) -> AnyElement {
    // Disable the size-derived inner padding (`input_px`/`input_py`) on the
    // Input itself so the text column wrapper can apply the DESIGN.md spec:
    // `padding: sm md (8px 12px)` — `INPUT_TEXTAREA_PAD_X` horizontal,
    // `INPUT_TEXTAREA_PAD_Y` vertical. Without `input_padding(false)` the
    // `Small` defaults (px=8, py=2) would contradict the spec.
    let inner = match grow_mode {
        // In `Fill` mode `h_full()` fills the text column's (definite) height
        // and scrolls internally instead of auto-growing past it. The row is
        // left at the default cross-axis `stretch`, so the `flex_1` text column
        // inherits the row's full height — that definite height is what
        // `h_full`'s `relative(1.)` resolves against. Single-line states ignore
        // it.
        InputGrowMode::Fill => composer_input(state, cx).h_full(),
        // In `AutoGrow` mode the inner editor self-sizes to N lines (already
        // configured via `InputState::auto_grow`); `h_full` is omitted so the
        // column shrinks to the editor's natural height instead of stretching
        // to fill the parent. The auto-grow cap is owned by `InputState` (set
        // at construction via `auto_grow(1, max_rows)` and updated on live
        // config reload via `set_auto_grow`).
        InputGrowMode::AutoGrow => composer_input(state, cx),
    };
    let inner = tab.apply(state, cx, inner);
    let t = d::current(cx);
    // Focus-within accent border.
    let focus_handle = state.read(cx).focus_handle(cx);
    // The editor scrolls itself (negative y = rows hidden above). Read from the
    // last paint; an input scroll notifies its ancestors, so this re-renders.
    let scrolled_past_top = state.read(cx).scroll_handle().offset().y < px(-0.5);

    match grow_mode {
        InputGrowMode::Fill => {
            // Fill: classic side-by-side layout — text flex_1 on the left,
            // action column flex_none on the right, both in a flex_row.
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
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .min_w_0()
                        .px(px(d::INPUT_TEXTAREA_PAD_X))
                        .py(px(d::INPUT_TEXTAREA_PAD_Y))
                        .child(inner),
                )
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
                .into_any()
        }
        InputGrowMode::AutoGrow => {
            // AutoGrow: stacked layout — text area full-width on top, action
            // row right-aligned on its own row below. This mirrors Zed's
            // composer where text and buttons never share a horizontal row,
            // eliminating the clip of long lines behind the button column.
            // The dock height formula (`bottom_dock_height_for_rows`) accounts
            // for the action row's fixed height (`DOCK_BOTTOM_INPUT_ACTION_ROW_H`)
            // as a constant addend over the per-line text area.
            div()
                .track_focus(&focus_handle)
                .flex()
                .flex_col()
                .w_full()
                .bg(t.modal_input_bg)
                .border_1()
                .border_color(t.border)
                .in_focus(|s| s.border_color(d::PRIMARY))
                .rounded(px(d::RADIUS_MD))
                // Text area — full width, auto-sized to content row count. Once
                // rows scroll off the top, a fade over the first row says the
                // draft continues above (DESIGN.md's tab-overflow edge fade).
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .min_w_0()
                        .px(px(d::INPUT_TEXTAREA_PAD_X))
                        .pt(px(d::COMPOSER_TEXT_PAD_Y))
                        .child(inner)
                        .when(scrolled_past_top, |area| {
                            area.child(overflow_fade_top(t.modal_input_bg))
                        }),
                )
                // Action row — right-aligned below the text, with gap to the
                // chrome edge. Matches `DOCK_BOTTOM_INPUT_ACTION_ROW_H` in
                // the height formula: `BUTTON_HEIGHT + INPUT_PANEL_BUTTON_GAP`.
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .pr(px(d::INPUT_PANEL_BUTTON_GAP))
                        .pl(px(d::INPUT_PANEL_BUTTON_GAP))
                        .pb(px(d::INPUT_PANEL_BUTTON_GAP))
                        .child(action),
                )
                .into_any()
        }
    }
}

/// Input-colored to transparent over the composer's top padding and first
/// row. It has no listeners, so clicks and wheel events reach the editor.
fn overflow_fade_top(bg: gpui::Hsla) -> gpui::Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(d::INPUT_OVERFLOW_FADE_H))
        .bg(gpui::linear_gradient(
            180.,
            // Opaque over the padding only: the first row itself fades out
            // from its top edge, so it reads as cut off rather than missing.
            gpui::linear_color_stop(bg, d::COMPOSER_TEXT_PAD_Y / d::INPUT_OVERFLOW_FADE_H),
            gpui::linear_color_stop(bg.opacity(0.), 1.),
        ))
}

#[cfg(test)]
mod geometry_tests;

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

    /// Raising the auto-grow cap shows the rows the old cap hid at once — the
    /// expand toggle has no edit to wait for — and lowering it hides them again.
    #[gpui::test]
    fn set_auto_grow_reclamps_the_shown_rows(cx: &mut TestAppContext) {
        init_gpui_component(cx);
        let window = cx.add_window(|window, cx| InputState::new(window, cx).auto_grow(1, 2));
        window
            .update(cx, |state, window, cx| {
                state.set_value("1\n2\n3\n4", window, cx);
                assert_eq!(state.display_rows(), 2);
                state.set_auto_grow(1, 5);
                assert_eq!(state.display_rows(), 4);
                state.set_auto_grow(1, 3);
                assert_eq!(state.display_rows(), 3);
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

    /// Reproduction: composing a Korean syllable via the macOS IME when the
    /// composition sits at a non-zero document offset (here, on the second line
    /// after a newline). macOS drives this through `EntityInputHandler`:
    /// `set_marked_text` → `replace_and_mark_text_in_range(replacementRange,
    /// text, selectedRange)` where `selectedRange` is RELATIVE to the marked
    /// text (per NSTextInputClient), then `insert_text` → `replace_text_in_range`
    /// on commit. We drive the exact same calls to check the syllable survives.
    #[gpui::test]
    fn korean_ime_compose_after_newline_preserves_syllable(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler as _;
        init_gpui_component(cx);
        let window = cx.add_window(|window, cx| InputState::new(window, cx).multi_line(true));
        window
            .update(cx, |state, window, cx| {
                // Line 1 = "가", then a newline; compose on line 2 so the marked
                // text is at a non-zero offset.
                state.set_value("가\n", window, cx);
                state.move_cursor_to_end(cx);

                // Compose 한 = ㅎ → 하 → 한. selectedRange {1,0} (cursor after the
                // single-unit marked text), replacementRange = None (continuing
                // composition relies on the app's markedRange).
                state.replace_and_mark_text_in_range(None, "ㅎ", Some(1..1), window, cx);
                state.replace_and_mark_text_in_range(None, "하", Some(1..1), window, cx);
                state.replace_and_mark_text_in_range(None, "한", Some(1..1), window, cx);

                // While marking, the text is "가\n한" and the marked range is the
                // trailing 한 (UTF-16 offsets 2..3).
                assert_eq!(state.value().as_ref(), "가\n한", "marked value");
                assert_eq!(
                    state.marked_text_range(window, cx),
                    Some(2..3),
                    "marked range (utf-16)"
                );
                // The cursor / selection must stay within the document.
                let utf16_len = state.value().encode_utf16().count();
                let sel = state
                    .selected_text_range(false, window, cx)
                    .expect("selection")
                    .range;
                assert!(
                    sel.start <= utf16_len && sel.end <= utf16_len,
                    "selection {sel:?} out of bounds (utf-16 len {utf16_len})"
                );

                // Commit (insertText with NSNotFound replacementRange → None).
                state.replace_text_in_range(None, "한", window, cx);
                assert_eq!(state.value().as_ref(), "가\n한", "committed value");
            })
            .unwrap();
    }
}
