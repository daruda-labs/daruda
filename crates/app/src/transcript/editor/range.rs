//! The two recent-activity windows, shared by chat and agent defaults.

use std::rc::Rc;

use daruda_config::{TAIL_WINDOW_ALL, TAIL_WINDOW_CHOICES};
use gpui::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*, px};

use super::{ResetSpec, panel_heading, reset_footer, scroll_region};
use crate::surface::strings as s;
use crate::ui::{Selectable as _, button, button_group, theme};

/// The two windows, in panel order. Each host maps it onto its own store.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RangeLevel {
    Steps,
    Calls,
}

impl RangeLevel {
    pub(crate) const ALL: [Self; 2] = [Self::Steps, Self::Calls];

    /// This level's slot in a `[u8; 2]` of window sizes.
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Steps => 0,
            Self::Calls => 1,
        }
    }

    fn heading(self) -> String {
        match self {
            Self::Steps => s::agent_chat::tail_level_steps(),
            Self::Calls => s::agent_chat::tail_level_calls(),
        }
    }
}

pub(crate) type RangeEdit = Rc<dyn Fn(RangeLevel, u8, &mut Window, &mut App)>;

/// `values` holds each level's size at [`RangeLevel::index`].
pub(crate) fn range_editor(
    id: &str,
    values: [u8; 2],
    font_size: f32,
    on_change: RangeEdit,
    reset: Option<ResetSpec>,
    cx: &App,
) -> AnyElement {
    let mut content = scroll_region(SharedString::from(format!("{id}-range-scroll")));
    for level in RangeLevel::ALL {
        let value = values[level.index()];
        let choices = choices(value);
        let on_change = on_change.clone();
        let slot = level.index();
        content = content.child(panel_heading(level.heading(), cx)).child(
            button_group(SharedString::from(format!("{id}-range-{slot}")), cx)
                .w_full()
                .children(choices.iter().map(|&size| {
                    button(
                        SharedString::from(format!("{id}-range-{slot}-{size}")),
                        value_label(size),
                    )
                    .tab_stop(true)
                    .flex_1()
                    .selected(size == value)
                }))
                .on_click(move |indices, window, app| {
                    if let Some(&size) = indices.first().and_then(|&i| choices.get(i)) {
                        on_change(level, size, window, app);
                    }
                }),
        );
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .text_size(px(font_size))
        .child(content)
        .child(reset_footer(
            SharedString::from(format!("{id}-range-reset")),
            reset,
        ))
        .into_any_element()
}

fn choices(current: u8) -> Vec<u8> {
    let mut values = vec![TAIL_WINDOW_ALL];
    values.extend(TAIL_WINDOW_CHOICES);
    if !values.contains(&current) {
        values.push(current);
    }
    values
}

pub(crate) fn value_label(value: u8) -> String {
    if value == TAIL_WINDOW_ALL {
        s::agent_chat::tail_window_all()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_has_its_own_slot() {
        let slots: Vec<_> = RangeLevel::ALL.iter().map(|l| l.index()).collect();
        assert_eq!(slots, [0, 1]);
    }

    #[test]
    fn custom_sizes_are_visible_without_duplicating_standard_choices() {
        for current in [0, 1, 3, 5, 10, 12, u8::MAX] {
            let offered = choices(current);
            assert_eq!(offered.iter().filter(|&&size| size == current).count(), 1);
            assert_eq!(offered[0], TAIL_WINDOW_ALL);
        }
    }
}
