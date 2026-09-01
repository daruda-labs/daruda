//! Display-filter chip, persistent popover, and filtered-row placeholder.

use gpui::{Anchor, AnyElement, App, Context, IntoElement, SharedString, div, prelude::*, px};

use super::options_panel::{axis_chip_label, fixed_region, panel_root, scroll_region};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    ButtonVariants as _, Disableable as _, Popover, Selectable as _, Sizable as _, button,
    button_chip_on_surface, checkbox,
};
use crate::workspace::main_area::agent_chat_pane::display_filter::{
    DisplayFilter, FilterAxis, FilterFacet, SectionState,
};
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::rows::FilteredAway;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

pub(super) fn display_filter_chip(
    pane_id: PaneId,
    filter: PaneChoice<DisplayFilter>,
    default_open: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let view = cx.entity().downgrade();
    Popover::new(SharedString::from(format!(
        "agent-chat-display-filter-popover-{pane_id}"
    )))
    .default_open(default_open)
    .anchor(Anchor::TopRight)
    .trigger(
        button_chip_on_surface(
            ("agent-chat-display-filter", pane_id as usize),
            SharedString::from(display_filter_chip_label(filter)),
            surface,
            cx,
        )
        .selected(!filter.is_following())
        .tooltip(SharedString::from(s::agent_chat_filter_tooltip())),
    )
    .content(move |_, window, cx| {
        panel_root(theme::AGENT_CHAT_OPTIONS_PANEL_W, window)
            .child(filter_panel(&view, filter, pane_id, cx))
            .into_any_element()
    })
}

/// The chip's full text, overridden mark included. Also the filter axis's slot
/// in the compact bar's tooltip, so the two readings of the same setting
/// cannot diverge.
pub(super) fn display_filter_chip_label(filter: PaneChoice<DisplayFilter>) -> String {
    axis_chip_label(
        s::agent_chat_filter_chip(&filter_value(filter.value())),
        filter,
    )
}

/// The chip names what is **missing**, not what is left: it is the shorter
/// list, and it is what the user did — every box starts checked, so a filtered
/// pane is one the user took something out of.
fn filter_value(filter: DisplayFilter) -> String {
    let hidden = filter.hidden();
    match hidden.as_slice() {
        [] => s::agent_chat_filter_none(),
        [one] => s::agent_chat_filter_hidden(&facet_label(*one)),
        [first, second] => s::agent_chat_filter_hidden(&format!(
            "{} + {}",
            facet_label(*first),
            facet_label(*second)
        )),
        _ => s::agent_chat_filter_hidden_count(hidden.len()),
    }
}

pub(super) fn filter_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    choice: PaneChoice<DisplayFilter>,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let current = choice.value();
    let reset_view = view.clone();
    let mut facets = scroll_region(SharedString::from(format!(
        "agent-chat-filter-facets-scroll-{pane_id}"
    )));
    for axis in FilterAxis::ALL {
        facets = facets.child(panel_heading(axis_label(axis), cx));
        let rows = axis
            .rows()
            .into_iter()
            .map(|facet| filter_checkbox(view, current, facet, pane_id));
        match axis.parent() {
            // A parent toggle owns its rows, so they nest under it.
            Some(parent) => {
                facets = facets
                    .child(parent_checkbox(view, current, parent, pane_id))
                    .child(
                        div()
                            .ml(px(theme::AGENT_CHAT_OPTION_NEST_INDENT))
                            .flex()
                            .flex_col()
                            .gap(px(theme::GAP_SM))
                            .children(rows),
                    );
            }
            None => facets = facets.children(rows),
        }
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(facets)
        .child(
            fixed_region().child(
                button(
                    SharedString::from(format!("agent-chat-filter-reset-{pane_id}")),
                    s::agent_chat_filter_reset_default(),
                )
                .ghost()
                .xsmall()
                // Offered on a value that already equals the default: what the
                // button undoes is the *override*, not the value.
                .disabled(choice.is_following())
                .on_click(move |_, _window, app| {
                    if let Some(view) = reset_view.upgrade() {
                        view.update(app, |v, cx| v.reset_display_filter(cx));
                    }
                }),
            ),
        )
        .into_any_element()
}

