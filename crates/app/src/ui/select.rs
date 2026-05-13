//! ui::select — wrapper over `gpui_component::select`.
//!
//! Daruda's old `Select` widget mapped `(value, label)` pairs to a
//! single dropdown, emitting `SelectEvent::Changed(value)` on pick.
//! `gpui_component::Select` decouples state (`SelectState<D>`) from
//! render (`Select<D>`) and emits `SelectEvent::Confirm(Option<Value>)`,
//! so callers wire two pieces:
//!
//! 1. Construct [`SelectState`] (a `Vec<SelectOption>`-delegated alias)
//!    inside an `Entity` field via [`state_with_options`], passing an
//!    optional initial value.
//! 2. In `render`, build the visual element with [`select`] and chain
//!    placeholders / disabled flags as usual.
//!
//! `xsmall()` is auto-applied so call sites stay compact (CLAUDE.md
//! §10). The third argument of [`select`] selects Tab participation
//! via the [`SelectTabSpec`] trait: pass `isize` to cycle at that
//! index or `()` to skip the cycle. `gpui_component::Select` has no
//! `.tab_index(n)` builder, so the spec mutates the underlying
//! `FocusHandle`'s tab fields directly — the value sticks because the
//! handle is stored on `SelectState` and reused across re-renders.

use gpui::{App, Context, Entity, Focusable as _, SharedString, Window};
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
/// `xsmall`. Caller chains `.placeholder(...)` / `.disabled(...)` etc.
///
/// `tab` decides Tab cycle participation (`isize` for a slot, `()` to
/// skip). The `cx` is needed only to dereference the state entity and
/// reach the `FocusHandle` — no rendering work happens here.
pub fn select<T: SelectTabSpec>(
    state: &Entity<SelectState>,
    cx: &App,
    tab: T,
) -> Select<Vec<SelectOption>> {
    tab.apply(state, cx);
    Select::new(state).xsmall()
}
