//! The display-filter editor: one checkbox per facet, one section per axis,
//! with a tri-state toggle, a count and a disclosure over each parented one.

use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement, SharedString, div, prelude::*, px};

use super::state::FilterEditorState;
use super::{ResetSpec, TextRoles, editor_column, reset_footer, scroll_content, scroll_region};
use crate::surface::strings as s;
use crate::transcript::display_filter::{DisplayFilter, FilterAxis, FilterFacet, SectionState};
use crate::ui::{ButtonVariants as _, Icon, IconName, button_bare, checkbox, theme};

/// What the editor does with a click. A section toggle is its own action rather
/// than a run of facet toggles: setting a whole section is one move, and the
/// tri-state parent has to be able to say so.
pub(crate) type FilterFacetPress = Rc<dyn Fn(FilterFacet, &mut App)>;
pub(crate) type FilterSectionPress = Rc<dyn Fn(FilterFacet, bool, &mut App)>;
pub(crate) type FilterDisclosePress = Rc<dyn Fn(FilterAxis, &mut App)>;

pub(crate) struct FilterEditorActions {
    pub on_toggle: FilterFacetPress,
    pub on_section: FilterSectionPress,
    /// Opens or shuts a parented section's child rows — presentation only.
    pub on_disclose: FilterDisclosePress,
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
        [] => s::agent_chat::filter_none(),
        [one] => s::agent_chat::filter_hidden(facet_label(*one)),
        [first, second] => s::agent_chat::filter_hidden(format!(
            "{} + {}",
            facet_label(*first),
            facet_label(*second)
        )),
        _ => s::agent_chat::filter_hidden_count(hidden.len()),
    }
}

pub(crate) fn filter_editor(
    current: DisplayFilter,
    state: FilterEditorState,
    id_prefix: &str,
    font_size: f32,
    actions: FilterEditorActions,
    cx: &App,
) -> AnyElement {
    let text = TextRoles::from_base(font_size);
    let t = theme::current(cx);
    let last = FilterAxis::ALL.len() - 1;
    let facets =
        scroll_content().children(FilterAxis::ALL.into_iter().enumerate().map(|(ix, axis)| {
            // Nested rows read a step under their parent; a flat section's rows are
            // its top level.
            let row_size = if axis.parent().is_some() {
                text.aux
            } else {
                text.body
            };
            let rows = axis.rows().into_iter().map(|facet| {
                child_row(
                    filter_checkbox(current, facet, id_prefix, &actions).text_size(px(row_size)),
                )
            });
            let section = div()
                .flex()
                .flex_col()
                .when(ix > 0, |section| section.pt(px(theme::GAP_LG)))
                .when(ix < last, |section| {
                    section
                        .pb(px(theme::GAP_LG))
                        .border_b_1()
                        .border_color(t.border)
                });
            match axis.parent() {
                // A parent toggle owns its rows, so they nest under it.
                Some(parent) => {
                    let open = state.is_open(axis);
                    section
                        .child(parent_row(
                            current, axis, parent, open, id_prefix, text, &actions, cx,
                        ))
                        .when(open, |section| {
                            section.child(
                                div()
                                    .ml(px(theme::TRANSCRIPT_EDITOR_NEST_INDENT))
                                    .grid()
                                    .grid_cols(2)
                                    .gap_x(px(theme::GAP_LG))
                                    .children(rows),
                            )
                        })
                }
                None => section
                    .child(
                        div()
                            .text_size(px(text.aux))
                            .text_color(t.text_muted)
                            .pb(px(theme::GAP_SM))
                            .child(SharedString::from(axis_label(axis))),
                    )
                    .children(rows),
            }
        }));
    editor_column(text)
        .child(scroll_region(
            format!("{id_prefix}-filter-facets-scroll"),
            facets,
        ))
        .child(reset_footer(
            SharedString::from(format!("{id_prefix}-filter-reset")),
            actions.reset,
            text,
            cx,
        ))
        .into_any_element()
}

fn child_row(child: impl IntoElement) -> impl IntoElement {
    div()
        .min_h(px(theme::TRANSCRIPT_EDITOR_CHILD_ROW_MIN_H))
        .flex()
        .items_center()
        .child(child)
}