/// A parented section's own toggle. Tri-state: checked when every row under it
/// is on, indeterminate on a partial set — clicking it sets the whole section.
///
/// Reads the state through `section_state(parent)` rather than one axis's
/// selection, so each parented axis drives its own filter. `parent` comes from
/// the axis table, so the id and label cannot drift from the section that holds
/// them.
fn parent_checkbox(
    view: &gpui::WeakEntity<AgentChatView>,
    current: DisplayFilter,
    parent: FilterFacet,
    pane_id: PaneId,
) -> impl IntoElement + use<> {
    let state = current.section_state(parent);
    let view = view.clone();
    checkbox(
        SharedString::from(format!("agent-chat-filter-{}-{pane_id}", parent.token())),
        facet_label(parent),
        (),
    )
    .checked(state == SectionState::On)
    .indeterminate(state == SectionState::Partial)
    .on_click(move |selected, _window, app| {
        if let Some(view) = view.upgrade() {
            view.update(app, |v, cx| v.set_filter_section(parent, *selected, cx));
        }
    })
}

fn filter_checkbox(
    view: &gpui::WeakEntity<AgentChatView>,
    current: DisplayFilter,
    facet: FilterFacet,
    pane_id: PaneId,
) -> impl IntoElement + use<> {
    let view = view.clone();
    checkbox(
        SharedString::from(format!("agent-chat-filter-{}-{pane_id}", facet.token())),
        facet_label(facet),
        (),
    )
    .checked(current.contains(facet))
    .on_click(move |_, _window, app| {
        if let Some(view) = view.upgrade() {
            view.update(app, |v, cx| v.toggle_display_facet(facet, cx));
        }
    })
}

fn panel_heading(label: String, cx: &App) -> impl IntoElement {
    div()
        .text_color(theme::current(cx).text_subtle)
        .child(SharedString::from(label))
}

fn axis_label(axis: FilterAxis) -> String {
    match axis {
        FilterAxis::Kind => s::agent_chat_filter_axis_kind(),
        FilterAxis::Reply => s::agent_chat_filter_axis_reply(),
        FilterAxis::Tool => s::agent_chat_filter_axis_tool(),
    }
}

fn facet_label(facet: FilterFacet) -> String {
    match facet {
        FilterFacet::Thinking => s::agent_chat_filter_thinking(),
        FilterFacet::Prose => s::agent_chat_filter_prose(),
        FilterFacet::ProseAnswer => s::agent_chat_filter_prose_answer(),
        FilterFacet::ProsePreamble => s::agent_chat_filter_prose_preamble(),
        FilterFacet::Tools => s::agent_chat_filter_tools(),
        FilterFacet::ToolRead => s::agent_chat_filter_tool_read(),
        FilterFacet::ToolEdit => s::agent_chat_filter_tool_edit(),
        FilterFacet::ToolSearch => s::agent_chat_filter_tool_search(),
        FilterFacet::ToolRun => s::agent_chat_filter_tool_run(),
        FilterFacet::ToolOther => s::agent_chat_filter_tool_other(),
    }
}

/// The reveal control's copy.
///
/// Collapsed, the number states exactly what clicking does — nothing more. A
/// count that also named what a fold held back read as "1 now, 6 on the next
/// click" when the other six were somewhere this control cannot reach.
///
/// Revealed, there is no number at all. Those rows are on screen, so a count
/// restates them, and `Hide 12 filtered rows` is readable as a description of
/// the current state precisely when the state is the opposite.
fn filtered_chip_label(filtered: FilteredAway, revealed: bool) -> String {
    if revealed {
        return s::agent_chat_filtered_hide_again();
    }
    s::agent_chat_filtered_show(filtered.revealable)
}

