//! Task-page scope, status tabs, and recoverable empty states.

use daruda_store::tasks::{TaskFilter, TaskScope};
use gpui::{AnyElement, App, FontWeight, IntoElement, div, prelude::*, px};

use super::list::{FILTERS, TaskList};
use super::{TaskGroupKey, TaskGrouping, grouping::TaskGroup};
use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, Disableable as _, DropdownMenu as _, PopupMenuItem, button, button_bare,
    button_with_icon, icons, menu_builder, tab, tab_bar, theme,
};
use crate::workspace::layout::{RightDockSnapshot, TaskProjects};

pub(in crate::workspace) fn header(
    snap: &RightDockSnapshot,
    close: AnyElement,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_STANDARD))
        .px(px(theme::DOCK_PAGE_PAD))
        .py(px(theme::PAD_STANDARD))
        .child(
            div()
                .flex_none()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(strings::dock::right_tab_tasks()),
        )
        .child(scope_picker(snap, cx))
        .child(div().flex_1())
        .child(new_button(snap))
        .child(close)
        .into_any_element()
}

fn project_label(scope: TaskScope, projects: &TaskProjects) -> String {
    if scope == TaskScope::AllProjects {
        return strings::task::scope_every_project();
    }
    let project = scope.project(projects.active);
    let name = project.and_then(|id| projects.name(id));
    match (project, name) {
        (Some(id), Some(name)) if Some(id) == projects.active => strings::task::scope_current(name),
        (_, Some(name)) => name.to_owned(),
        _ => strings::task::project_not_open(),
    }
}

fn scope_picker(snap: &RightDockSnapshot, cx: &App) -> impl IntoElement {
    let projects = snap.task_projects.clone();
    let selected = snap.task_scope;
    let workspace = snap.workspace.clone();
    let label = project_label(selected, &projects);
    button_bare("task-scope")
        .debug_selector(|| "task-scope".into())
        .tab_stop(true)
        .min_w_0()
        .tooltip(strings::task::scope_hint())
        .child(icons::icon(icons::FOLDER).text_color(theme::current(cx).text_muted))
        .child(
            div()
                .min_w_0()
                .max_w(px(theme::RIGHT_PANEL_TASK_PROJECT_MAX_W))
                .truncate()
                .child(label),
        )
        .child(icons::icon(icons::EXPAND_MORE))
        .dropdown_menu(menu_builder(move |menu, _, _| {
            let mut scopes = vec![TaskScope::AllProjects];
            scopes.extend(projects.names.iter().map(|(id, _)| {
                if Some(*id) == projects.active {
                    TaskScope::ActiveProject
                } else {
                    TaskScope::Project(*id)
                }
            }));
            scopes
                .into_iter()
                .fold(menu.scrollable(true), |menu, scope| {
                    let workspace = workspace.clone();
                    let checked = scope == selected
                        || (scope != TaskScope::AllProjects
                            && selected != TaskScope::AllProjects
                            && scope.project(projects.active) == selected.project(projects.active));
                    menu.item(
                        PopupMenuItem::new(project_label(scope, &projects))
                            .checked(checked)
                            .on_click(move |_, _, cx| {
                                if let Some(ws) = workspace.upgrade() {
                                    ws.update(cx, |ws, cx| ws.set_task_scope(scope, cx));
                                }
                            }),
                    )
                })
        }))
}

fn new_button(snap: &RightDockSnapshot) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    let project = snap
        .task_scope
        .project(snap.task_projects.active)
        .or(snap.task_projects.active);
    let target = project.and_then(|id| snap.task_projects.name(id));
    button_with_icon("task-new", strings::task::new_button(), icons::ADD)
        .debug_selector(|| "task-new".into())
        .primary()
        .flex_none()
        .tab_stop(true)
        .disabled(target.is_none())
        .tooltip(
            target
                .map(strings::task::new_in_project)
                .unwrap_or_else(strings::task::project_not_open),
        )
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.new_task_in_scope(window, cx));
            }
        })
}

pub(super) fn filter_label(filter: TaskFilter) -> String {
    match filter {
        TaskFilter::All => strings::task::filter_all(),
        TaskFilter::Backlog => strings::task::filter_backlog(),
        TaskFilter::Running => strings::task::filter_running(),
        TaskFilter::Done => strings::task::filter_done(),
        TaskFilter::Failed => strings::task::filter_failed(),
        TaskFilter::Cancelled => strings::task::filter_cancelled(),
    }
}

pub(super) fn status_tabs(snap: &RightDockSnapshot, list: &TaskList<'_>) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    tab_bar("task-status-tabs")
        .w_full()
        .menu(true)
        .selected_index(
            FILTERS
                .iter()
                .position(|filter| *filter == snap.task_filter)
                .unwrap_or(0),
        )
        .children(FILTERS.into_iter().zip(list.counts).enumerate().map(
            |(index, (filter, count))| {
                tab(strings::common::filter_count(filter_label(filter), count))
                    .debug_selector(move || format!("task-status-{index}"))
            },
        ))
        .on_click(move |index, _, cx| {
            if let Some(filter) = FILTERS.get(*index).copied()
                && let Some(ws) = workspace.upgrade()
            {
                ws.update(cx, |ws, cx| ws.set_task_filter(filter, cx));
            }
        })
}

