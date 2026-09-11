//! Tail-window chip, its dropdown menu, and the equivalent panel body.
//!
//! Third of the Activity Bar's transcript axes, alongside `fold_mode` and
//! `filter`: each owns its chip label, its return to the configured default,
//! and the panel the popover shows for its tab. This one carries a window per
//! [`TailLevel`], listed from `TailLevel::ALL` on every surface below.

use daruda_config::TAIL_WINDOW_CHOICES;
use gpui::{AnyElement, Context, IntoElement, SharedString, prelude::*, px};

use super::axis_chip::axis_chip_label;
use crate::surface::strings as s;
use crate::transcript::editor::{panel_heading, scroll_region};
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, Selectable as _, button_chip_on_surface, radio,
};
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::tail::{TailLevel, TailWindow};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// One entry of one level's list. `Default` is not a window value — it hands
/// that level back to config, which is why the two cannot be one type: a level
/// following `All` and a level pinned to `All` are different states.
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

    /// Exactly one entry is checked per level: a following level marks
    /// `Default`, so the list states *that* it follows config rather than
    /// restating the value the chip already carries.
    fn is_current(self, tail: PaneChoice<TailWindow>) -> bool {
        match self {
            Self::Default => tail.is_following(),
            Self::Window(window) => tail.chosen() == Some(window),
        }
    }
}

/// Both levels' choices, as the chip and its two surfaces read them.
#[derive(Clone, Copy)]
pub(in crate::workspace::main_area::agent_chat_pane::render) struct TailChoices {
    pub steps: PaneChoice<TailWindow>,
    pub calls: PaneChoice<TailWindow>,
}

impl TailChoices {
    fn get(self, level: TailLevel) -> PaneChoice<TailWindow> {
        match level {
            TailLevel::Steps => self.steps,
            TailLevel::Calls => self.calls,
        }
    }

    /// Whether the whole axis still follows config — both levels do.
    pub(in crate::workspace::main_area::agent_chat_pane::render) fn is_following(self) -> bool {
        self.steps.is_following() && self.calls.is_following()
    }
}

/// Activity-bar chip for the tail window.
pub(super) fn tail_window_chip(
    pane_id: PaneId,
    tail: TailChoices,
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
/// the compact bar's tooltip, so the two readings cannot diverge.
///
/// The second slot appears only when the call level withholds something the
/// steps slot does not already state — not when it matches the steps, and not
/// on `All`, which withholds no call at all.
pub(super) fn tail_window_chip_label(tail: TailChoices) -> String {
    let steps = tail.steps.value();
    let calls = tail.calls.value();
    let value = if calls == TailWindow::All || calls == steps {
        tail_window_value(steps)
    } else {
        s::agent_chat_tail_window_pair(&tail_window_value(steps), &tail_window_value(calls))
    };
    // The mark is about following config, and an axis is following only when
    // both of its levels are.
    axis_chip_label(s::agent_chat_tail_window_chip(&value), tail.is_following())
}

pub(super) fn tail_window_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TailChoices,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    // A heading per level, flat in one scrolling band — the shape the filter
    // panel gives its facet axes. Its siblings head only *sub*-sections, and a
    // level is exactly that; scrolling matters because two levels' lists
    // together outgrow the popover where one did not.
    let mut band = scroll_region(SharedString::from(format!(
        "agent-chat-tail-levels-{pane_id}"
    )))
    .text_size(px(theme::agent_chat_font_size(cx)));
    for level in TailLevel::ALL {
        band = band.child(panel_heading(tail_level_heading(level), cx));
        band = band.children(tail_window_choices().map(|choice| {
            let view = view.clone();
            radio(
                SharedString::from(format!(
                    "agent-chat-tail-option-{}-{}-{pane_id}",
                    level.token(),
                    choice.token()
                )),
                choice.label(),
                (),
            )
            .checked(choice.is_current(current.get(level)))
            .on_click(move |_, _window, app| {
                if let Some(view) = view.upgrade() {
                    view.update(app, |v, cx| apply_choice(v, level, choice, cx));
                }
            })
        }));
    }
    band.into_any_element()
}

/// The one place a picked entry becomes a state change, shared by the chip's
/// menu and the panel's radio group so the two cannot drift on what an entry
/// does.
fn apply_choice(
    view: &mut AgentChatView,
    level: TailLevel,
    choice: TailChoice,
    cx: &mut Context<AgentChatView>,
) {
    match choice {
        TailChoice::Default => view.reset_tail_window(level, cx),
        TailChoice::Window(window) => view.set_tail_window(level, window, cx),
    }
}

/// A level's entries in list order — one list behind both the dropdown and the
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

