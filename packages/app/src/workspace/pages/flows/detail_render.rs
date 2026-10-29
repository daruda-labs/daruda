//! Draws the Flows page's detail in place of its list.

use gpui::{
    AnyElement, AnyView, Context, MouseButton, MouseDownEvent, StyleRefinement, div, prelude::*, px,
};

use super::detail::{FlowDetail, FlowDetailBody, FlowDetailId, RunDetail, RunReport};
use crate::surface::strings as s;
use crate::ui::{button_icon, icons, theme};
use crate::workspace::Workspace;
use crate::workspace::pages::PageState;

pub(in crate::workspace) fn render(
    detail: &FlowDetail,
    _page: &PageState,
    session_problem: Option<String>,
    layout: super::execution_layout::ExecutionLayout,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let id = detail.id;
    match &detail.body {
        // Full height and outside the list's scroll frame: the canvas pans
        // itself. Cached because a run reports node by node, and this keeps
        // those repaints in the view's own subtree (render-cost rule 10). The
        // view tracks its own focus handle.
        FlowDetailBody::Graph(view) => div()
            .id(("flow-graph-detail", id.0 as usize))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |ws, ev: &MouseDownEvent, window, cx| {
                    ws.open_flow_graph_menu_at(id, ev.position, window, cx)
                }),
            )
            .child(
                AnyView::from(view.clone()).cached(StyleRefinement::default().size_full().flex()),
            )
            .into_any_element(),
        FlowDetailBody::Run(run) => div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::current(cx).dock_bg)
            .child(run_header(id, run, cx))
            .child(super::execution_render::body(
                id,
                detail.lane,
                run,
                layout,
                session_problem,
                cx,
            ))
            .into_any_element(),
    }
}

fn run_header(id: FlowDetailId, run: &RunDetail, cx: &mut Context<Workspace>) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_SM))
        .px(px(theme::DOCK_PAGE_PAD))
        .py(px(theme::GAP_SM))
        .child(
            button_icon("flow-run-back", icons::BACK, cx)
                .tooltip(
                    if run.execution.content == super::execution_detail::ExecutionContent::Report {
                        s::flow::run_return_to_execution()
                    } else {
                        s::flow::back_to_list()
                    },
                )
                .debug_selector(|| "flow-run-back".into())
                .on_click(cx.listener(|ws, _, window, cx| ws.back_from_execution(window, cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(t.text_primary)
                .child(
                    div().child(match &run.execution.inspection {
                        super::execution_detail::Inspection::Ready(h) => h
                            .source
                            .as_ref()
                            .and_then(|p| p.file_stem())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(s::flow::run_history_title),
                        _ => s::flow::run_history_title(),
                    }),
                )
                .child(
                    div()
                        .text_size(px(theme::FONT_SIZE_SM))
                        .text_color(t.text_muted)
                        .child(s::flow::run_history_title()),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(t.text_muted)
                .child(match &run.execution.inspection {
                    super::execution_detail::Inspection::Ready(history) => {
                        super::execution_render::presentation::started(history.started_at)
                    }
                    _ => run.started.to_string(),
                }),
        )
        .child(
            button_icon("flow-execution-refresh", icons::REFRESH, cx)
                .tooltip(s::flow::run_refresh())
                .on_click(cx.listener(move |ws, _, _, cx| ws.refresh_execution(id, cx))),
        )
        .child(
            button_icon("flow-run-open-file", icons::SKILL, cx)
                .tooltip(s::flow::run_report_title())
                .on_click(cx.listener(move |ws, _, _, cx| ws.show_execution_report(id, cx))),
        )
        .into_any_element()
}

pub(super) fn run_body(id: FlowDetailId, run: &RunDetail, cx: &gpui::App) -> AnyElement {
    let muted = theme::current(cx).text_muted;
    match &run.report {
        RunReport::Loading => div()
            .text_color(muted)
            .child(s::common::loading())
            .into_any_element(),
        RunReport::Missing => div()
            .text_color(muted)
            .child(s::flow::run_report_missing())
            .into_any_element(),
        RunReport::Loaded(text) => {
            crate::ui::markdown(("flow-run-report", id.0 as usize), text.clone())
                .text_size(px(theme::editor_font_size(cx)))
                .selectable(true)
                .full_width(true)
                .into_any_element()
        }
    }
}
