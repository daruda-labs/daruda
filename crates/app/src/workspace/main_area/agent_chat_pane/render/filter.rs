//! Display-filter chip, menu, and filtered-row placeholder.

use gpui::{AnyElement, Context, IntoElement, SharedString, div, prelude::*, px};

use super::fold_header::{FoldHeader, FoldRow};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, Selectable as _, Sizable as _, button_on_surface,
};
use crate::workspace::main_area::agent_chat_pane::display_filter::{
    DisplayFilter, FilterAxis, FilterFacet,
};
use crate::workspace::main_area::agent_chat_pane::fold::FoldKey;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

pub(super) fn display_filter_chip(
    pane_id: PaneId,
    filter: DisplayFilter,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let label = SharedString::from(s::agent_chat_filter_chip(&filter_value(filter)));
    let view = cx.entity().downgrade();
    button_on_surface(
        ("agent-chat-display-filter", pane_id as usize),
        label,
        surface,
        cx,
    )
    .xsmall()
    .selected(!filter.is_empty())
    .tooltip(SharedString::from(s::agent_chat_filter_tooltip()))
    .dropdown_menu(move |menu, _window, _cx| build_filter_menu(&view, filter, menu))
}

/// A bare count would read as "3 things are shown". The fraction says what it
/// really is — 3 of the 11 facets are picked — in the same horizontal budget.
fn filter_value(filter: DisplayFilter) -> String {
    if filter.is_empty() {
        s::agent_chat_filter_none()
    } else {
        s::agent_chat_filter_count(filter.selected_count(), FilterFacet::ALL.len())
    }
}

fn build_filter_menu(
    view: &gpui::WeakEntity<AgentChatView>,
    current: DisplayFilter,
    menu: PopupMenu,
) -> PopupMenu {
    let menu = {
        let view = view.clone();
        // No `checked`: this resets the filter, it is not a twelfth facet the
        // user can leave switched on.
        menu.item(
            PopupMenuItem::new(SharedString::from(s::agent_chat_filter_clear())).on_click(
                move |_, _window, app| {
                    if let Some(view) = view.upgrade() {
                        view.update(app, |v, cx| v.clear_display_filter(cx));
                    }
                },
            ),
        )
    };
    FilterAxis::ALL.into_iter().fold(menu, |menu, axis| {
        let menu = menu.separator().label(SharedString::from(axis_label(axis)));
        FilterFacet::ALL
            .into_iter()
            .filter(|f| f.axis() == axis)
            .fold(menu, |menu, facet| {
                let view = view.clone();
                menu.item(
                    PopupMenuItem::new(SharedString::from(facet_label(facet)))
                        .checked(current.contains(facet))
                        .on_click(move |_, _window, app| {
                            if let Some(view) = view.upgrade() {
                                view.update(app, |v, cx| v.toggle_display_facet(facet, cx));
                            }
                        }),
                )
            })
    })
}

fn axis_label(axis: FilterAxis) -> String {
    match axis {
        FilterAxis::Kind => s::agent_chat_filter_axis_kind(),
        FilterAxis::Tool => s::agent_chat_filter_axis_tool(),
        FilterAxis::Status => s::agent_chat_filter_axis_status(),
    }
}

fn facet_label(facet: FilterFacet) -> String {
    match facet {
        FilterFacet::Thinking => s::agent_chat_filter_thinking(),
        FilterFacet::Prose => s::agent_chat_filter_prose(),
        FilterFacet::Tools => s::agent_chat_filter_tools(),
        FilterFacet::ToolRead => s::agent_chat_filter_tool_read(),
        FilterFacet::ToolEdit => s::agent_chat_filter_tool_edit(),
        FilterFacet::ToolSearch => s::agent_chat_filter_tool_search(),
        FilterFacet::ToolRun => s::agent_chat_filter_tool_run(),
        FilterFacet::ToolOther => s::agent_chat_filter_tool_other(),
        FilterFacet::StatusRunning => s::agent_chat_filter_status_running(),
        FilterFacet::StatusOk => s::agent_chat_filter_status_ok(),
        FilterFacet::StatusFailed => s::agent_chat_filter_status_failed(),
    }
}

/// The placeholder's copy.
///
/// The row is a disclosure, so its number states what clicking it does:
/// `revealable`, the rows this control puts on screen. When a collapsed step or
/// response is holding filtered rows the reveal cannot reach, `excluded` is
/// named too — otherwise the reachable count silently reads as the whole cut
/// (the shipped `1 row hidden` next to a filter that had dropped nineteen).
/// Promising the larger number instead would repeat the tail row's own bug: a
/// label offering a reveal that folding blocks.
fn filtered_away_label(revealable: usize, excluded: usize, collapsed: bool) -> String {
    let held_by_a_fold = excluded > revealable;
    match (collapsed, held_by_a_fold) {
        (true, false) => s::agent_chat_filtered_show(revealable),
        (true, true) => s::agent_chat_filtered_show_partial(revealable, excluded),
        (false, false) => s::agent_chat_filtered_hide(revealable),
        (false, true) => s::agent_chat_filtered_hide_partial(revealable, excluded),
    }
}

pub(super) fn filtered_away_bar(
    this: &AgentChatView,
    run_start: usize,
    revealable: usize,
    excluded: usize,
    collapsed: bool,
    cx: &mut Context<AgentChatView>,
) -> AnyElement {
    let title = div()
        .text_color(this.dim(theme::agent_chat_fg_muted(cx)))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(SharedString::from(filtered_away_label(
            revealable, excluded, collapsed,
        )))
        .into_any_element();
    FoldRow::section(
        SharedString::from(format!("agent-chat-filtered-{run_start}")),
        FoldKey::Filtered(run_start),
        !collapsed,
        FoldHeader::with_title(title),
    )
    .render(this.dim_amount, cx)
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

    #[test]
    fn the_chip_reads_all_until_a_facet_is_picked() {
        assert_eq!(
            filter_value(DisplayFilter::default()),
            s::agent_chat_filter_none()
        );
        let one = DisplayFilter::default().toggled(FilterFacet::Tools);
        let total = FilterFacet::ALL.len();
        assert_eq!(filter_value(one), s::agent_chat_filter_count(1, total));
        assert_eq!(
            filter_value(one.toggled(FilterFacet::ToolEdit)),
            s::agent_chat_filter_count(2, total)
        );
    }

    #[test]
    fn the_chip_counts_selections_not_visible_rows() {
        let two = DisplayFilter::default()
            .toggled(FilterFacet::Tools)
            .toggled(FilterFacet::ToolEdit);
        let shown = filter_value(two);
        assert!(
            shown.contains(&FilterFacet::ALL.len().to_string()),
            "the total must be visible so the count cannot read as a result count: {shown}"
        );
    }
}