fn tail_level_heading(level: TailLevel) -> String {
    match level {
        TailLevel::Steps => s::agent_chat_tail_level_steps(),
        TailLevel::Calls => s::agent_chat_tail_level_calls(),
    }
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
    current: TailChoices,
    menu: PopupMenu,
) -> PopupMenu {
    TailLevel::ALL
        .into_iter()
        .enumerate()
        .fold(menu, |menu, (ix, level)| {
            // A flat menu with a heading per level rather than two submenus:
            // every entry stays one click away, which is the whole reason the
            // chip carries a menu next to the panel that lists the same set.
            let menu = if ix == 0 { menu } else { menu.separator() };
            let menu = menu.item(PopupMenuItem::label(SharedString::from(
                tail_level_heading(level),
            )));
            tail_window_choices().fold(menu, |m, choice| {
                let view = view.clone();
                m.item(
                    PopupMenuItem::new(SharedString::from(choice.label()))
                        .checked(choice.is_current(current.get(level)))
                        .on_click(move |_, _window, app| {
                            if let Some(view) = view.upgrade() {
                                view.update(app, |v, cx| apply_choice(v, level, choice, cx));
                            }
                        }),
                )
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices(steps: PaneChoice<TailWindow>, calls: PaneChoice<TailWindow>) -> TailChoices {
        TailChoices { steps, calls }
    }

    fn following(window: TailWindow) -> TailChoices {
        choices(PaneChoice::Seeded(window), PaneChoice::Seeded(window))
    }

    #[test]
    fn the_chip_names_the_current_window() {
        assert!(
            tail_window_chip_label(following(TailWindow::All))
                .contains(&s::agent_chat_tail_window_all())
        );
        let last = TailWindow::last(TAIL_WINDOW_CHOICES[0]);
        assert_ne!(
            tail_window_chip_label(following(last)),
            tail_window_chip_label(following(TailWindow::All))
        );
    }

    /// The second slot costs bar width, so it appears only when the call level
    /// withholds something the steps slot does not already state.
    #[test]
    fn the_chip_spends_its_second_slot_only_on_a_call_window_of_its_own() {
        let seeded = |steps, calls| {
            tail_window_chip_label(choices(
                PaneChoice::Seeded(steps),
                PaneChoice::Seeded(calls),
            ))
        };
        let all = s::agent_chat_tail_window_all();

        // Nothing of its own to say: same window as the steps, or no window.
        assert_eq!(
            seeded(TailWindow::All, TailWindow::All),
            seeded_one(all.clone())
        );
        assert_eq!(
            seeded(TailWindow::Last(3), TailWindow::All),
            seeded_one(s::agent_chat_tail_window_last(3)),
            "a call level that withholds nothing must not restate the steps"
        );
        assert_eq!(
            seeded(TailWindow::Last(2), TailWindow::Last(2)),
            seeded_one(s::agent_chat_tail_window_last(2))
        );

        // Its own window: both slots, both values named.
        let split = seeded(TailWindow::All, TailWindow::Last(3));
        assert_ne!(split, seeded_one(all.clone()));
        assert!(
            split.contains('3') && split.contains(&all),
            "the split label names both: {split}"
        );
        let both = seeded(TailWindow::Last(10), TailWindow::Last(3));
        assert!(
            both.contains("10") && both.contains('3'),
            "the split label names both: {both}"
        );
    }

    /// The single-slot chip, as the collapsed cases above must all render.
    fn seeded_one(value: String) -> String {
        s::agent_chat_tail_window_chip(&value)
    }

    /// The mark is about following config, not about the value — and one level
    /// pinned is enough to take the axis off the default.
    #[test]
    fn either_level_marks_the_chip() {
        let all = TailWindow::All;
        let baseline = tail_window_chip_label(following(all));
        for pinned in [
            choices(PaneChoice::Chosen(all), PaneChoice::Seeded(all)),
            choices(PaneChoice::Seeded(all), PaneChoice::Chosen(all)),
        ] {
            assert_ne!(tail_window_chip_label(pinned), baseline);
            assert!(!pinned.is_following());
        }
        assert!(following(all).is_following());
    }

    /// `All` is a value the user can pin; `Default` is the absence of a pin.
    /// The list has to offer both, and check exactly one — per level.
    #[test]
    fn the_list_separates_following_from_pinning_the_same_value() {
        let checked = |tail| {
            tail_window_choices()
                .filter(|c| c.is_current(tail))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            checked(PaneChoice::Seeded(TailWindow::All)),
            vec![TailChoice::Default]
        );
        assert_eq!(
            checked(PaneChoice::Chosen(TailWindow::All)),
            vec![TailChoice::Window(TailWindow::All)]
        );
    }

    /// The dropdown and the panel's radio group are the same control in two
    /// shapes; a choice reachable from one but not the other is a bug. Both
    /// iterate every level, so the same holds level by level.
    #[test]
    fn the_menu_and_the_panel_offer_the_same_choices() {
        let choices: Vec<_> = tail_window_choices().collect();
        assert_eq!(choices.len(), TAIL_WINDOW_CHOICES.len() + 2);
        assert_eq!(choices[0], TailChoice::Default);
        assert_eq!(choices[1], TailChoice::Window(TailWindow::All));
        assert!(choices.iter().all(|c| !c.label().is_empty()));
        // Each level heads its own copy of that list, and the two headings are
        // distinguishable — a shared heading would leave the flat menu unable
        // to say which level an entry belongs to.
        let headings: Vec<_> = TailLevel::ALL
            .iter()
            .map(|l| tail_level_heading(*l))
            .collect();
        assert_eq!(headings.len(), 2);
        assert_ne!(headings[0], headings[1]);
        assert!(headings.iter().all(|h| !h.is_empty()));
    }

    /// A level's choices answer only about that level.
    #[test]
    fn one_levels_list_ignores_the_other_level() {
        let split = choices(
            PaneChoice::Chosen(TailWindow::Last(3)),
            PaneChoice::Seeded(TailWindow::All),
        );
        assert_eq!(
            tail_window_choices()
                .filter(|c| c.is_current(split.get(TailLevel::Steps)))
                .collect::<Vec<_>>(),
            vec![TailChoice::Window(TailWindow::Last(3))]
        );
        assert_eq!(
            tail_window_choices()
                .filter(|c| c.is_current(split.get(TailLevel::Calls)))
                .collect::<Vec<_>>(),
            vec![TailChoice::Default]
        );
    }
}
