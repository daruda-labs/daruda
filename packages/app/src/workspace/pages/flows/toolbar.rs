//! Search, grouping and recovery actions for the two flow lists.

use crate::surface::strings as s;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenuItem, button, icons, list_page, menu_builder,
    theme,
};
use crate::workspace::{
    flow_browser::FlowPageSnapshot,
    flow_browser::{FlowGrouping, FlowTab, RunFilter},
    flow_paths::FlowOrigin,
};
use gpui::{AnyElement, App, IntoElement, div, prelude::*, px};

pub(super) fn search(snap: &FlowPageSnapshot, cx: &App) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    list_page::toolbar()
        .child(div().flex_1().min_w_0().child(list_page::search(
            "flow-search-clear",
            s::common::search_clear().into(),
            &snap.flow_browser.search,
            !snap.flow_browser.query.is_empty(),
            move |window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.clear_flow_search(window, cx));
                }
            },
            cx,
        )))
        .when(snap.flow_browser.state.tab == FlowTab::Definitions, |row| {
            row.child(grouping_picker(snap))
        })
}

fn grouping_label(grouping: FlowGrouping) -> String {
    match grouping {
        FlowGrouping::None => s::common::group_none(),
        FlowGrouping::Origin => s::flow::column_origin(),
    }
}

fn grouping_picker(snap: &FlowPageSnapshot) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let selected = snap.flow_browser.state.grouping;
    button(
        "flow-grouping",
        s::common::group_by(grouping_label(selected)),
    )
    .tab_stop(true)
    .flex_none()
    .child(icons::icon(icons::EXPAND_MORE))
    .dropdown_menu(menu_builder(move |menu, _, _| {
        [FlowGrouping::None, FlowGrouping::Origin]
            .into_iter()
            .fold(menu, |menu, grouping| {
                let workspace = workspace.clone();
                menu.item(
                    PopupMenuItem::new(grouping_label(grouping))
                        .checked(grouping == selected)
                        .on_click(move |_, _, cx| {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(cx, |ws, cx| ws.set_flow_grouping(grouping, cx));
                            }
                        }),
                )
            })
    }))
}

pub(super) fn group_header(
    origin: FlowOrigin,
    count: usize,
    snap: &FlowPageSnapshot,
    cx: &App,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let open = snap.flow_browser.state.is_open(origin);
    let id = format!("flow-group-{origin:?}");
    list_page::group_header(
        gpui::SharedString::from(id.clone()),
        gpui::SharedString::from(format!("flow-group-chevron-{origin:?}")),
        s::common::filter_count(super::controls::origin_label(Some(origin)), count),
        open,
        cx,
    )
    .debug_selector(move || id)
    .on_click(move |_, _, cx| {
        if let Some(ws) = workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.toggle_flow_group(origin, cx));
        }
    })
}

pub(super) fn results(
    snap: &FlowPageSnapshot,
    visible: usize,
    total: usize,
    cx: &App,
) -> impl IntoElement {
    let state = &snap.flow_browser.state;
    let filtered = !snap.flow_browser.query.trim().is_empty()
        || match state.tab {
            FlowTab::Definitions => state.origin.is_some(),
            FlowTab::Runs => state.run_filter != RunFilter::All,
        };
    let workspace = snap.workspace.clone();
    let clear = filtered.then(|| {
        button("flow-clear-filters", s::flow::clear_filters())
            .ghost()
            .tab_stop(true)
            .debug_selector(|| "flow-clear-filters".into())
            .on_click(move |_, window, cx| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.clear_flow_filters(window, cx));
                }
            })
            .into_any_element()
    });
    list_page::results(s::flow::result_count(visible, total), clear, cx)
}

pub(super) fn empty(snap: &FlowPageSnapshot, total: usize, cx: &App) -> AnyElement {
    let message = if !snap
        .flow_browser
        .targets
        .iter()
        .any(|target| target.lane == snap.flow_lane)
    {
        s::flow::empty_unavailable()
    } else if total > 0 {
        s::flow::empty_filtered()
    } else {
        match snap.flow_browser.state.tab {
            FlowTab::Definitions => s::flow::panel_flows_empty(),
            FlowTab::Runs => s::flow::panel_past_empty(),
        }
    };
    list_page::empty(message, cx).into_any_element()
}

pub(super) fn attention(snap: &FlowPageSnapshot, count: usize, cx: &App) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(theme::GAP_SM))
        .p(px(theme::PAD_STANDARD))
        .border_1()
        .border_color(theme::current(cx).border)
        .rounded(px(theme::RADIUS_SM))
        .text_color(theme::current(cx).text_body)
        .child(s::flow::pending_questions(count))
        .child(
            button("flow-show-questions", s::flow::show_questions())
                .tab_stop(true)
                .debug_selector(|| "flow-show-questions".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.show_flow_questions(window, cx));
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouping_labels_describe_sources_not_execution_state() {
        assert_eq!(
            grouping_label(FlowGrouping::Origin),
            s::flow::column_origin()
        );
    }
}
