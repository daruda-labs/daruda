//! In-window grouping preferences and stable, disjoint task sections.

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{TaskFilter, TaskRecord};

use super::list::FILTERS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum TaskGrouping {
    #[default]
    None,
    Status,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TaskGroupKey {
    Status(TaskFilter),
    Project(ProjectUuid),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct TaskGroups {
    pub mode: TaskGrouping,
    collapsed: Vec<TaskGroupKey>,
}

impl TaskGroups {
    pub fn set_mode(&mut self, mode: TaskGrouping) {
        if self.mode != mode {
            self.mode = mode;
            self.collapsed.clear();
        }
    }

    pub fn is_open(&self, key: TaskGroupKey) -> bool {
        !self.collapsed.contains(&key)
    }

    pub fn toggle(&mut self, key: TaskGroupKey) {
        if self.is_open(key) {
            self.collapsed.push(key);
        } else {
            self.collapsed.retain(|collapsed| *collapsed != key);
        }
    }

    pub fn reveal(&mut self, task: &TaskRecord) {
        self.collapsed.retain(|key| !key.matches(task));
    }
}

impl TaskGroupKey {
    fn matches(self, task: &TaskRecord) -> bool {
        match self {
            Self::Status(filter) => filter.matches(&task.lifecycle),
            Self::Project(project) => task.spec.project == project,
        }
    }
}

pub(super) struct TaskGroup<'a> {
    pub key: TaskGroupKey,
    pub tasks: Vec<&'a TaskRecord>,
}

pub(super) fn project<'a>(
    visible: &[&'a TaskRecord],
    mode: TaskGrouping,
    projects: &[(ProjectUuid, String)],
) -> Vec<TaskGroup<'a>> {
    let keys = match mode {
        TaskGrouping::None => return Vec::new(),
        TaskGrouping::Status => FILTERS
            .into_iter()
            .skip(1)
            .map(TaskGroupKey::Status)
            .collect(),
        TaskGrouping::Project => {
            let mut keys: Vec<_> = projects
                .iter()
                .map(|(id, _)| TaskGroupKey::Project(*id))
                .collect();
            for task in visible {
                let key = TaskGroupKey::Project(task.spec.project);
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
            keys
        }
    };
    keys.into_iter()
        .filter_map(|key| {
            let tasks: Vec<_> = visible
                .iter()
                .copied()
                .filter(|task| key.matches(task))
                .collect();
            (!tasks.is_empty()).then_some(TaskGroup { key, tasks })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::seed::{lifecycle, record};
    use daruda_store::tasks::{Lifecycle, LifecycleKind};

    #[test]
    fn groups_keep_filtered_order_and_include_closed_projects() {
        let project = ProjectUuid::new();
        let closed = ProjectUuid::new();
        let a = record(project, "A", Lifecycle::Backlog);
        let b = record(closed, "B", lifecycle(LifecycleKind::Cancelled));
        let c = record(project, "C", Lifecycle::Backlog);
        let visible = [&c, &b, &a];
        let groups = project_groups(&visible, project);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]
                .tasks
                .iter()
                .map(|t| t.spec.title.as_str())
                .collect::<Vec<_>>(),
            ["C", "A"]
        );
        assert_eq!(groups[1].key, TaskGroupKey::Project(closed));
        let groups = super::project(&visible, TaskGrouping::Status, &[]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].key, TaskGroupKey::Status(TaskFilter::Backlog));
        assert_eq!(groups[1].key, TaskGroupKey::Status(TaskFilter::Cancelled));
        assert_eq!(
            groups.iter().map(|g| g.tasks.len()).sum::<usize>(),
            visible.len()
        );
    }

    fn project_groups<'a>(visible: &[&'a TaskRecord], id: ProjectUuid) -> Vec<TaskGroup<'a>> {
        super::project(visible, TaskGrouping::Project, &[(id, "App".into())])
    }

    #[test]
    fn saving_reveals_only_its_group_and_mode_changes_reset_folds() {
        let task = record(ProjectUuid::new(), "Task", Lifecycle::Backlog);
        let key = TaskGroupKey::Project(task.spec.project);
        let other = TaskGroupKey::Project(ProjectUuid::new());
        let mut groups = TaskGroups::default();
        groups.set_mode(TaskGrouping::Project);
        groups.toggle(key);
        groups.toggle(other);
        groups.reveal(&task);
        assert!(groups.is_open(key));
        assert!(!groups.is_open(other));
        groups.set_mode(TaskGrouping::Status);
        assert!(groups.is_open(other));
    }
}
