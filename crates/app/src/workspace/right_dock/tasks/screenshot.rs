//! Deterministic list fixtures; no task launches or agent sessions.

use crate::agent::tasks_global::GlobalTasks;
use crate::workspace::{Workspace, pages::Page};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{
    SessionEndReason, Task, TaskExecution, TaskFilter, TaskScope, TaskState,
};
use gpui::{BorrowAppContext as _, Context, Window};

fn fixtures(project: ProjectUuid) -> Vec<Task> {
    let path = std::path::PathBuf::from("/preview/worktree");
    [
        (
            "Improve project filtering",
            TaskState::Running {
                worktree_path: path.clone(),
            },
        ),
        ("Add keyboard navigation to task picker", TaskState::Backlog),
        (
            "Simplify task creation and editing",
            TaskState::Done {
                worktree_path: path.clone(),
                end_reason: SessionEndReason::Stop,
            },
        ),
        (
            "Investigate CI authentication failure",
            TaskState::Error {
                worktree_path: path.clone(),
                message: "Authentication failed".into(),
            },
        ),
        (
            "Explore alternate sidebar layout",
            TaskState::Cancelled {
                worktree_path: path,
            },
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (title, state))| {
        let mut task = Task::new(project, title.into(), String::new(), None);
        task.state = state;
        task.created_at = chrono::Utc::now() - chrono::Duration::minutes(30 + index as i64);
        task.updated_at = task.created_at + chrono::Duration::minutes(10);
        if !matches!(task.state, TaskState::Backlog) {
            task.execution = Some(TaskExecution::begin(
                Default::default(),
                if index.is_multiple_of(2) {
                    "codex"
                } else {
                    "claude"
                }
                .into(),
                None,
                "/preview/worktree".into(),
            ));
        }
        if matches!(
            task.state,
            TaskState::Done { .. } | TaskState::Error { .. } | TaskState::Cancelled { .. }
        ) {
            task.finished_at = Some(task.updated_at);
        }
        task
    })
    .collect()
}

impl Workspace {
    pub(in crate::workspace) fn seed_task_list_for_shot(
        &mut self,
        scope: TaskScope,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_project().is_none() {
            self.add_project(self.data_dir.clone(), window, cx);
        }
        let Some(project) = self.active_project().map(|p| p.uuid) else {
            return;
        };
        let active = self.active;
        let root = std::env::temp_dir().join("daruda-task-list-preview");
        if let Err(error) = std::fs::create_dir_all(&root) {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(error.to_string())
                    .at(file!(), line!())
                    .dedup("screenshot.tasks.fixture")
                    .build(),
                cx,
            );
            return;
        }
        self.add_project(root, window, cx);
        let Some(other) = self.active_project().map(|p| p.uuid) else {
            return;
        };
        for p in &mut self.projects {
            if p.uuid == project {
                p.name = "daruda".into();
            }
            if p.uuid == other {
                p.name = "website".into();
            }
        }
        self.activate_lane(active, window, cx);
        cx.update_global::<GlobalTasks, _>(|tasks, _| {
            tasks.tasks = fixtures(project);
            tasks.add(Task::new(
                other,
                "Refresh the documentation site".into(),
                String::new(),
                None,
            ));
        });
        self.set_task_scope(scope, cx);
        self.set_task_filter(TaskFilter::All, cx);
        self.task_browser
            .search
            .clone()
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.open_page(Page::Tasks, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_cover_every_status_without_live_sessions() {
        let tasks = fixtures(ProjectUuid::new());
        for filter in super::super::list::FILTERS.into_iter().skip(1) {
            assert_eq!(
                tasks
                    .iter()
                    .filter(|task| filter.matches(&task.state))
                    .count(),
                1
            );
        }
        assert!(tasks.iter().all(|task| {
            task.execution
                .as_ref()
                .is_none_or(|run| run.session_id.is_none())
        }));
    }
}
