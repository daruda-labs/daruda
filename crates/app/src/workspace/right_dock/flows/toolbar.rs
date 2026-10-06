//! Search, grouping and recovery actions for the two flow lists.

use crate::surface::strings as s;
use crate::ui::{
    ButtonVariants as _, DropdownMenu as _, PopupMenuItem, button, icons, menu_builder, theme,
};
use crate::workspace::{
    flow_browser::{FlowGrouping, FlowTab, RunFilter},
    flow_paths::FlowOrigin,
    layout::RightDockSnapshot,
};
use gpui::{AnyElement, App, IntoElement, MouseButton, div, prelude::*, px};

pub(super) fn search(snap: &RightDockSnapshot, cx: &App) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_STANDARD))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .relative()
                .child(crate::ui::input(&snap.flow_browser.search, cx, ()))
                .when(!snap.flow_browser.query.is_empty(), |row| {
                    row.child(
                        crate::ui::button_icon("flow-search-clear", icons::CLOSE, cx)
                            .tooltip(s::common::search_clear())
                            .absolute()
                            .right(px(theme::PAD_XS))
                            .top_0()
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                if let Some(ws) = workspace.upgrade() {
                                    ws.update(cx, |ws, cx| ws.clear_flow_search(window, cx));
                                }
                            }),
                    )
                }),
        )
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

fn grouping_picker(snap: &RightDockSnapshot) -> impl IntoElement {
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
    snap: &RightDockSnapshot,
    cx: &App,
) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let open = snap.flow_browser.state.is_open(origin);
    let id = format!("flow-group-{origin:?}");
    crate::ui::button_bare(gpui::SharedString::from(id.clone()))
        .w_full()
        .justify_start()
        .tab_stop(true)
        .debug_selector(move || id)
        .py(px(theme::PAD_STANDARD))
        .text_color(theme::current(cx).text_muted)
        .child(icons::icon(if open {
            icons::EXPAND_MORE
        } else {
            icons::CHEVRON_RIGHT
        }))
        .child(s::common::filter_count(
            super::controls::origin_label(Some(origin)),
            count,
        ))
        .on_click(move |_, _, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.toggle_flow_group(origin, cx));
            }
        })
}

pub(super) fn results(
    snap: &RightDockSnapshot,
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
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .child(s::flow::result_count(visible, total))
        .when(filtered, |row| {
            row.child(
                button("flow-clear-filters", s::flow::clear_filters())
                    .ghost()
                    .tab_stop(true)
                    .debug_selector(|| "flow-clear-filters".into())
                    .on_click(move |_, window, cx| {
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.clear_flow_filters(window, cx));
                        }
                    }),
            )
        })
}

pub(super) fn empty(snap: &RightDockSnapshot, total: usize, cx: &App) -> AnyElement {
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
    div()
        .py(px(theme::DOCK_PAGE_PAD))
        .text_color(theme::current(cx).text_muted)
        .child(message)
        .into_any_element()
}

pub(super) fn attention(snap: &RightDockSnapshot, count: usize, cx: &App) -> impl IntoElement {
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
