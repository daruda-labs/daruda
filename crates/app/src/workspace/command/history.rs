//! Command history picker — `Cmd+Shift+H`.
//!
//! Wraps `crate::ui::list` (over `gpui_component::list`) on the focused
//! terminal pane's `command_history()`. Picking an entry scrolls the
//! terminal so the command's `B` mark lands at the top of the
//! viewport. Cancelling (Escape) dismisses without side effects.
//!
//! The modal captures the command list ONCE at open time. Live
//! updates while the picker is open are not surfaced — the user is
//! filtering the snapshot they were shown.

use daruda_terminal::{CommandHistoryEntry, view::TerminalView};
use gpui::{
    Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString, Subscription,
    WeakEntity, Window, div, prelude::*, px,
};

use crate::ui::theme;

use crate::ui::WindowExt as _;
use crate::ui::list::{FilteredItem, FilteredListState, ListEvent, list, searchable_list_state};

/// One row in the picker. Holds the underlying entry plus a
/// pre-formatted label so [`FilteredItem::label`] is a cheap clone.
#[derive(Clone)]
pub(in crate::workspace) struct CommandHistoryItem {
    pub(in crate::workspace) start_row: u32,
    pub(in crate::workspace) label: SharedString,
    /// The lower-cased command text, kept around for substring filter
    /// without re-allocating per keystroke.
    match_haystack: SharedString,
}

impl CommandHistoryItem {
    pub(in crate::workspace) fn from_entry(entry: &CommandHistoryEntry) -> Self {
        let exit_badge = match entry.exit_code {
            Some(0) => "✓ ".to_string(),
            Some(n) => format!("✗{n} "),
            None => "  ".to_string(),
        };
        let label: SharedString = format!("{exit_badge}{}", entry.command_text).into();
        let match_haystack: SharedString = entry.command_text.to_lowercase().into();
        Self {
            start_row: entry.start_row,
            label,
            match_haystack,
        }
    }
}

impl FilteredItem for CommandHistoryItem {
    fn label(&self) -> SharedString {
        self.label.clone()
    }

    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        self.match_haystack.contains(&query.to_lowercase())
    }
}

/// Modal host. Owns the list state and the per-pick callback into the
/// focused TerminalView. Closes itself via `window.close_dialog`.
pub(in crate::workspace) struct CommandHistoryModal {
    list_state: Entity<FilteredListState<CommandHistoryItem>>,
    target_view: WeakEntity<TerminalView>,
    _list_subscription: Subscription,
}

impl CommandHistoryModal {
    pub(in crate::workspace) fn new(
        target_view: WeakEntity<TerminalView>,
        items: Vec<CommandHistoryItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_state = cx.new(|cx| searchable_list_state(items, window, cx));
        let _list_subscription = cx.subscribe_in(
            &list_state,
            window,
            |this, state, ev: &ListEvent, window, cx| match ev {
                ListEvent::Confirm(ix) => {
                    let chosen = state.read(cx).delegate().item_at(*ix).cloned();
                    if let Some(entry) = chosen
                        && let Some(view) = this.target_view.upgrade()
                    {
                        view.update(cx, |v, cx| {
                            v.jump_to_screen_row_top(entry.start_row, cx);
                        });
                    }
                    window.close_dialog(cx);
                }
                ListEvent::Cancel => {
                    window.close_dialog(cx);
                }
                ListEvent::Select(_) => {}
            },
        );
        Self {
            list_state,
            target_view,
            _list_subscription,
        }
    }
}

impl Focusable for CommandHistoryModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Delegate to the list state so the embedded query input gets
        // focus on open.
        self.list_state.focus_handle(cx)
    }
}

impl Render for CommandHistoryModal {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Modal frame sized to the worktree create/remove panel for
        // chrome consistency. Dialog provides the outer chrome (bg /
        // border / radius / padding); we cap the height so a long
        // history scrolls inside.
        div()
            .w(px(theme::MODAL_PANEL_WIDTH))
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .flex()
            .flex_col()
            .child(list(&self.list_state))
    }
}
