//! Aligned task columns; long metadata stays in the title or a tooltip.

use daruda_core::age::{self, AgeBucket};
use daruda_store::tasks::{Lifecycle, TaskAgentSurface, TaskRecord, TaskScope};
use gpui::{AnyElement, App, Div, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use super::{TaskGrouping, list::TaskList};
use crate::surface::strings;
use crate::ui::list_table::{self, Column, ListTable};
use crate::ui::theme::list_metrics as metrics;
use crate::ui::{theme, tooltip};
use crate::workspace::layout::RightDockSnapshot;

#[derive(Clone, Copy)]
enum TaskColumn {
    Indicator,
    Title,
    Project,
    Status,
    Agent,
    Updated,
}

fn layout(show_project: bool) -> ListTable<TaskColumn> {
    let mut columns = vec![
        (
            TaskColumn::Indicator,
            Column::Fixed(theme::RIGHT_PANEL_TASK_INDICATOR_W),
        ),
        (TaskColumn::Title, Column::title()),
    ];
    if show_project {
        columns.push((TaskColumn::Project, Column::Fixed(metrics::PROJECT_W)));
    }
    columns.extend([
        (TaskColumn::Status, Column::Fixed(metrics::STATUS_W)),
        (TaskColumn::Agent, Column::Fixed(metrics::AGENT_W)),
        (TaskColumn::Updated, Column::Fixed(metrics::RELATIVE_TIME_W)),
    ]);
    ListTable::new(columns)
}

pub(super) fn table(snap: &RightDockSnapshot, list: &TaskList<'_>, cx: &App) -> AnyElement {
    let show_project = snap.task_browser.state.scope == TaskScope::AllProjects
        && snap.task_browser.state.groups.mode != TaskGrouping::Project;
    let layout = layout(show_project);
    let mut table = layout.body().child(header(&layout, cx));
    if snap.task_browser.state.groups.mode == TaskGrouping::None {
        table = table.children(list.visible.iter().map(|task| row(task, snap, &layout, cx)));
    } else {
        for group in super::grouping::project(
            &list.visible,
            snap.task_browser.state.groups.mode,
            &snap.task_projects.names,
        ) {
            table = table.child(super::controls::group_header(&group, snap, cx));
            if snap.task_browser.state.groups.is_open(group.key) {
                table = table.children(group.tasks.iter().map(|task| row(task, snap, &layout, cx)));
            }
        }
    }
    list_table::scroll("task-table-scroll", table).into_any_element()
}

fn header(layout: &ListTable<TaskColumn>, cx: &App) -> Div {
    layout.header(
        |column, cell| match column {
            TaskColumn::Indicator => cell.into_any_element(),
            TaskColumn::Title => cell.child(strings::task::column_task()).into_any_element(),
            TaskColumn::Project => cell
                .id("task-project-column")
                .debug_selector(|| "task-project-column".into())
                .child(strings::task::column_project())
                .into_any_element(),
            TaskColumn::Status => cell
                .child(strings::common::column_status())
                .into_any_element(),
            TaskColumn::Agent => cell.child(strings::task::column_agent()).into_any_element(),
            TaskColumn::Updated => cell
                .child(strings::task::column_updated())
                .into_any_element(),
        },
        cx,
    )
}

fn row(
    task: &TaskRecord,
    snap: &RightDockSnapshot,
    layout: &ListTable<TaskColumn>,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let id = task.id().to_owned();
    layout
        .row(|column, cell| match column {
            TaskColumn::Indicator => cell
                .child(super::indicator_cell(&task.lifecycle, *snap.now, cx))
                .into_any_element(),
            TaskColumn::Title => title(task, snap, cell, cx),
            TaskColumn::Project => {
                let project = snap
                    .task_projects
                    .name(task.spec.project)
                    .map(str::to_owned)
                    .unwrap_or_else(strings::task::project_not_open);
                cell.id("project")
                    .debug_selector({
                        let id = task.id().to_owned();
                        move || format!("task-project-{id}")
                    })
                    .tooltip(tooltip::text(project.clone()))
                    .text_color(t.text_muted)
                    .child(project)
                    .into_any_element()
            }
            TaskColumn::Status => cell
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(super::status_pill::status_pill(
                    task,
                    snap,
                    compact_status(&task.lifecycle).into(),
                    super::state_label(&task.lifecycle),
                    cx,
                ))
                .into_any_element(),
            TaskColumn::Agent => {
                let agent = agent_label(task, &snap.task_agents);
                cell.id("agent")
                    .tooltip(tooltip::text(agent.clone()))
                    .text_color(t.text_muted)
                    .child(agent)
                    .into_any_element()
            }
            TaskColumn::Updated => {
                let elapsed = (*snap.now - task.updated_at).to_std().unwrap_or_default();
                cell.id("updated")
                    .tooltip(tooltip::text(task.updated_at.to_rfc3339()))
                    .text_color(t.text_muted)
                    .child(updated_label(elapsed.as_secs()))
                    .into_any_element()
            }
        })
        .id(SharedString::from(format!("task-row-{}", task.id())))
        .debug_selector({
            let id = task.id().to_owned();
            move || format!("task-row-{id}")
        })
        .hover(|style| style.bg(t.overlay_hover))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| {
                    ws.open_task_editor(Some(id.clone()), window, cx)
                });
            }
        })
        .into_any_element()
}

