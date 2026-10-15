//! One projection drives visible rows and status counts.

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{TaskFilter, TaskRecord, TaskScope, TasksBook};

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
    pub visible: Vec<&'a TaskRecord>,
}

impl<'a> TaskList<'a> {
    pub fn project(
        tasks: &'a TasksBook,
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
            .records()
            .filter(|task| scope.matches(task.spec.project, active))
        {
            list.scoped_count += 1;
            if !query.is_empty() && !super::matches_task(task, &query) {
                continue;
            }
            for (count, bucket) in list.counts.iter_mut().zip(FILTERS) {
                *count += usize::from(bucket.matches(&task.lifecycle));
            }
            if filter.matches(&task.lifecycle) {
                list.visible.push(task);
            }
        }
        list.visible
            .sort_by_key(|task| std::cmp::Reverse(task.spec.created_at));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::seed::{book, lifecycle, record};
    use daruda_store::tasks::LifecycleKind as K;

    #[test]
    fn counts_apply_project_and_search_but_not_selected_status() {
        let project = ProjectUuid::new();
        let other = ProjectUuid::new();
        let tasks = book(vec![
            record(project, "Fix login", lifecycle(K::Backlog)),
            record(project, "Fix token", lifecycle(K::Running)),
            record(project, "Write docs", lifecycle(K::Backlog)),
            record(other, "Fix settings", lifecycle(K::Backlog)),
        ]);
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
        assert_eq!(list.visible[0].spec.title, "Fix token");
        let list = TaskList::project(
            &tasks,
            TaskScope::Project(other),
            Some(project),
            TaskFilter::All,
            "",
        );
        assert_eq!(list.scoped_count, 1);
        assert_eq!(list.visible[0].spec.project, other);
        let list = TaskList::project(&tasks, TaskScope::AllProjects, None, TaskFilter::All, "fix");
        assert_eq!(list.counts[0], 3);
    }

    #[test]
    fn terminal_buckets_are_disjoint_and_empty_search_keeps_scope_count() {
        let project = ProjectUuid::default();
        let tasks = book(
            [K::Done, K::Error, K::Cancelled]
                .into_iter()
                .map(|kind| record(project, "Task", lifecycle(kind)))
                .collect(),
        );
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

    /// The new states fold into the closest tab: starting runs as running,
    /// an undelivered prompt as failed, an interrupted run as cancelled.
    #[test]
    fn new_states_count_under_their_closest_tab() {
        let project = ProjectUuid::default();
        let tasks = book(
            [K::Preparing, K::Dispatching, K::Undelivered, K::Interrupted]
                .into_iter()
                .map(|kind| record(project, "Task", lifecycle(kind)))
                .collect(),
        );
        let list = TaskList::project(
            &tasks,
            TaskScope::ActiveProject,
            Some(project),
            TaskFilter::All,
            "",
        );
        assert_eq!(list.counts, [4, 0, 2, 0, 1, 1]);
    }
}
