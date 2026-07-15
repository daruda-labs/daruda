use gpui::{App, Entity, Global};

use crate::text::TextViewState;

pub(crate) fn init(cx: &mut App) {
    cx.set_global(GlobalState::new());
}

impl Global for GlobalState {}

pub(crate) struct GlobalState {
    pub(crate) text_view_state_stack: Vec<Entity<TextViewState>>,
    /// The `TextViewState` whose selectable block is currently in an active
    /// drag-selection (mouse held down). Set by the block's mouse-down handler
    /// when a selection starts, cleared on mouse-up / outside-clear. Read by
    /// [`crate::text::active_text_selection`] so a host (e.g. an autoscroll
    /// driver) can extend or bound the live selection while the drag runs.
    pub(crate) selecting_state: Option<Entity<TextViewState>>,
}

impl GlobalState {
    pub(crate) fn new() -> Self {
        Self {
            text_view_state_stack: Vec::new(),
            selecting_state: None,
        }
    }

    pub(crate) fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub(crate) fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }

    pub(crate) fn text_view_state(&self) -> Option<&Entity<TextViewState>> {
        self.text_view_state_stack.last()
    }
}
