//! Shared column geometry for definitions and runs; details stay in their editors.

use super::list::{DefinitionList, ORIGINS, RunList, RunRow};
use crate::surface::strings as s;
use crate::ui::{Badge, theme, tooltip};
use crate::workspace::{
    flow_browser::{FlowGrouping, RunFilter},
    layout::RightDockSnapshot,
};
use gpui::{AnyElement, App, Div, IntoElement, MouseButton, SharedString, div, prelude::*, px};

pub(super) fn frame() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::PAD_SM))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
}

pub(super) fn cell(width: f32) -> Div {
    div().w(px(width)).flex_none().min_w_0().truncate()
}

pub(super) fn title_cell() -> Div {
    div()
        .w(px(theme::FLOW_TABLE_TITLE_MIN_W))
        .flex_grow()
        .flex_shrink_0()
        .min_w_0()
}

pub(super) fn actions() -> Div {
    div()
        .w(px(theme::FLOW_TABLE_ACTIONS_W))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(theme::GAP_SM))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn table_header(runs: bool, cx: &App) -> Div {
    let t = theme::current(cx);
    let mut row = frame()
        .text_color(t.text_muted)
        .border_b_1()
        .border_color(t.border)
        .child(title_cell().child(if runs {
            s::flow::column_run()
        } else {
            s::flow::column_flow()
        }));
    if runs {
        row = row
            .child(cell(theme::FLOW_TABLE_STATUS_W).child(s::common::column_status()))
            .child(cell(theme::FLOW_TABLE_TIME_W).child(s::flow::column_started()))
            .child(cell(theme::FLOW_TABLE_STAGE_W).child(s::flow::column_stage()));
    } else {
        row = row
            .child(
                cell(theme::FLOW_TABLE_ORIGIN_W)
                    .id("flow-source-column")
                    .debug_selector(|| "flow-source-column".into())
                    .child(s::flow::column_origin()),
            )
            .child(cell(theme::FLOW_TABLE_TIME_W).child(s::flow::column_modified()));
    }
    row.child(
        cell(theme::FLOW_TABLE_ACTIONS_W)
            .debug_selector(|| "flow-actions-column".into())
            .text_right()
            .child(s::flow::column_actions()),
    )
}

pub(super) fn definitions(
    snap: &RightDockSnapshot,
    list: &DefinitionList<'_>,
    cx: &App,
) -> AnyElement {
    let mut body = div()
        .flex()
        .flex_col()
        .min_w(px(theme::FLOW_TABLE_FILES_MIN_W))
        .child(table_header(false, cx));
    if snap.flow_browser.state.grouping == FlowGrouping::None {
        body = body.children(
            list.visible
                .iter()
                .map(|file| super::files::flow_row(file, snap, cx)),
        );
    } else {
        for origin in ORIGINS.into_iter().flatten() {
            let files: Vec<_> = list
                .visible
                .iter()
                .filter(|file| file.origin == origin)
                .collect();
            if files.is_empty() {
                continue;
            }
            body = body.child(super::toolbar::group_header(origin, files.len(), snap, cx));
            if snap.flow_browser.state.is_open(origin) {
                body = body.children(
                    files
                        .into_iter()
                        .map(|file| super::files::flow_row(file, snap, cx)),
                );
            }
        }
    }
    div()
        .id("flow-definitions-scroll")
        .debug_selector(|| "flow-table-scroll".into())
        .overflow_x_scroll()
        .child(body)
        .into_any_element()
}

pub(super) fn runs(snap: &RightDockSnapshot, list: &RunList<'_>, cx: &App) -> AnyElement {
    div()
        .id("flow-runs-scroll")
        .debug_selector(|| "flow-table-scroll".into())
        .overflow_x_scroll()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(theme::FLOW_TABLE_RUNS_MIN_W))
                .child(table_header(true, cx))
                .children(list.visible.iter().map(|run| run_row(*run, snap, cx))),
        )
        .into_any_element()
}