fn title(task: &TaskRecord, snap: &RightDockSnapshot, cell: Div, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    let metadata = strings::task::row_metadata(short_id(task.id()), &task.spec.branch_name);
    cell.flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .truncate()
                .text_color(t.text_body)
                .child(task.spec.title.clone()),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .text_color(t.text_muted)
                .text_size(px(theme::FONT_SIZE_XS))
                .child(metadata),
        )
        .when(
            !task.session_ids.is_empty()
                || !task.subtasks.is_empty()
                || task.lifecycle != Lifecycle::Backlog,
            |title| {
                title.child(
                    div()
                        .flex()
                        .w_full()
                        .overflow_hidden()
                        .gap(px(theme::GAP_SM))
                        .text_size(px(theme::FONT_SIZE_XS))
                        .children(super::duration_cell(task, snap, t))
                        .children(super::session_badge(task, snap, cx))
                        .children(super::failure_indicator(task, snap))
                        .when(!task.subtasks.is_empty(), |meta| {
                            meta.child(super::subtask_progress_cell(task, cx))
                        }),
                )
            },
        )
        .id("open")
        .debug_selector({
            let id = task.id().to_owned();
            move || format!("task-title-{id}")
        })
        .cursor_pointer()
        .tooltip(tooltip::text(format!(
            "{}\n{}\n{}",
            task.spec.title,
            task.id(),
            task.spec.branch_name
        )))
        .into_any_element()
}

/// The pill's word: the status tab a state counts under, except where a
/// state has a name of its own.
fn compact_status(state: &Lifecycle) -> String {
    use daruda_store::tasks::TaskFilter;
    super::controls::filter_label(match state {
        Lifecycle::Backlog => TaskFilter::Backlog,
        Lifecycle::Preparing { .. } | Lifecycle::Dispatching { .. } => {
            return strings::task::state_starting();
        }
        Lifecycle::Running { .. } => TaskFilter::Running,
        Lifecycle::Done { .. } => TaskFilter::Done,
        Lifecycle::Error { .. } => TaskFilter::Failed,
        Lifecycle::Cancelled { .. } => TaskFilter::Cancelled,
        Lifecycle::Undelivered { .. } => return strings::task::state_undelivered(),
        Lifecycle::Interrupted { .. } => return strings::task::state_interrupted(),
    })
}

fn short_id(id: &str) -> String {
    id.chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn updated_label(seconds: u64) -> String {
    match age::bucket(seconds) {
        AgeBucket::JustNow => strings::task::updated_now(),
        AgeBucket::Minutes(n) => strings::task::updated_minutes(n),
        AgeBucket::Hours(n) => strings::task::updated_hours(n),
        AgeBucket::Days(n) => strings::task::updated_days(n),
    }
}

fn agent_label(task: &TaskRecord, agents: &[daruda_config::AgentDefinition]) -> String {
    if let Some(run) = &task.execution {
        return agents
            .iter()
            .find(|agent| agent.id == run.agent_id)
            .map(|agent| agent.name.trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(&run.agent_id)
            .to_owned();
    }
    match task.spec.agent_surface {
        TaskAgentSurface::Terminal => strings::task::agent_terminal(),
        TaskAgentSurface::AgentChat if task.lifecycle == Lifecycle::Backlog => {
            strings::task::agent_on_start()
        }
        TaskAgentSurface::AgentChat => strings::task::agent_unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::{project::ProjectUuid, tasks::TaskExecution};

    fn blank() -> TaskRecord {
        crate::runs::seed::record(ProjectUuid::new(), "", Lifecycle::Backlog)
    }

    #[test]
    fn updated_labels_use_whole_age_units() {
        assert_eq!(updated_label(59), strings::task::updated_now());
        assert_eq!(updated_label(60), strings::task::updated_minutes(1));
        assert_eq!(updated_label(3599), strings::task::updated_minutes(59));
        assert_eq!(updated_label(3600), strings::task::updated_hours(1));
        assert_eq!(updated_label(86400), strings::task::updated_days(1));
    }

    #[test]
    fn agent_identity_comes_from_execution_not_legacy_agent_type() {
        let mut task = blank();
        task.spec.agent_surface = TaskAgentSurface::AgentChat;
        assert_eq!(agent_label(&task, &[]), strings::task::agent_on_start());
        task.execution = Some(TaskExecution::begin(
            Default::default(),
            "codex".into(),
            None,
            "/tmp".into(),
        ));
        let named = |name: &str| {
            let mut agent = daruda_config::AgentDefinition::claude_default();
            agent.id = "codex".into();
            agent.name = name.into();
            [agent]
        };
        assert_eq!(agent_label(&task, &named("Codex")), "Codex");
        assert_eq!(agent_label(&task, &[]), "codex");
        assert_eq!(agent_label(&task, &named(" ")), "codex");
    }

    #[test]
    fn id_uses_the_distinguishing_ulid_suffix() {
        assert_eq!(short_id("01ABCDEFGHIJKLMNPQRST123456"), "123456");
        assert_eq!(short_id("old"), "old");
    }
}
