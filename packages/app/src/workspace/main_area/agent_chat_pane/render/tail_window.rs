//! Activity-range tooltip and the shared range editor bound to this pane.

#[cfg(test)]
use daruda_config::TAIL_WINDOW_CHOICES;
use gpui::{AnyElement, Context};
use std::rc::Rc;

use super::axis_chip::axis_chip_label;
use crate::surface::strings as s;
use crate::transcript::editor::range::{RangeLevel, range_editor};
use crate::transcript::editor::{ResetSpec, SaveSpec};
use crate::ui::theme;
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::tail::{TailLevel, TailWindow};
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

/// Both levels' choices, as the chip and its two surfaces read them.
#[derive(Clone, Copy)]
pub(in crate::workspace::main_area::agent_chat_pane::render) struct TailChoices {
    pub steps: PaneChoice<TailWindow>,
    pub calls: PaneChoice<TailWindow>,
}

impl TailChoices {
    /// Whether the whole axis still follows config — both levels do.
    pub(in crate::workspace::main_area::agent_chat_pane::render) fn is_following(self) -> bool {
        self.steps.is_following() && self.calls.is_following()
    }
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
        s::agent_chat::tail_window_pair(tail_window_value(steps), tail_window_value(calls))
    };
    // The mark is about following config, and an axis is following only when
    // both of its levels are.
    axis_chip_label(s::agent_chat::tail_window_chip(&value), tail.is_following())
}

pub(super) fn tail_window_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TailChoices,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let change = view.clone();
    let reset = view.clone();
    let save_view = view.clone();
    range_editor(
        &format!("agent-chat-{pane_id}"),
        [current.steps.value().size(), current.calls.value().size()],
        theme::agent_chat_font_size(cx),
        Rc::new(move |level, size, _, app| {
            if let Some(view) = change.upgrade() {
                let level = match level {
                    RangeLevel::Steps => TailLevel::Steps,
                    RangeLevel::Calls => TailLevel::Calls,
                };
                view.update(app, |v, cx| {
                    v.set_tail_window(level, TailWindow::last(size), cx)
                });
            }
        }),
        Some(ResetSpec {
            label: s::agent_chat::use_agent_defaults(),
            source: super::fold_mode::pane_source(current.is_following()),
            save: Some(SaveSpec {
                label: s::agent_chat::save_as_agent_default(),
                on_save: Rc::new(move |_window, app| {
                    if let Some(view) = save_view.upgrade() {
                        view.update(app, |v, cx| v.save_tail_window_as_agent_default(cx));
                    }
                }),
            }),
            overridden: !current.is_following(),
            on_reset: Rc::new(move |_, app| {
                if let Some(view) = reset.upgrade() {
                    view.update(app, |v, cx| v.reset_tail_window(cx));
                }
            }),
        }),
        cx,
    )
}

/// The chip's value slot reuses the menu item's own wording, so the chip and
/// the item that set it read alike — and a bare count can't be mistaken for the
/// "N earlier steps" row it sits above.
fn tail_window_value(tail: TailWindow) -> String {
    match tail {
        TailWindow::All => s::agent_chat::tail_window_all(),
        TailWindow::Last(n) => s::agent_chat::tail_window_last(n),
    }
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
                .contains(&s::agent_chat::tail_window_all())
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
        let all = s::agent_chat::tail_window_all();

        // Nothing of its own to say: same window as the steps, or no window.
        assert_eq!(
            seeded(TailWindow::All, TailWindow::All),
            seeded_one(all.clone())
        );
        assert_eq!(
            seeded(TailWindow::Last(3), TailWindow::All),
            seeded_one(s::agent_chat::tail_window_last(3)),
            "a call level that withholds nothing must not restate the steps"
        );
        assert_eq!(
            seeded(TailWindow::Last(2), TailWindow::Last(2)),
            seeded_one(s::agent_chat::tail_window_last(2))
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
        s::agent_chat::tail_window_chip(&value)
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
}
