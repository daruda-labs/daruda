//! Where the TaskEdit form runs its task — a worktree it creates, or a lane
//! already registered — and the branch checks that choice implies.

use std::path::Path;

use daruda_store::project::{LaneKind, ProjectUuid};
use daruda_store::tasks::{Lifecycle, TaskRecord, TaskRunIn, TasksBook};
use gpui::{Context, SharedString};

use super::TaskEditorId;
use super::state::{BranchValidation, RunInChoice, TaskEditContent};
use crate::runs::GlobalTasks;
use crate::ui::select::SelectOption;
use crate::workspace::Workspace;

use super::task_edit_ops::validate_branch;

/// The picker's options: every lane of `project`, by path.
pub(super) fn lane_options(ws: &Workspace, project: Option<ProjectUuid>) -> Vec<SelectOption> {
    ws.task_project_lanes(project)
        .iter()
        .filter_map(|lane| {
            let path = lane.path.to_str()?.to_string();
            Some(SelectOption::new(path, lane.display_name()))
        })
        .collect()
}

/// The lane a draft or saved task preselects: its own, else the active one
/// when that lane is in the task's project.
pub(super) fn initial_lane(
    task: Option<&TaskRecord>,
    ws: &Workspace,
    project: Option<ProjectUuid>,
) -> Option<SharedString> {
    let path = match task.map(|t| &t.spec.run_in) {
        Some(TaskRunIn::ExistingLane { path }) => path.clone(),
        _ if ws.active_project().map(|p| p.uuid) == project => ws.active_lane()?.path.clone(),
        _ => return None,
    };
    path.to_str().map(|s| SharedString::from(s.to_string()))
}

pub(super) fn run_in_choice(task: Option<&TaskRecord>) -> RunInChoice {
    match task.map(|t| &t.spec.run_in) {
        Some(TaskRunIn::ExistingLane { .. }) => RunInChoice::ExistingLane,
        _ => RunInChoice::NewWorktree,
    }
}

/// Whether a form's task has yet to start — a draft (`None`) or a Backlog
/// task — so where it runs, its branch and Start are still open to it.
pub(in crate::workspace) fn not_started(task: Option<&TaskRecord>) -> bool {
    task.is_none_or(|task| task.lifecycle == Lifecycle::Backlog)
}

/// [`not_started`] for the task `te` edits.
pub(in crate::workspace) fn location_editable(te: &TaskEditContent, tasks: &GlobalTasks) -> bool {
    not_started(te.task_id.as_deref().and_then(|id| tasks.get(id)))
}

/// Another task already running (or handing its prompt over) in `lane`,
/// for the picker's warning.
pub(in crate::workspace) fn task_running_in<'a>(
    tasks: &'a TasksBook,
    lane: &Path,
    except: Option<&str>,
) -> Option<&'a TaskRecord> {
    tasks.records().find(|task| {
        Some(task.id()) != except
            && matches!(&task.lifecycle,
                Lifecycle::Dispatching { worktree, .. } | Lifecycle::Running { worktree, .. }
                    if daruda_core::path::same_path(worktree, lane))
    })
}

impl Workspace {
    /// `validate_branch`, plus — while the location is still editable — the
    /// check git would otherwise fail at Start: a registered lane already
    /// has this branch checked out. A started task's branch is its own
    /// lane's, so it skips that check.
    pub(super) fn branch_validation_for(
        &self,
        text: &str,
        editable: bool,
        project: Option<ProjectUuid>,
    ) -> BranchValidation {
        let validation = validate_branch(text);
        if validation != BranchValidation::Valid || !editable {
            return validation;
        }
        let branch = text.trim();
        let taken = self
            .task_project_lanes(project)
            .iter()
            .any(|lane| matches!(&lane.kind, LaneKind::Git { branch: Some(b), .. } if b == branch));
        if taken {
            BranchValidation::Exists
        } else {
            validation
        }
    }

    /// The lanes of the project a task form names; none when that project
    /// is not open here.
    pub(super) fn task_project_lanes(&self, project: Option<ProjectUuid>) -> &[crate::lane::Lane] {
        project
            .and_then(|uuid| self.project_by_uuid(uuid))
            .map_or(&[], |p| p.lanes.as_slice())
    }

    pub(super) fn set_task_run_in(
        &mut self,
        editor_id: TaskEditorId,
        choice: RunInChoice,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_editor_mut(editor_id) {
            te.run_in = choice;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use daruda_store::tasks::{ExecutionRef, Lifecycle, TaskRecord};

    use super::task_running_in;
    use crate::runs::seed::{book, record};

    fn running(title: &str, lane: &str) -> TaskRecord {
        record(
            daruda_store::project::ProjectUuid::default(),
            title,
            Lifecycle::Running {
                exec: ExecutionRef {
                    task_id: title.into(),
                    execution_id: "run".into(),
                },
                worktree: PathBuf::from(lane),
            },
        )
    }

    #[test]
    fn a_running_task_in_the_lane_is_found_but_not_the_task_itself() {
        let own = running("Own", "/repo/main");
        let own_id = own.id().to_owned();
        let tasks = book(vec![own, running("Elsewhere", "/repo/other")]);
        let lane = PathBuf::from("/repo/main");
        assert_eq!(
            task_running_in(&tasks, &lane, None).map(|t| t.spec.title.as_str()),
            Some("Own")
        );
        assert!(task_running_in(&tasks, &lane, Some(&own_id)).is_none());
    }
}
