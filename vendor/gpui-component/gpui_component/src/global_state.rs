use gpui::{App, Entity, Global, Pixels, Point, SharedString};

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
    /// The link the last right press landed on — its URL *and* the position
    /// of the press that found it — recorded in the **capture** phase so a
    /// host's own bubble-phase right-press handler, the one that opens a
    /// context menu, can read it while building that menu.
    ///
    /// The position is what makes a stale record harmless: the host asks with
    /// the position of the press it is answering, so a record left behind by
    /// some other press cannot be adopted. Consumed by
    /// [`crate::text::take_right_clicked_link`].
    pub(crate) right_clicked_link: Option<(Point<Pixels>, SharedString)>,
}

impl GlobalState {
    pub(crate) fn new() -> Self {
        Self {
            text_view_state_stack: Vec::new(),
            selecting_state: None,
            right_clicked_link: None,
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
