//! Three-column execution history: run list, overview, and attempt inspector.

use super::{
    detail::{FlowDetailId, RunDetail},
    execution_detail::Inspection,
};
use crate::ui::{DropdownMenu as _, PopupMenuItem, icons, menu_builder};
use crate::workspace::flow_browser::RunFilter;
use crate::{
    surface::strings as s,
    ui::{ButtonVariants as _, Disableable as _, button, theme},
    workspace::Workspace,
};
use daruda_flow::{NodeId, history::RunHistory};
use gpui::{AnyElement, Context, div, prelude::*, px};
use theme::palette;

mod evidence;
mod inspector;
mod overview;
pub(super) mod presentation;

pub(super) fn nodes(history: &RunHistory) -> Vec<NodeId> {
    let mut nodes: Vec<_> = history
        .flow
        .as_ref()
        .map(|f| f.nodes.iter().map(|n| n.id.clone()).collect())
        .unwrap_or_default();
    for node in history
        .records
        .iter()
        .map(|r| &r.id)
        .chain(history.sessions.iter().map(|s| &s.node))
    {
        if !nodes.contains(node) {
            nodes.push(node.clone());
        }
    }
    nodes
}

pub(super) fn body(
    id: FlowDetailId,
    lane: daruda_store::project::LaneRef,
    run: &RunDetail,
    layout: super::execution_layout::ExecutionLayout,
    session_problem: Option<String>,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let content = if run.execution.content == super::execution_detail::ExecutionContent::Report {
        div()
            .id(("execution-report-scroll", id.0 as usize))
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(theme::DOCK_PAGE_PAD))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(s::flow::run_report_title())
                    .child(
                        button("execution-report-external", s::flow::run_report_open_file())
                            .ghost()
                            .on_click(cx.listener(move |ws, _, window, cx| {
                                ws.open_run_report_file(id, window, cx)
                            })),
                    ),
            )
            .child(super::detail_render::run_body(id, run, cx))
            .into_any_element()
    } else {
        match &run.execution.inspection {
            Inspection::Loading => div()
                .p(px(theme::DOCK_PAGE_PAD))
                .child(s::common::loading())
                .into_any_element(),
            Inspection::Failed(error) => div()
                .id(("execution-failed", id.0 as usize))
                .flex_1()
                .overflow_y_scroll()
                .p(px(theme::DOCK_PAGE_PAD))
                .child(error.clone())
                .child(super::detail_render::run_body(id, run, cx))
                .into_any_element(),
            // Keep intrinsic evidence height out of the flex row measurement;
            // each column scrolls within the remaining page bounds.
            Inspection::Ready(history) => div()
                .absolute()
                .inset_0()
                .flex()
                .min_h_0()
                .min_w_0()
                .when(layout.history(), |row| {
                    row.child(rail(id, lane, run, history, cx))
                })
                .when(layout.overview(), |row| {
                    row.child(overview::render(id, run, history, cx))
                })
                .child(inspector::render(
                    id,
                    run,
                    history,
                    layout.overview(),
                    session_problem,
                    cx,
                ))
                .into_any_element(),
        }
    };
    div()
        .relative()
        .flex()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_hidden()
        .border_t_1()
        .border_color(t.border)
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(t.text_primary)
        .child(content)
        .into_any_element()
}

fn rail(
    id: FlowDetailId,
    lane: daruda_store::project::LaneRef,
    run: &RunDetail,
    history: &RunHistory,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let mut entries = div()
        .id(("execution-run-list", id.0 as usize))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll();
    let mut visible = 0;
    for row in &run.execution.rail {
        let entry = &row.entry;
        let source = &row.source;
        if run.execution.filter != RunFilter::All
            && RunFilter::for_status(entry.status) != run.execution.filter
        {
            continue;
        }
        if run.execution.source_only && source != &history.source {
            continue;
        }
        visible += 1;
        let dir = entry.dir.clone();
        let title = source
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        entries =
            entries.child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "execution-run-{}",
                        dir.display()
                    )))
                    .px(px(theme::PAD_XL))
                    .py(px(theme::PAD_XL))
                    .border_l_2()
                    .border_color(if entry.dir == run.dir {
                        palette::ACCENT
                    } else {
                        palette::TRANSPARENT
                    })
                    .when(entry.dir == run.dir, |d| d.bg(t.lane_card_active_bg))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.lane_card_active_bg))
                    .child(
                        div()
                            .truncate()
                            .child(presentation::started(row.started_at)),
                    )
                    .child(presentation::run_badge(entry.status, cx))
                    .child(div().text_color(t.text_muted).child(format!(
                        "{} · {}",
                        s::flow::run_duration(),
                        presentation::duration(row.elapsed)
                    )))
                    .when_some(title, |d, title| {
                        d.child(
                            div()
                                .text_color(t.text_muted)
                                .text_size(px(theme::FONT_SIZE_SM))
                                .truncate()
                                .child(title),
                        )
                    })
                    .on_click(cx.listener(move |ws, _, window, cx| {
                        ws.open_run_detail(lane, &dir, window, cx)
                    })),
            );
    }
    div()
        .flex()
        .flex_col()
        .w(px(palette::FLOW_RUN_HISTORY_W))
        .flex_none()
        .min_h_0()
        .border_r_1()
        .border_color(t.border)
        .child(
            div().p(px(theme::PAD_STANDARD)).child(
                button(
                    "execution-source-filter",
                    if run.execution.source_only {
                        s::flow::run_source_history()
                    } else {
                        s::flow::run_worktree_history()
                    },
                )
                .ghost()
                .w_full()
                .disabled(history.source.is_none())
                .on_click(cx.listener(move |ws, _, _, cx| ws.toggle_execution_source(id, cx))),
            ),
        )
        .child(filter_picker(id, run.execution.filter, cx))
        .child(entries.when(visible == 0, |d| {
            d.p(px(theme::PAD_XL)).child(s::flow::run_status_empty())
        }))
        .into_any_element()
}

fn filter_picker(id: FlowDetailId, selected: RunFilter, cx: &mut Context<Workspace>) -> AnyElement {
    let workspace = cx.weak_entity();
    div()
        .px(px(theme::PAD_STANDARD))
        .pb(px(theme::PAD_STANDARD))
        .child(
            button(
                "execution-status-filter",
                super::controls::run_label(selected),
            )
            .ghost()
            .w_full()
            .child(icons::icon(icons::EXPAND_MORE))
            .dropdown_menu(menu_builder(move |menu, _, _| {
                RunFilter::ALL
                    .into_iter()
                    .filter(|f| *f != RunFilter::Asking)
                    .fold(menu, |menu, filter| {
                        let workspace = workspace.clone();
                        menu.item(
                            PopupMenuItem::new(super::controls::run_label(filter))
                                .checked(filter == selected)
                                .on_click(move |_, _, cx| {
                                    if let Some(ws) = workspace.upgrade() {
                                        ws.update(cx, |ws, cx| {
                                            ws.set_execution_filter(id, filter, cx)
                                        });
                                    }
                                }),
                        )
                    })
            })),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {}
