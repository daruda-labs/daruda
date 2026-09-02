//! Tail-window chip, its dropdown menu, and the equivalent panel body.
//!
//! Third of the Activity Bar's transcript axes, alongside `fold_mode` and
//! `filter`: each owns its chip label, its return to the configured default,
//! and the panel the combined view-options popover shows for its tab.

use daruda_config::TAIL_WINDOW_CHOICES;
use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use super::axis_chip::axis_chip_label;
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, Selectable as _, button_chip_on_surface, radio,
};
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// One entry of the tail axis's list. `Default` is not a window value — it
/// hands the axis back to config, which is why the two cannot be one type: a
/// pane following `All` and a pane pinned to `All` are different states.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TailChoice {
    Default,
    Window(TailWindow),
}

impl TailChoice {
    /// Element-id fragment. `default` cannot collide with a window: those key
    /// off a step count.
    fn token(self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::Window(window) => window.size().to_string(),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Default => s::agent_chat_tail_window_default(),
            Self::Window(window) => tail_window_value(window),
        }
    }

    /// Exactly one entry is checked: a following pane marks `Default`, so the
    /// list states *that* the pane follows config rather than restating the
    /// value the chip already carries.
    fn is_current(self, tail: PaneChoice<TailWindow>) -> bool {
        match self {
            Self::Default => tail.is_following(),
            Self::Window(window) => tail.chosen() == Some(window),
        }
    }
}

/// Activity-bar chip for the tail window.
pub(super) fn tail_window_chip(
    pane_id: PaneId,
    tail: PaneChoice<TailWindow>,
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
    .selected(!tail.is_following())
    .tooltip(SharedString::from(s::agent_chat_tail_window_tooltip()))
    .dropdown_menu(move |menu, _window, _cx| build_tail_window_menu(&view, tail, menu))
}

/// The chip's full text, overridden mark included. Also the tail axis's line in
/// the compact bar's tooltip, so the two readings of the same setting cannot
/// diverge.
pub(super) fn tail_window_chip_label(tail: PaneChoice<TailWindow>) -> String {
    axis_chip_label(
        s::agent_chat_tail_window_chip(&tail_window_value(tail.value())),
        tail,
    )
}

pub(super) fn tail_window_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    current: PaneChoice<TailWindow>,
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
        .children(tail_window_choices().map(|choice| {
            let view = view.clone();
            radio(
                SharedString::from(format!(
                    "agent-chat-tail-option-{}-{pane_id}",
                    choice.token()
                )),
                choice.label(),
                (),
            )
            .checked(choice.is_current(current))
            .on_click(move |_, _window, app| {
                if let Some(view) = view.upgrade() {
                    view.update(app, |v, cx| match choice {
                        TailChoice::Default => v.reset_tail_window(cx),
                        TailChoice::Window(window) => v.set_tail_window(window, cx),
                    });
                }
            })
        }))
        .into_any_element()
}

/// The axis's entries in list order — one list behind both the dropdown and the
/// panel's radio group. `Default` leads: it is the state the others depart
/// from, and this axis has no footer button to hold it.
fn tail_window_choices() -> impl Iterator<Item = TailChoice> {
    std::iter::once(TailChoice::Default)
        .chain(std::iter::once(TailChoice::Window(TailWindow::All)))
        .chain(
            TAIL_WINDOW_CHOICES
                .into_iter()
                .map(|n| TailChoice::Window(TailWindow::last(n))),
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
    current: PaneChoice<TailWindow>,
    menu: PopupMenu,
) -> PopupMenu {
    tail_window_choices().fold(menu, |m, choice| {
        let view = view.clone();
        m.item(
            PopupMenuItem::new(SharedString::from(choice.label()))
                .checked(choice.is_current(current))
                .on_click(move |_, _window, app| {
                    if let Some(view) = view.upgrade() {
                        view.update(app, |v, cx| match choice {
                            TailChoice::Default => v.reset_tail_window(cx),
                            TailChoice::Window(window) => v.set_tail_window(window, cx),
                        });
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
        assert!(
            tail_window_chip_label(PaneChoice::Seeded(TailWindow::All))
                .contains(&s::agent_chat_tail_window_all())
        );
        let last = TailWindow::last(TAIL_WINDOW_CHOICES[0]);
        assert_ne!(
            tail_window_chip_label(PaneChoice::Seeded(last)),
            tail_window_chip_label(PaneChoice::Seeded(TailWindow::All))
        );
    }

    /// The mark is about following config, not about the value.
    #[test]
    fn only_a_chosen_window_marks_the_chip() {
        let all = TailWindow::All;
        assert_ne!(
            tail_window_chip_label(PaneChoice::Chosen(all)),
            tail_window_chip_label(PaneChoice::Seeded(all))
        );
    }

    /// `All` is a value the user can pin; `Default` is the absence of a pin.
    /// The list has to offer both, and check exactly one.
    #[test]
    fn the_list_separates_following_from_pinning_the_same_value() {
        let following = PaneChoice::Seeded(TailWindow::All);
        let pinned = PaneChoice::Chosen(TailWindow::All);
        let checked = |tail| {
            tail_window_choices()
                .filter(|c| c.is_current(tail))
                .collect::<Vec<_>>()
        };
        assert_eq!(checked(following), vec![TailChoice::Default]);
        assert_eq!(checked(pinned), vec![TailChoice::Window(TailWindow::All)]);
    }

    /// The dropdown and the panel's radio group are the same control in two
    /// shapes; a choice reachable from one but not the other is a bug.
    #[test]
    fn the_menu_and_the_panel_offer_the_same_choices() {
        let choices: Vec<_> = tail_window_choices().collect();
        assert_eq!(choices.len(), TAIL_WINDOW_CHOICES.len() + 2);
        assert_eq!(choices[0], TailChoice::Default);
        assert_eq!(choices[1], TailChoice::Window(TailWindow::All));
        assert!(choices.iter().all(|c| !c.label().is_empty()));
    }
}