pub(super) fn empty_state(snap: &RightDockSnapshot, scoped_count: usize, cx: &App) -> AnyElement {
    let project_available = snap.task_scope == TaskScope::AllProjects
        || snap
            .task_scope
            .project(snap.task_projects.active)
            .and_then(|id| snap.task_projects.name(id))
            .is_some();
    let message = if !project_available {
        strings::task::empty_project_unavailable()
    } else if scoped_count == 0 {
        strings::task::empty_scope(project_label(snap.task_scope, &snap.task_projects))
    } else {
        strings::task::empty_filtered()
    };
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(theme::GAP_STANDARD))
        .py(px(theme::DOCK_PAGE_PAD))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .child(message)
        .into_any_element()
}

pub(super) fn results(snap: &RightDockSnapshot, list: &TaskList<'_>, cx: &App) -> impl IntoElement {
    let filtered = snap.task_filter != TaskFilter::All || !snap.task_search_query.trim().is_empty();
    let workspace = snap.workspace.clone();
    div()
        .flex()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap(px(theme::GAP_SM))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_muted)
        .child(strings::task::result_count(
            list.visible.len(),
            list.scoped_count,
        ))
        .when(filtered, |row| {
            row.child(
                button("task-clear-filters", strings::task::clear_filters())
                    .ghost()
                    .debug_selector(|| "task-clear-filters".into())
                    .tab_stop(true)
                    .on_click(move |_, window, cx| {
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.clear_task_filters(window, cx));
                        }
                    }),
            )
        })
}

fn grouping_label(mode: TaskGrouping) -> String {
    match mode {
        TaskGrouping::None => strings::common::group_none(),
        TaskGrouping::Status => strings::common::column_status(),
        TaskGrouping::Project => strings::task::column_project(),
    }
}

pub(super) fn grouping_picker(snap: &RightDockSnapshot) -> impl IntoElement {
    let selected = snap.task_groups.mode;
    let workspace = snap.workspace.clone();
    button(
        "task-grouping",
        strings::common::group_by(grouping_label(selected)),
    )
    .debug_selector(|| "task-grouping".into())
    .flex_none()
    .tab_stop(true)
    .child(icons::icon(icons::EXPAND_MORE))
    .dropdown_menu(menu_builder(move |menu, _, _| {
        [
            TaskGrouping::None,
            TaskGrouping::Status,
            TaskGrouping::Project,
        ]
        .into_iter()
        .fold(menu, |menu, mode| {
            let workspace = workspace.clone();
            menu.item(
                PopupMenuItem::new(grouping_label(mode))
                    .checked(mode == selected)
                    .on_click(move |_, _, cx| {
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.set_task_grouping(mode, cx));
                        }
                    }),
            )
        })
    }))
}

pub(super) fn group_header(
    group: &TaskGroup<'_>,
    index: usize,
    snap: &RightDockSnapshot,
    cx: &App,
) -> impl IntoElement {
    let label = match group.key {
        TaskGroupKey::Status(filter) => filter_label(filter),
        TaskGroupKey::Project(project) => snap
            .task_projects
            .name(project)
            .map(str::to_owned)
            .unwrap_or_else(strings::task::project_not_open),
    };
    let key = group.key;
    let workspace = snap.workspace.clone();
    button_bare(("task-group", index))
        .debug_selector(move || format!("task-group-{index}"))
        .ghost()
        .tab_stop(true)
        .w_full()
        .justify_start()
        .text_color(theme::current(cx).text_muted)
        .child(crate::ui::disclosure(
            ("task-group-chevron", index),
            snap.task_groups.is_open(key),
        ))
        .child(strings::common::filter_count(label, group.tasks.len()))
        .on_click(move |_, _, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.toggle_task_group(key, cx));
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::ProjectUuid;

    #[test]
    fn current_project_is_a_marker_on_its_name_not_another_scope() {
        let current = ProjectUuid::new();
        let other = ProjectUuid::new();
        let projects = TaskProjects {
            active: Some(current),
            names: vec![(current, "app".into()), (other, "site".into())],
        };
        assert_eq!(
            project_label(TaskScope::ActiveProject, &projects),
            strings::task::scope_current("app")
        );
        assert_eq!(
            project_label(TaskScope::Project(current), &projects),
            strings::task::scope_current("app")
        );
        assert_eq!(project_label(TaskScope::Project(other), &projects), "site");
        assert_eq!(
            project_label(TaskScope::Project(ProjectUuid::default()), &projects),
            strings::task::project_not_open()
        );
    }
}
