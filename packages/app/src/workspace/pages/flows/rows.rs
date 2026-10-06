//! Flow tables provide content; shared list geometry owns every column.

use super::list::{DefinitionList, ORIGINS, RunList, RunRow};
use crate::surface::strings as s;
use crate::ui::list_table::{self, Column, ListTable};
use crate::ui::theme::list_metrics as metrics;
use crate::ui::{Badge, theme, tooltip};
use crate::workspace::{
    flow_browser::FlowPageSnapshot,
    flow_browser::{FlowGrouping, RunFilter},
};
use gpui::{AnyElement, App, Div, IntoElement, MouseButton, SharedString, div, prelude::*, px};

#[derive(Clone, Copy)]
pub(super) enum FileColumn {
    Title,
    Source,
    Modified,
    Actions,
}

#[derive(Clone, Copy)]
enum RunColumn {
    Title,
    Status,
    Started,
    Stage,
    Actions,
}

fn file_layout() -> ListTable<FileColumn> {
    ListTable::new([
        (FileColumn::Title, Column::title()),
        (FileColumn::Source, Column::Fixed(metrics::SOURCE_W)),
        (FileColumn::Modified, Column::Fixed(metrics::TIMESTAMP_W)),
        (
            FileColumn::Actions,
            Column::FixedUnclipped(metrics::ACTIONS_W),
        ),
    ])
}

fn run_layout() -> ListTable<RunColumn> {
    ListTable::new([
        (RunColumn::Title, Column::title()),
        (RunColumn::Status, Column::Fixed(metrics::STATUS_W)),
        (RunColumn::Started, Column::Fixed(metrics::TIMESTAMP_W)),
        (RunColumn::Stage, Column::Fixed(metrics::STAGE_W)),
        (
            RunColumn::Actions,
            Column::FixedUnclipped(metrics::ACTIONS_W),
        ),
    ])
}

pub(super) fn actions(cell: Div) -> Div {
    cell.flex()
        .items_center()
        .justify_end()
        .gap(px(theme::GAP_SM))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn action_heading(cell: Div) -> AnyElement {
    cell.debug_selector(|| "flow-actions-column".into())
        .text_right()
        .child(s::flow::column_actions())
        .into_any_element()
}

pub(super) fn definitions(
    snap: &FlowPageSnapshot,
    list: &DefinitionList<'_>,
    cx: &App,
) -> AnyElement {
    let layout = file_layout();
    let mut body = layout.body().child(layout.header(
        |column, cell| {
            match column {
                FileColumn::Title => cell.child(s::flow::column_flow()).into_any_element(),
                FileColumn::Source => cell
                    .id("flow-source-column")
                    .debug_selector(|| "flow-source-column".into())
                    .child(s::flow::column_origin())
                    .into_any_element(),
                FileColumn::Modified => cell.child(s::flow::column_modified()).into_any_element(),
                FileColumn::Actions => action_heading(cell),
            }
        },
        cx,
    ));
    if snap.flow_browser.state.grouping == FlowGrouping::None {
        body = body.children(
            list.visible
                .iter()
                .map(|file| super::files::flow_row(file, snap, &layout, cx)),
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
                        .map(|file| super::files::flow_row(file, snap, &layout, cx)),
                );
            }
        }
    }
    list_table::scroll("flow-definitions-scroll", body)
        .debug_selector(|| "flow-table-scroll".into())
        .into_any_element()
}

pub(super) fn runs(snap: &FlowPageSnapshot, list: &RunList<'_>, cx: &App) -> AnyElement {
    let layout = run_layout();
    let body = layout
        .body()
        .child(layout.header(
            |column, cell| match column {
                RunColumn::Title => cell.child(s::flow::column_run()).into_any_element(),
                RunColumn::Status => cell.child(s::common::column_status()).into_any_element(),
                RunColumn::Started => cell.child(s::flow::column_started()).into_any_element(),
                RunColumn::Stage => cell.child(s::flow::column_stage()).into_any_element(),
                RunColumn::Actions => action_heading(cell),
            },
            cx,
        ))
        .children(
            list.visible
                .iter()
                .map(|run| run_row(*run, snap, &layout, cx)),
        );
    list_table::scroll("flow-runs-scroll", body)
        .debug_selector(|| "flow-table-scroll".into())
        .into_any_element()
}

fn run_row(
    run: RunRow<'_>,
    snap: &FlowPageSnapshot,
    layout: &ListTable<RunColumn>,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let id = run
        .dir()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = match run {
        RunRow::Past(past) => s::flow::run_status(past.status).to_string(),
        RunRow::Live(live) => live.doing.to_string(),
    };
    let row = layout
        .row(|column, cell| match column {
            RunColumn::Title => cell
                .id("flow-run-name")
                .tooltip(tooltip::text(run.dir().display().to_string()))
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_color(t.text_body)
                        .child(run_name(run, &id)),
                )
                .child(
                    div()
                        .w_full()
                        .truncate()
                        .text_size(px(theme::FONT_SIZE_XS))
                        .text_color(t.text_muted)
                        .child(if run.source().is_some() {
                            id.clone()
                        } else {
                            s::flow::not_recorded()
                        }),
                )
                .into_any_element(),
            RunColumn::Status => {
                let color = match run {
                    RunRow::Live(_) if run.filter() == RunFilter::Asking => theme::WARNING,
                    RunRow::Live(_) => t.right_panel_task_running_color,
                    RunRow::Past(past) => super::past::status_color(past.status),
                };
                let fill = gpui::Hsla {
                    a: theme::RIGHT_PANEL_STATUS_PILL_BG_ALPHA,
                    ..color
                };
                cell.id("flow-run-status")
                    .tooltip(tooltip::text(detail.clone()))
                    .child(
                        Badge::new(super::controls::run_label(run.filter()))
                            .text_color(t.text_body)
                            .bg_color(fill)
                            .border_color(fill)
                            .truncate(),
                    )
                    .into_any_element()
            }
            RunColumn::Started => {
                let started = match run {
                    RunRow::Live(_) => crate::workspace::flow_request::run_started_at(&id)
                        .map(s::flow::run_started_at)
                        .unwrap_or_default(),
                    RunRow::Past(past) => past.started.to_string(),
                };
                cell.text_color(t.text_muted)
                    .child(started)
                    .into_any_element()
            }
            RunColumn::Stage => cell
                .id("flow-run-stage")
                .text_color(t.text_muted)
                .tooltip(tooltip::text(detail.clone()))
                .when(matches!(run, RunRow::Live(_)), |cell| {
                    cell.child(detail.clone())
                })
                .into_any_element(),
            RunColumn::Actions => match run {
                RunRow::Live(live) => actions(cell)
                    .child(super::live::stop_button(live, snap))
                    .into_any_element(),
                RunRow::Past(past) => actions(cell)
                    .children(super::past::resume_button(past, snap))
                    .into_any_element(),
            },
        })
        .id(SharedString::from(format!(
            "flow-past-{}",
            run.dir().display()
        )))
        .debug_selector({
            let dir = run.dir().to_path_buf();
            move || format!("flow-run-row-{}", dir.display())
        });
    match run {
        RunRow::Live(live) => div()
            .flex()
            .flex_col()
            .gap(px(theme::GAP_SM))
            .child(row)
            .children(
                live.asking
                    .as_ref()
                    .map(|ask| super::live::ask_block(live.lane, ask, live.also_waiting, snap, cx)),
            )
            .into_any_element(),
        RunRow::Past(past) => {
            let workspace = snap.workspace.clone();
            let lane = snap.flow_lane;
            row.when_some(past.report.clone(), |row, report| {
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
