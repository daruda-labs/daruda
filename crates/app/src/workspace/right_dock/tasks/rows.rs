//! Aligned task columns; long metadata stays in the title or a tooltip.

use daruda_store::tasks::{Task, TaskAgentSurface, TaskScope, TaskState};
use gpui::{AnyElement, App, Div, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use super::{TaskGrouping, list::TaskList};
use crate::surface::strings;
use crate::ui::{theme, tooltip};
use crate::workspace::layout::RightDockSnapshot;

pub(super) fn table(snap: &RightDockSnapshot, list: &TaskList<'_>, cx: &App) -> AnyElement {
    let show_project = snap.task_browser.state.scope == TaskScope::AllProjects
        && snap.task_browser.state.groups.mode != TaskGrouping::Project;
    let mut table = div()
        .flex()
        .flex_col()
        .min_w(px(if show_project {
            theme::TASK_TABLE_ALL_MIN_W
        } else {
            theme::TASK_TABLE_MIN_W
        }))
        .child(header(show_project, cx));
    if snap.task_browser.state.groups.mode == TaskGrouping::None {
        table = table.children(
            list.visible
                .iter()
                .map(|task| row(task, snap, show_project, cx)),
        );
    } else {
        for group in super::grouping::project(
            &list.visible,
            snap.task_browser.state.groups.mode,
            &snap.task_projects.names,
        ) {
            table = table.child(super::controls::group_header(&group, snap, cx));
            if snap.task_browser.state.groups.is_open(group.key) {
                table = table.children(
                    group
                        .tasks
                        .iter()
                        .map(|task| row(task, snap, show_project, cx)),
                );
            }
        }
    }
    div()
        .id("task-table-scroll")
        .overflow_x_scroll()
        .child(table)
        .into_any_element()
}

fn frame() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::PAD_SM))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
}

fn cell(width: f32) -> Div {
    div().w(px(width)).flex_none().min_w_0().truncate()
}

fn title_cell() -> Div {
    div()
        .w(px(theme::TASK_TABLE_TITLE_MIN_W))
        .flex_grow()
        .flex_shrink_0()
}

fn header(show_project: bool, cx: &App) -> Div {
    frame()
        .text_color(theme::current(cx).text_muted)
        .border_b_1()
        .border_color(theme::current(cx).border)
        .child(cell(theme::RIGHT_PANEL_TASK_INDICATOR_W))
        .child(title_cell().child(strings::task::column_task()))
        .when(show_project, |row| {
            row.child(
                cell(theme::RIGHT_PANEL_TASK_PROJECT_MAX_W)
                    .id("task-project-column")
                    .debug_selector(|| "task-project-column".into())
                    .child(strings::task::column_project()),
            )
        })
        .child(cell(theme::TASK_TABLE_STATUS_W).child(strings::common::column_status()))
        .child(cell(theme::TASK_TABLE_AGENT_W).child(strings::task::column_agent()))
        .child(cell(theme::TASK_TABLE_UPDATED_W).child(strings::task::column_updated()))
}

fn row(task: &Task, snap: &RightDockSnapshot, show_project: bool, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    let workspace = snap.workspace.clone();
    let id = task.id.clone();
    let project = snap
        .task_projects
        .name(task.project)
        .map(str::to_owned)
        .unwrap_or_else(strings::task::project_not_open);
    let agent = agent_label(task, &snap.task_agents);
    let elapsed = (*snap.now - task.updated_at).to_std().unwrap_or_default();
    let updated = updated_label(elapsed.as_secs());
    let metadata = strings::task::row_metadata(short_id(task), &task.branch_name);
    let title = title_cell()
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .truncate()
                .text_color(t.text_body)
                .child(task.title.clone()),
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
                || !matches!(task.state, TaskState::Backlog),
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
        );
    frame()
        .id(SharedString::from(format!("task-row-{}", task.id)))
        .debug_selector({
            let id = task.id.clone();
            move || format!("task-row-{id}")
        })
        .hover(|style| style.bg(t.overlay_hover))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| {
                    ws.open_task_edit_pane(Some(id.clone()), window, cx)
                });
            }
        })
        .child(super::indicator_cell(&task.state, *snap.now, cx))
        .child(
            title
                .id("open")
                .debug_selector({
                    let id = task.id.clone();
                    move || format!("task-title-{id}")
                })
                .cursor_pointer()
                .tooltip(tooltip::text(format!(
                    "{}\n{}\n{}",
                    task.title, task.id, task.branch_name
                ))),
        )
        .when(show_project, |row| {
            row.child(
                cell(theme::RIGHT_PANEL_TASK_PROJECT_MAX_W)
                    .id("project")
                    .debug_selector({
                        let id = task.id.clone();
                        move || format!("task-project-{id}")
                    })
                    .tooltip(tooltip::text(project.clone()))
                    .text_color(t.text_muted)
                    .child(project),
            )
        })
        .child(
            cell(theme::TASK_TABLE_STATUS_W)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(super::status_pill::status_pill(
                    task,
                    snap,
                    compact_status(&task.state).into(),
                    super::state_label(&task.state),
                    cx,
                )),
        )
        .child(
            cell(theme::TASK_TABLE_AGENT_W)
                .id("agent")
                .tooltip(tooltip::text(agent.clone()))
                .text_color(t.text_muted)
                .child(agent),
        )
        .child(
            cell(theme::TASK_TABLE_UPDATED_W)
                .id("updated")
                .tooltip(tooltip::text(task.updated_at.to_rfc3339()))
                .text_color(t.text_muted)
                .child(updated),
        )
        .into_any_element()
}

fn compact_status(state: &TaskState) -> String {
    use daruda_store::tasks::TaskFilter;
    super::controls::filter_label(match state {
        TaskState::Backlog => TaskFilter::Backlog,
        TaskState::Running { .. } => TaskFilter::Running,
        TaskState::Done { .. } => TaskFilter::Done,
        TaskState::Error { .. } => TaskFilter::Failed,
        TaskState::Cancelled { .. } => TaskFilter::Cancelled,
    })
}

fn short_id(task: &Task) -> String {
    task.id
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn updated_label(seconds: u64) -> String {
    match seconds {
        0..60 => strings::task::updated_now(),
        60..3600 => strings::task::updated_minutes(seconds / 60),
        3600..86400 => strings::task::updated_hours(seconds / 3600),
        _ => strings::task::updated_days(seconds / 86400),
    }
}

fn agent_label(task: &Task, agents: &[daruda_config::AgentDefinition]) -> String {
    if let Some(run) = &task.execution {
        return agents
            .iter()
            .find(|agent| agent.id == run.agent_id)
            .map(|agent| agent.name.trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(&run.agent_id)
            .to_owned();
    }
    match task.agent_surface {
        TaskAgentSurface::Terminal => strings::task::agent_terminal(),
        TaskAgentSurface::AgentChat if matches!(task.state, TaskState::Backlog) => {
            strings::task::agent_on_start()
        }
        TaskAgentSurface::AgentChat => strings::task::agent_unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::{project::ProjectUuid, tasks::TaskExecution};

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
        let mut task = Task::new(ProjectUuid::new(), String::new(), String::new(), None);
        task.agent_surface = TaskAgentSurface::AgentChat;
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
        let mut task = Task::new(ProjectUuid::new(), String::new(), String::new(), None);
        task.id = "01ABCDEFGHIJKLMNPQRST123456".into();
        assert_eq!(short_id(&task), "123456");
        task.id = "old".into();
        assert_eq!(short_id(&task), "old");
    }
}
