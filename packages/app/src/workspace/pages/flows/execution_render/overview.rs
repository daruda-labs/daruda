//! Read-only graph summary and node results.

use super::*;
use gpui::{AnyView, StyleRefinement};

pub(super) fn render(
    id: FlowDetailId,
    run: &RunDetail,
    history: &RunHistory,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let nodes = nodes(history);
    let attempts: usize = nodes.iter().map(|n| history.attempts(n).len()).sum();
    let completed = nodes
        .iter()
        .filter(|node| {
            history.state().is_some_and(|state| {
                state.node_states().any(|(id, state)| {
                    id == *node
                        && !state.is_invalidated()
                        && matches!(
                            state.last(),
                            daruda_flow::run_log::machine::AttemptState::Passed { .. }
                                | daruda_flow::run_log::machine::AttemptState::Pinned
                        )
                })
            })
        })
        .count();
    let title = history
        .source
        .as_ref()
        .and_then(|p| p.file_stem())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(s::flow::run_history_title);
    let status = run
        .execution
        .rail
        .iter()
        .find(|r| r.entry.dir == run.dir)
        .map(|r| r.entry.status)
        .unwrap_or(daruda_flow::marker::RunStatus::Unknown);
    let summary = div()
        .p(px(theme::DOCK_PAGE_PAD))
        .border_b_1()
        .border_color(t.border)
        .child(
            div()
                .flex()
                .justify_between()
                .gap(px(theme::GAP_LG))
                .child(div().min_w_0().truncate().child(title))
                .child(presentation::run_badge(status, cx)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(theme::DOCK_PAGE_PAD))
                .py(px(theme::PAD_XL))
                .child(metric(
                    s::flow::run_nodes(),
                    if nodes.is_empty() || history.state().is_none() {
                        s::flow::run_not_recorded()
                    } else {
                        format!("{completed}/{}", nodes.len())
                    },
                    cx,
                ))
                .child(metric(
                    s::flow::run_elapsed(),
                    presentation::duration(history.elapsed),
                    cx,
                ))
                .child(metric(
                    s::flow::run_cost(),
                    if history.spent.cost_mixed {
                        s::flow::run_mixed_cost()
                    } else {
                        history
                            .spent
                            .cost
                            .as_ref()
                            .map(|cost| format!("{:.4} {}", cost.amount, cost.currency))
                            .unwrap_or_else(s::flow::run_not_recorded)
                    },
                    cx,
                ))
                .child(metric(
                    s::flow::run_attempts(),
                    if nodes.is_empty() {
                        s::flow::run_not_recorded()
                    } else {
                        attempts.to_string()
                    },
                    cx,
                )),
        );
    let graph = div()
        .h(px(palette::FLOW_RUN_GRAPH_H))
        .flex_none()
        .overflow_hidden()
        .cursor(gpui::CursorStyle::OpenHand)
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .when_some(run.execution.graph.as_ref(), |d, graph| {
            d.child(
                AnyView::from(graph.canvas.clone())
                    .cached(StyleRefinement::default().size_full().flex()),
            )
        })
        .when(history.flow.is_none(), |d| {
            d.p(px(theme::PAD_XL))
                .child(s::flow::run_snapshot_missing())
        });
    let mut rows = div().flex().flex_col();
    for node in nodes {
        let node_id = node.clone();
        let attempts = history.attempts(&node);
        let selected = run
            .execution
            .selection
            .as_ref()
            .is_some_and(|selection| selection.node() == &node);
        let result = attempts
            .last()
            .and_then(|(_, e)| history.attempt(&node, *e));
        rows = rows.child(
            div()
                .id(gpui::SharedString::from(format!("execution-node-{node}")))
                .flex()
                .items_center()
                .gap(px(theme::GAP_LG))
                .px(px(theme::PAD_XL))
                .py(px(theme::PAD_LG))
                .border_b_1()
                .border_color(t.border)
                .cursor_pointer()
                .when(selected, |d| d.bg(t.lane_card_active_bg))
                .hover(|d| d.bg(t.lane_card_active_bg))
                .child(div().flex_1().min_w_0().truncate().child(node.to_string()))
                .child(presentation::node_badge(history, &node, cx))
                .child(
                    div()
                        .text_color(t.text_muted)
                        .child(attempts.len().to_string()),
                )
                .child(
                    div().text_color(t.text_muted).child(
                        result
                            .map(|a| format!("{:.1}s", a.took.as_secs_f64()))
                            .unwrap_or_else(s::flow::run_not_recorded),
                    ),
                )
                .on_click(cx.listener(move |ws, _, _, cx| {
                    ws.select_execution_node(id, node_id.clone(), cx)
                })),
        );
    }
    div()
        .id(("execution-overview", id.0 as usize))
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_y_scroll()
        .child(summary)
        .child(
            div()
                .px(px(theme::PAD_XL))
                .py(px(theme::PAD_STANDARD))
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(t.text_muted)
                .child(s::flow::run_canvas_hint()),
        )
        .child(graph)
        .child(
            div()
                .flex()
                .gap(px(theme::GAP_LG))
                .px(px(theme::PAD_XL))
                .py(px(theme::PAD_LG))
                .border_y_1()
                .border_color(t.border)
                .text_color(t.text_muted)
                .child(div().flex_1().child(s::flow::run_nodes()))
                .child(s::common::column_status())
                .child(s::flow::run_attempts())
                .child(s::flow::run_duration()),
        )
        .child(rows)
        .child(
            div().p(px(theme::PAD_XL)).child(
                button("execution-report", s::flow::run_report_title())
                    .ghost()
                    .on_click(cx.listener(move |ws, _, _, cx| ws.show_execution_report(id, cx))),
            ),
        )
        .into_any_element()
}

fn metric(label: String, value: String, cx: &gpui::App) -> AnyElement {
    div()
        .flex_1()
        .child(
            div()
                .text_size(px(theme::FONT_SIZE_SM))
                .text_color(theme::current(cx).text_muted)
                .child(label),
        )
        .child(value)
        .into_any_element()
}

#[cfg(test)]
mod tests {}