/// A section's head: its tri-state toggle, how many of its rows are on, and the
/// disclosure for those rows. The three are separate controls, so opening the
/// rows never toggles them.
#[allow(clippy::too_many_arguments)]
fn parent_row(
    current: DisplayFilter,
    axis: FilterAxis,
    parent: FilterFacet,
    open: bool,
    id_prefix: &str,
    text: TextRoles,
    actions: &FilterEditorActions,
    cx: &App,
) -> impl IntoElement + use<> {
    let rows = axis.rows();
    let selected = rows
        .iter()
        .filter(|&&facet| current.contains(facet))
        .count();
    let on_disclose = actions.on_disclose.clone();
    div()
        .min_h(px(theme::TRANSCRIPT_EDITOR_PARENT_ROW_MIN_H))
        .flex()
        .items_center()
        .gap(px(theme::GAP_LG))
        .child(
            div().flex_1().min_w_0().child(
                parent_checkbox(current, parent, id_prefix, actions).text_size(px(text.body)),
            ),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(text.aux))
                .text_color(theme::current(cx).text_muted)
                .child(SharedString::from(s::agent_chat::filter_section_count(
                    selected,
                    rows.len(),
                ))),
        )
        .child(
            button_bare(SharedString::from(format!(
                "{id_prefix}-filter-disclose-{}",
                parent.token()
            )))
            .ghost()
            .tab_stop(true)
            .icon(Icon::new(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }))
            .tooltip(SharedString::from(if open {
                s::agent_chat::filter_section_hide()
            } else {
                s::agent_chat::filter_section_show()
            }))
            .on_click(move |_, _, app| on_disclose(axis, app)),
        )
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
) -> crate::ui::Checkbox {
    let state = current.section_state(parent);
    let on_section = actions.on_section.clone();
    checkbox(
        SharedString::from(format!("{id_prefix}-filter-{}", parent.token())),
        facet_label(parent),
        0,
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
) -> crate::ui::Checkbox {
    let on_toggle = actions.on_toggle.clone();
    checkbox(
        SharedString::from(format!("{id_prefix}-filter-{}", facet.token())),
        facet_label(facet),
        0,
    )
    .checked(current.contains(facet))
    .on_click(move |_, _window, app| on_toggle(facet, app))
}

fn axis_label(axis: FilterAxis) -> String {
    match axis {
        FilterAxis::Kind => s::agent_chat::filter_axis_kind(),
        FilterAxis::Reply => s::agent_chat::filter_axis_reply(),
        FilterAxis::Tool => s::agent_chat::filter_axis_tool(),
    }
}

fn facet_label(facet: FilterFacet) -> String {
    match facet {
        FilterFacet::Thinking => s::agent_chat::filter_thinking(),
        FilterFacet::Prose => s::agent_chat::filter_prose(),
        FilterFacet::ProseAnswer => s::agent_chat::filter_prose_answer(),
        FilterFacet::ProsePreamble => s::agent_chat::filter_prose_preamble(),
        FilterFacet::Tools => s::agent_chat::filter_tools(),
        FilterFacet::ToolRead => s::agent_chat::filter_tool_read(),
        FilterFacet::ToolEdit => s::agent_chat::filter_tool_edit(),
        FilterFacet::ToolDelete => s::agent_chat::filter_tool_delete(),
        FilterFacet::ToolSearch => s::agent_chat::filter_tool_search(),
        FilterFacet::ToolRun => s::agent_chat::filter_tool_run(),
        FilterFacet::ToolFetch => s::agent_chat::filter_tool_fetch(),
        FilterFacet::ToolMcp => s::agent_chat::filter_tool_mcp(),
        FilterFacet::ToolAgent => s::agent_chat::filter_tool_agent(),
        FilterFacet::ToolOther => s::agent_chat::filter_tool_other(),
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
            s::agent_chat::filter_none()
        );
        let one = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
        assert_eq!(
            filter_value(one),
            s::agent_chat::filter_hidden(facet_label(FilterFacet::ToolEdit))
        );
        assert_eq!(
            filter_value(one.toggled(FilterFacet::Thinking)),
            s::agent_chat::filter_hidden(format!(
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
        assert_eq!(filter_value(three), s::agent_chat::filter_hidden_count(3));
    }
}