fn run_row(run: RunRow<'_>, snap: &RightDockSnapshot, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    let id = run
        .dir()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = run_name(run, &id);
    let subtitle = if run.source().is_some() {
        id.clone()
    } else {
        s::flow::not_recorded()
    };
    let started = match run {
        RunRow::Live(_) => crate::workspace::flow_request::run_started_at(&id)
            .map(s::flow::run_started_at)
            .unwrap_or_default(),
        RunRow::Past(past) => past.started.to_string(),
    };
    let color = match run {
        RunRow::Live(_) if run.filter() == RunFilter::Asking => theme::WARNING,
        RunRow::Live(_) => t.right_panel_task_running_color,
        RunRow::Past(past) => super::past::status_color(past.status),
    };
    let status = super::controls::run_label(run.filter());
    let detail = match run {
        RunRow::Past(past) => s::flow::run_status(past.status).to_string(),
        RunRow::Live(live) => live.doing.to_string(),
    };
    let fill = gpui::Hsla {
        a: theme::RIGHT_PANEL_STATUS_PILL_BG_ALPHA,
        ..color
    };
    let mut row = frame()
        .id(SharedString::from(format!(
            "flow-past-{}",
            run.dir().display()
        )))
        .debug_selector({
            let dir = run.dir().to_path_buf();
            move || format!("flow-run-row-{}", dir.display())
        })
        .child(
            title_cell()
                .id("flow-run-name")
                .tooltip(tooltip::text(run.dir().display().to_string()))
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_color(t.text_body)
                        .child(name),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(px(theme::FONT_SIZE_XS))
                        .text_color(t.text_muted)
                        .child(subtitle),
                ),
        )
        .child(
            cell(theme::FLOW_TABLE_STATUS_W)
                .id("flow-run-status")
                .tooltip(tooltip::text(detail.clone()))
                .child(
                    Badge::new(status)
                        .text_color(t.text_body)
                        .bg_color(fill)
                        .border_color(fill)
                        .truncate(),
                ),
        )
        .child(
            cell(theme::FLOW_TABLE_TIME_W)
                .text_color(t.text_muted)
                .child(started),
        )
        .child(
            cell(theme::FLOW_TABLE_STAGE_W)
                .id("flow-run-stage")
                .text_color(t.text_muted)
                .tooltip(tooltip::text(detail.clone()))
                .when(matches!(run, RunRow::Live(_)), |cell| cell.child(detail)),
        );
    match run {
        RunRow::Live(live) => {
            row = row.child(actions().child(super::live::stop_button(live, snap)));
            div()
                .flex()
                .flex_col()
                .gap(px(theme::GAP_SM))
                .child(row)
                .children(
                    live.asking.as_ref().map(|ask| {
                        super::live::ask_block(live.lane, ask, live.also_waiting, snap, cx)
                    }),
                )
                .into_any_element()
        }
        RunRow::Past(past) => {
            let workspace = snap.workspace.clone();
            let lane = snap.flow_lane;
            row.child(actions().children(super::past::resume_button(past, snap)))
                .when_some(past.report.clone(), |row, report| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(t.overlay_hover))
                        .on_click(move |_, window, cx| {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(cx, |ws, cx| {
                                    ws.open_browsed_report(lane, &report, window, cx)
                                });
                            }
                        })
                })
                .into_any_element()
        }
    }
}

fn run_name(run: RunRow<'_>, id: &str) -> String {
    run.source()
        .map(crate::workspace::flow_paths::flow_label)
        .unwrap_or_else(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_runs_are_named_by_id_not_by_an_unrelated_current_flow() {
        let past = crate::workspace::flow_history::FlowRunEntry {
            dir: "run-01".into(),
            started: "now".into(),
            report: None,
            status: daruda_flow::marker::RunStatus::Done,
        };
        assert_eq!(run_name(RunRow::Past(&past), "run-01"), "run-01");
    }
}