/// The filter's reveal control, riding the response bar's trailing slot.
///
/// The tally is one per run and the bar is the run's one header, so this is
/// where it belongs. As a row of its own it sat at the group bars' indent and
/// wore their chevron while carrying no count — it read as one more group.
pub(super) fn filtered_chip(
    run_start: usize,
    filtered: FilteredAway,
    revealed: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    button_chip_on_surface(
        ("agent-chat-filtered", run_start),
        SharedString::from(filtered_chip_label(filtered, revealed)),
        surface,
        cx,
    )
    // The row's own gap is tuned for text meeting text; a bordered chip needs
    // its own breathing room or it reads as fused to the count beside it.
    .mr(px(theme::GAP_STANDARD))
    .selected(revealed)
    .on_click(cx.listener(move |this, _ev, window, cx| {
        // The bar itself toggles the response fold, so the chip has to keep its
        // own click from reaching it.
        cx.stop_propagation();
        this.toggle_fold(FoldKey::Filtered(run_start), window, cx);
    }))
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_facet_and_axis_has_a_label() {
        for facet in FilterFacet::ALL {
            assert!(!facet_label(facet).is_empty(), "{facet:?}");
        }
        for axis in FilterAxis::ALL {
            assert!(!axis_label(axis).is_empty(), "{axis:?}");
        }
    }

    /// Nothing hidden reads `All`; from there the chip names what the user took
    /// out, in panel order rather than click order.
    #[test]
    fn the_chip_reads_all_until_a_kind_is_hidden() {
        assert_eq!(
            filter_value(DisplayFilter::default()),
            s::agent_chat_filter_none()
        );
        let one = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
        assert_eq!(
            filter_value(one),
            s::agent_chat_filter_hidden(&facet_label(FilterFacet::ToolEdit))
        );
        assert_eq!(
            filter_value(one.toggled(FilterFacet::Thinking)),
            s::agent_chat_filter_hidden(&format!(
                "{} + {}",
                facet_label(FilterFacet::Thinking),
                facet_label(FilterFacet::ToolEdit)
            ))
        );
    }

    fn cut(revealable: usize) -> FilteredAway {
        FilteredAway { revealable }
    }

    /// Unrevealed the chip promises a count; revealed it promises none, and the
    /// revealed string does not vary with either number. That invariance is the
    /// point: a count beside `Hide` reads as "still hidden" exactly when the
    /// rows are on screen.
    #[test]
    fn the_revealed_label_carries_no_count_at_all() {
        let revealed: Vec<String> = [12, 1, 30]
            .into_iter()
            .map(|r| filtered_chip_label(cut(r), true))
            .collect();
        assert!(
            revealed.iter().all(|l| l == &revealed[0]),
            "one string for every count: {revealed:?}"
        );
        assert!(
            !revealed[0].chars().any(char::is_numeric),
            "no digits in the revealed label: {}",
            revealed[0]
        );
    }

    /// The unrevealed half names what clicking reveals, and nothing else. The
    /// number is the reachable count, so the same count reads the same way no
    /// matter how much the filter took from inside a fold.
    #[test]
    fn the_unrevealed_label_names_only_what_clicking_reveals() {
        let one = filtered_chip_label(cut(1), false);
        assert!(one.contains('1'), "{one}");
        let many = filtered_chip_label(cut(12), false);
        assert!(many.contains("12"), "{many}");
        assert_ne!(one, many, "singular and plural differ");
        // No second number to disagree with the first.
        assert_eq!(
            many.matches(char::is_numeric).count(),
            2,
            "only the reachable count appears: {many}"
        );
    }

    #[test]
    fn three_or_more_hidden_kinds_use_a_semantic_count() {
        let three = DisplayFilter::default()
            .toggled(FilterFacet::Thinking)
            .toggled(FilterFacet::Prose)
            .toggled(FilterFacet::ToolEdit);
        assert_eq!(filter_value(three), s::agent_chat_filter_hidden_count(3));
    }
}
