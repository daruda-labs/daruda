//! Select wrapper over `gpui_component::select`.
//!
//! Keeps daruda's `(value, label)` option shape while using upstream
//! state/render split. Field geometry is shared with text inputs, and
//! Tab participation mirrors other inputs: `isize` cycles, `()` skips. The skip
//! path mutates the stored `FocusHandle` because upstream has no builder for it.

use crate::field_geometry::FieldGeometry;
use crate::theme;
use gpui::{App, Context, Entity, Focusable as _, SharedString, Styled as _, Window, px};
use gpui_component::Sizable as _;
use gpui_component::select::SelectItem;

pub use gpui_component::select::{Select, SelectEvent, SelectState as GpuiSelectState};

/// Re-export so call sites that build `SelectEvent` matchers can spell
/// out the value type without pulling `gpui_component` directly.
pub type ConfirmEvent = SelectEvent<Vec<SelectOption>>;

/// `(value, label)` option type — preserves the value/label distinction
/// daruda's old `Select` had. Implements `SelectItem` so it slots into
/// `gpui_component::Select` directly.
#[derive(Clone, Debug)]
pub struct SelectOption {
    pub value: SharedString,
    pub label: SharedString,
}

impl SelectOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }

    /// Convenience for options where value == label.
    pub fn simple(v: impl Into<SharedString>) -> Self {
        let v = v.into();
        Self {
            value: v.clone(),
            label: v,
        }
    }
}

impl SelectItem for SelectOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

/// State alias — what callers store in their entity field.
pub type SelectState = GpuiSelectState<Vec<SelectOption>>;

/// Tab-participation specifier for [`select`]. Implemented for `isize`
/// (cycle at that index) and `()` (skip cycle).
pub trait SelectTabSpec {
    fn apply(self, state: &Entity<SelectState>, cx: &App);
}

impl SelectTabSpec for isize {
    fn apply(self, state: &Entity<SelectState>, cx: &App) {
        let _ = state.read(cx).focus_handle(cx).tab_index(self);
    }
}

impl SelectTabSpec for () {
    fn apply(self, state: &Entity<SelectState>, cx: &App) {
        let _ = state.read(cx).focus_handle(cx).tab_stop(false);
    }
}

/// Construct a [`SelectState`] from a static option list, optionally
/// pre-selected. Folding the initial selection into the constructor
/// avoids the one-frame "no selection" flash that `cx.new(|cx|
/// state_with_options(...))` followed by an immediate
/// `set_selected_value` call would produce.
pub fn state_with_options(
    options: Vec<SelectOption>,
    initial: Option<&SharedString>,
    window: &mut Window,
    cx: &mut Context<SelectState>,
) -> SelectState {
    let mut state = GpuiSelectState::new(options, None, window, cx);
    if let Some(value) = initial {
        state.set_selected_value(value, window, cx);
    }
    state
}

/// Build a render-time [`Select`] element from a state entity, sized
/// to the standard 28px field. Caller chains `.placeholder(...)` / `.disabled(...)` etc.
///
/// `tab` decides Tab cycle participation (`isize` for a slot, `()` to
/// skip). The `cx` is needed only to dereference the state entity and
/// reach the `FocusHandle` — no rendering work happens here.
pub fn select<T: SelectTabSpec>(
    state: &Entity<SelectState>,
    cx: &App,
    tab: T,
) -> Select<Vec<SelectOption>> {
    select_with_geometry(state, cx, tab, FieldGeometry::STANDARD)
}

/// A 36px select aligned with [`crate::form_input`].
pub fn form_select<T: SelectTabSpec>(
    state: &Entity<SelectState>,
    cx: &App,
    tab: T,
) -> Select<Vec<SelectOption>> {
    select_with_geometry(state, cx, tab, FieldGeometry::FORM)
}

fn select_with_geometry<T: SelectTabSpec>(
    state: &Entity<SelectState>,
    cx: &App,
    tab: T,
    geometry: FieldGeometry,
) -> Select<Vec<SelectOption>> {
    tab.apply(state, cx);
    // Match the `input` surface — override Select's default
    // `theme.background` with `modal_input_bg` (refine_style wins).
    geometry.apply(
        Select::new(state)
            .small()
            .h(px(geometry.height))
            .bg(theme::current(cx).modal_input_bg),
    )
}
