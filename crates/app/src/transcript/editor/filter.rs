//! The display-filter editor: one checkbox per facet, grouped by axis, with a
//! tri-state toggle over each parented section.

use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px};

use super::{ResetSpec, panel_heading, reset_footer, scroll_region};
use crate::surface::strings as s;
use crate::transcript::display_filter::{DisplayFilter, FilterAxis, FilterFacet, SectionState};
use crate::ui::checkbox;
use crate::ui::theme;

/// What the editor does with a click. A section toggle is its own action rather
/// than a run of facet toggles: setting a whole section is one move, and the
/// tri-state parent has to be able to say so.
pub(crate) type FilterFacetPress = Rc<dyn Fn(FilterFacet, &mut App)>;
pub(crate) type FilterSectionPress = Rc<dyn Fn(FilterFacet, bool, &mut App)>;

pub(crate) struct FilterEditorActions {
    pub on_toggle: FilterFacetPress,
    pub on_section: FilterSectionPress,
    pub reset: Option<ResetSpec>,
}

/// The value text for a filter — what a chip or a settings row shows without
/// opening the editor.
///
/// It names what is **missing**, not what is left: it is the shorter list, and
/// it is what the user did — every box starts checked, so a filtered pane is
/// one the user took something out of.
pub(crate) fn filter_value(filter: DisplayFilter) -> String {
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

pub(crate) fn filter_editor(
    current: DisplayFilter,
    id_prefix: &str,
    font_size: f32,
    actions: FilterEditorActions,
    cx: &App,
) -> AnyElement {
    let mut facets = scroll_region(SharedString::from(format!(
        "{id_prefix}-filter-facets-scroll"
    )));
    for axis in FilterAxis::ALL {
        facets = facets.child(panel_heading(axis_label(axis), cx));
        let rows = axis
            .rows()
            .into_iter()
            .map(|facet| filter_checkbox(current, facet, id_prefix, &actions));
        match axis.parent() {
            // A parent toggle owns its rows, so they nest under it.
            Some(parent) => {
                facets = facets
                    .child(parent_checkbox(current, parent, id_prefix, &actions))
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
        .text_size(px(font_size))
        .child(facets)
        .child(reset_footer(
            SharedString::from(format!("{id_prefix}-filter-reset")),
            s::agent_chat_filter_reset_default(),
            actions.reset,
        ))
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
    current: DisplayFilter,
    parent: FilterFacet,
    id_prefix: &str,
    actions: &FilterEditorActions,
) -> impl IntoElement + use<> {
    let state = current.section_state(parent);
    let on_section = actions.on_section.clone();
    checkbox(
        SharedString::from(format!("{id_prefix}-filter-{}", parent.token())),
        facet_label(parent),
        (),
    )
    .checked(state == SectionState::On)
    .indeterminate(state == SectionState::Partial)
    .on_click(move |selected, _window, app| on_section(parent, *selected, app))
}

fn filter_checkbox(
    current: DisplayFilter,
    facet: FilterFacet,
    id_prefix: &str,
    actions: &FilterEditorActions,
) -> impl IntoElement + use<> {
    let on_toggle = actions.on_toggle.clone();
    checkbox(
        SharedString::from(format!("{id_prefix}-filter-{}", facet.token())),
        facet_label(facet),
        (),
    )
    .checked(current.contains(facet))
    .on_click(move |_, _window, app| on_toggle(facet, app))
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

    /// Nothing hidden reads `All`; from there the value text names what the
    /// user took out, in panel order rather than click order.
    #[test]
    fn the_value_text_reads_all_until_a_kind_is_hidden() {
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

    #[test]
    fn three_or_more_hidden_kinds_use_a_semantic_count() {
        let three = DisplayFilter::default()
            .toggled(FilterFacet::Thinking)
            .toggled(FilterFacet::Prose)
            .toggled(FilterFacet::ToolEdit);
        assert_eq!(filter_value(three), s::agent_chat_filter_hidden_count(3));
    }
}
