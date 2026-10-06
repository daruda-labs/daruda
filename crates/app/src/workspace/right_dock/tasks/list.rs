//! One projection drives visible rows and status counts.

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Task, TaskFilter, TaskScope, TasksState};

pub(super) const FILTERS: [TaskFilter; 6] = [
    TaskFilter::All,
    TaskFilter::Backlog,
    TaskFilter::Running,
    TaskFilter::Done,
    TaskFilter::Failed,
    TaskFilter::Cancelled,
];

pub(super) struct TaskList<'a> {
    pub scoped_count: usize,
    pub counts: [usize; FILTERS.len()],
    pub visible: Vec<&'a Task>,
}

impl<'a> TaskList<'a> {
    pub fn project(
        tasks: &'a TasksState,
        scope: TaskScope,
        active: Option<ProjectUuid>,
        filter: TaskFilter,
        query: &str,
    ) -> Self {
        let query = query.trim().to_ascii_lowercase();
        let mut list = Self {
            scoped_count: 0,
            counts: [0; FILTERS.len()],
            visible: Vec::new(),
        };
        for task in tasks
            .tasks
            .iter()
            .filter(|task| scope.matches(task, active))
        {
            list.scoped_count += 1;
            if !query.is_empty() && !super::matches_task(task, &query) {
                continue;
            }
            for (count, bucket) in list.counts.iter_mut().zip(FILTERS) {
                *count += usize::from(bucket.matches(&task.state));
            }
            if filter.matches(&task.state) {
                list.visible.push(task);
            }
        }
        list.visible
            .sort_by_key(|task| std::cmp::Reverse(task.created_at));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::tasks::{SessionEndReason, TaskState};

    #[test]
    fn counts_apply_project_and_search_but_not_selected_status() {
        let project = ProjectUuid::new();
        let other = ProjectUuid::new();
        let mut tasks = TasksState::default();
        for (project, title, state) in [
            (project, "Fix login", TaskState::Backlog),
            (
                project,
                "Fix token",
                TaskState::Running {
                    worktree_path: "/tmp".into(),
                },
            ),
            (project, "Write docs", TaskState::Backlog),
            (other, "Fix settings", TaskState::Backlog),
        ] {
            let mut task = Task::new(project, title.into(), String::new(), None);
            task.state = state;
            tasks.add(task);
        }
        let list = TaskList::project(
            &tasks,
            TaskScope::ActiveProject,
            Some(project),
            TaskFilter::Running,
            " FIX ",
        );
        assert_eq!(list.scoped_count, 3);
        assert_eq!(list.counts, [2, 1, 1, 0, 0, 0]);
        assert_eq!(list.visible.len(), 1);
        assert_eq!(list.visible[0].title, "Fix token");
        let list = TaskList::project(
            &tasks,
            TaskScope::Project(other),
            Some(project),
            TaskFilter::All,
            "",
        );
        assert_eq!(list.scoped_count, 1);
        assert_eq!(list.visible[0].project, other);
        let list = TaskList::project(&tasks, TaskScope::AllProjects, None, TaskFilter::All, "fix");
        assert_eq!(list.counts[0], 3);
    }

    #[test]
    fn terminal_buckets_are_disjoint_and_empty_search_keeps_scope_count() {
        let project = ProjectUuid::default();
        let mut tasks = TasksState::default();
        for state in [
            TaskState::Done {
                worktree_path: "/tmp".into(),
                end_reason: SessionEndReason::Stop,
            },
            TaskState::Error {
                worktree_path: "/tmp".into(),
                message: "failed".into(),
            },
            TaskState::Cancelled {
                worktree_path: "/tmp".into(),
            },
        ] {
            let mut task = Task::new(project, "Task".into(), String::new(), None);
            task.state = state;
            tasks.add(task);
        }
        let list = TaskList::project(
            &tasks,
            TaskScope::ActiveProject,
            Some(project),
            TaskFilter::Done,
            "",
        );
        assert_eq!(list.counts, [3, 0, 0, 1, 1, 1]);
        assert_eq!(list.visible.len(), 1);
        let list = TaskList::project(
            &tasks,
            TaskScope::ActiveProject,
            Some(project),
            TaskFilter::All,
            "not present",
        );
        assert_eq!(list.scoped_count, 3);
        assert_eq!(list.counts, [0; 6]);
        assert!(list.visible.is_empty());
    }
}
