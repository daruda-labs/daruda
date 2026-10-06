//! In-window grouping preferences and stable, disjoint task sections.

use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{Task, TaskFilter};

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

    pub fn reveal(&mut self, task: &Task) {
        self.collapsed.retain(|key| !key.matches(task));
    }
}

impl TaskGroupKey {
    fn matches(self, task: &Task) -> bool {
        match self {
            Self::Status(filter) => filter.matches(&task.state),
            Self::Project(project) => task.project == project,
        }
    }
}

pub(super) struct TaskGroup<'a> {
    pub key: TaskGroupKey,
    pub tasks: Vec<&'a Task>,
}

pub(super) fn project<'a>(
    visible: &[&'a Task],
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
                let key = TaskGroupKey::Project(task.project);
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
    use daruda_store::tasks::TaskState;

    #[test]
    fn groups_keep_filtered_order_and_include_closed_projects() {
        let project = ProjectUuid::new();
        let closed = ProjectUuid::new();
        let a = Task::new(project, "A".into(), String::new(), None);
        let mut b = Task::new(closed, "B".into(), String::new(), None);
        b.state = TaskState::Cancelled {
            worktree_path: "/tmp".into(),
        };
        let c = Task::new(project, "C".into(), String::new(), None);
        let visible = [&c, &b, &a];
        let groups = project_groups(&visible, project);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups[0]
                .tasks
                .iter()
                .map(|t| t.title.as_str())
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

    fn project_groups<'a>(visible: &[&'a Task], id: ProjectUuid) -> Vec<TaskGroup<'a>> {
        super::project(visible, TaskGrouping::Project, &[(id, "App".into())])
    }

    #[test]
    fn saving_reveals_only_its_group_and_mode_changes_reset_folds() {
        let task = Task::new(ProjectUuid::new(), "Task".into(), String::new(), None);
        let key = TaskGroupKey::Project(task.project);
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
