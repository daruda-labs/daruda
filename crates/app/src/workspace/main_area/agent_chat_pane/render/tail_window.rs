//! Tail-window chip, its dropdown menu, and the equivalent panel body.
//!
//! Third of the Activity Bar's transcript axes, alongside `fold_mode` and
//! `filter`: each owns its chip label, its default test, and the panel the
//! combined view-options popover shows for its tab.

use daruda_config::TAIL_WINDOW_CHOICES;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, Selectable as _, button_chip_on_surface, radio,
};
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Activity-bar chip for the tail window.
pub(super) fn tail_window_chip(
    pane_id: PaneId,
    tail: TailWindow,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let view = cx.entity().downgrade();
    button_chip_on_surface(
        ("agent-chat-tail-window", pane_id as usize),
        SharedString::from(tail_window_chip_label(tail)),
        surface,
        cx,
    )
    .selected(!tail_is_default(tail))
    .tooltip(SharedString::from(s::agent_chat_tail_window_tooltip()))
    .dropdown_menu(move |menu, _window, _cx| build_tail_window_menu(&view, tail, menu))
}

/// The chip's full text. Also the tail axis's line in the compact bar's
/// tooltip, so the two readings of the same setting cannot diverge.
pub(super) fn tail_window_chip_label(tail: TailWindow) -> String {
    s::agent_chat_tail_window_chip(&tail_window_value(tail))
}

/// Whether the axis is at the value a fresh pane starts on. The compact bar
/// shows one control for three axes, so it needs this to say that something
/// behind the gear is set.
pub(super) fn tail_is_default(tail: TailWindow) -> bool {
    tail == TailWindow::All
}

pub(super) fn tail_window_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TailWindow,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    // No axis-name heading: its siblings head only *sub*-sections, and in the
    // combined popover the tab strip already names this one.
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .children(tail_window_choices().map(|(choice, label)| {
            let view = view.clone();
            radio(
                SharedString::from(format!(
                    "agent-chat-tail-option-{}-{pane_id}",
                    choice.size()
                )),
                label,
                (),
            )
            .checked(choice == current)
            .on_click(move |_, _window, app| {
                if let Some(view) = view.upgrade() {
                    view.update(app, |v, cx| v.set_tail_window(choice, cx));
                }
            })
        }))
        .into_any_element()
}

/// The axis's choices in menu order — one list behind both the dropdown and
/// the panel's radio group.
fn tail_window_choices() -> impl Iterator<Item = (TailWindow, String)> {
    std::iter::once((TailWindow::All, s::agent_chat_tail_window_all())).chain(
        TAIL_WINDOW_CHOICES.into_iter().map(|n| {
            (
                TailWindow::last(n),
                s::agent_chat_tail_window_last(usize::from(n)),
            )
        }),
    )
}

/// The chip's value slot reuses the menu item's own wording, so the chip and
/// the item that set it read alike — and a bare count can't be mistaken for the
/// "N earlier steps" row it sits above.
fn tail_window_value(tail: TailWindow) -> String {
    match tail {
        TailWindow::All => s::agent_chat_tail_window_all(),
        TailWindow::Last(n) => s::agent_chat_tail_window_last(n),
    }
}

fn build_tail_window_menu(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TailWindow,
    menu: PopupMenu,
) -> PopupMenu {
    tail_window_choices().fold(menu, |m, (choice, name)| {
        let view = view.clone();
        m.item(
            PopupMenuItem::new(SharedString::from(name))
                .checked(choice == current)
                .on_click(move |_, _window, app| {
                    if let Some(view) = view.upgrade() {
                        view.update(app, |v, cx| v.set_tail_window(choice, cx));
                    }
                }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_names_the_current_window() {
        assert!(tail_window_chip_label(TailWindow::All).contains(&s::agent_chat_tail_window_all()));
        let last = TailWindow::last(TAIL_WINDOW_CHOICES[0]);
        assert_ne!(
            tail_window_chip_label(last),
            tail_window_chip_label(TailWindow::All)
        );
    }

    #[test]
    fn only_the_full_window_counts_as_default() {
        assert!(tail_is_default(TailWindow::All));
        for n in TAIL_WINDOW_CHOICES {
            assert!(!tail_is_default(TailWindow::last(n)), "last {n}");
        }
    }

    /// The dropdown and the panel's radio group are the same control in two
    /// shapes; a choice reachable from one but not the other is a bug.
    #[test]
    fn the_menu_and_the_panel_offer_the_same_choices() {
        let choices: Vec<_> = tail_window_choices().collect();
        assert_eq!(choices.len(), TAIL_WINDOW_CHOICES.len() + 1);
        assert_eq!(choices[0].0, TailWindow::All);
        assert!(choices.iter().all(|(_, label)| !label.is_empty()));
    }
}
