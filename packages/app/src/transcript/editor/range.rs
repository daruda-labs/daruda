//! The two recent-activity windows, shared by chat and agent defaults.

use std::rc::Rc;

use daruda_config::{TAIL_WINDOW_ALL, TAIL_WINDOW_CHOICES};
use gpui::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*, px};

use super::{
    ResetSpec, TextRoles, aux_icon, editor_column, reset_footer, scroll_content, scroll_region,
};
use crate::surface::strings as s;
use crate::ui::{ButtonVariants as _, Selectable as _, button_bare, choice_variant, icons, theme};

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

    fn icon(self) -> &'static str {
        match self {
            Self::Steps => icons::TASKS,
            Self::Calls => icons::BUILD,
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
    let text = TextRoles::from_base(font_size);
    let t = theme::current(cx);
    let content =
        scroll_content().children(RangeLevel::ALL.into_iter().enumerate().map(|(ix, level)| {
            let value = values[level.index()];
            let slot = level.index();
            let on_change = on_change.clone();
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_LG))
                .when(ix > 0, |section| {
                    section
                        .mt(px(theme::TRANSCRIPT_EDITOR_SECTION_GAP))
                        .pt(px(theme::TRANSCRIPT_EDITOR_SECTION_GAP))
                        .border_t_1()
                        .border_color(t.border)
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::GAP_LG))
                        .text_color(t.text_body)
                        .child(aux_icon(level.icon(), t.text_muted))
                        .child(SharedString::from(level.heading())),
                )
                .child(
                    // Apart rather than joined, and wrapping rather than
                    // shrinking: a custom sixth size takes a line of its own
                    // before any label gets squeezed.
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .gap(px(theme::TRANSCRIPT_EDITOR_CHOICE_GAP))
                        .children(choices(value).into_iter().map(|size| {
                            let on_change = on_change.clone();
                            button_bare(SharedString::from(format!("{id}-range-{slot}-{size}")))
                                .custom(choice_variant(cx))
                                .tab_stop(true)
                                .flex_1()
                                .min_w(px(theme::TRANSCRIPT_EDITOR_CHOICE_MIN_W))
                                .h_auto()
                                .min_h(px(theme::TRANSCRIPT_EDITOR_CHOICE_MIN_H))
                                .selected(size == value)
                                .child(
                                    div()
                                        .text_size(px(text.body))
                                        .child(SharedString::from(value_label(size))),
                                )
                                .on_click(move |_, window, app| on_change(level, size, window, app))
                        })),
                )
        }));
    editor_column(text)
        .child(scroll_region(format!("{id}-range-scroll"), content))
        .child(reset_footer(
            SharedString::from(format!("{id}-range-reset")),
            reset,
            text,
            cx,
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
