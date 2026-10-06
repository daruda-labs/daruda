//! What the task list is narrowed to — window-local, never persisted.

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Task, TaskFilter, TaskScope};

use super::TaskGroups;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct TaskBrowserState {
    pub scope: TaskScope,
    pub filter: TaskFilter,
    pub groups: TaskGroups,
}

impl TaskBrowserState {
    /// Relax only the choices hiding `task` — its project, status and fold —
    /// and keep the rest. Search is the caller's: it lives in an input.
    pub fn reveal(&mut self, task: &Task, active: Option<ProjectUuid>) {
        if !self.scope.matches(task, active) {
            self.scope = TaskScope::Project(task.project);
        }
        if !self.filter.matches(&task.state) {
            self.filter = TaskFilter::All;
        }
        self.groups.reveal(task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::right_dock::tasks::{TaskGroupKey, TaskGrouping};

    #[test]
    fn reveal_relaxes_only_what_hides_the_task() {
        let active = ProjectUuid::new();
        let other = ProjectUuid::new();
        let task = Task::new(other, "Task".into(), String::new(), None);
        let mut state = TaskBrowserState {
            scope: TaskScope::ActiveProject,
            filter: TaskFilter::Failed,
            groups: TaskGroups::default(),
        };
        state.groups.set_mode(TaskGrouping::Project);
        state.groups.toggle(TaskGroupKey::Project(other));
        state.groups.toggle(TaskGroupKey::Project(active));
        state.reveal(&task, Some(active));
        assert_eq!(state.scope, TaskScope::Project(other));
        assert_eq!(state.filter, TaskFilter::All);
        assert!(state.groups.is_open(TaskGroupKey::Project(other)));
        assert!(!state.groups.is_open(TaskGroupKey::Project(active)));

        let mut kept = TaskBrowserState {
            scope: TaskScope::AllProjects,
            filter: TaskFilter::Backlog,
            groups: TaskGroups::default(),
        };
        kept.reveal(&task, Some(active));
        assert_eq!(kept.scope, TaskScope::AllProjects);
        assert_eq!(kept.filter, TaskFilter::Backlog);
    }
}
