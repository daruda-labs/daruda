//! A new task's definition, assembled once for every caller that creates one.
//!
//! The Task editor and the control surface both create tasks. Assembling the
//! spec in one place keeps them refusing the same branch and the same lane:
//! a task the orchestrator adds must not run somewhere the form would refuse.

use std::path::PathBuf;

use chrono::Utc;
use daruda_store::project::{LaneKind, ProjectUuid};
use daruda_store::tasks::{TaskAgentSurface, TaskRunIn, TaskSpec};

use crate::workspace::Workspace;

/// What a caller asks for, before it is checked against the project.
#[derive(Clone, Debug)]
pub(in crate::workspace) struct TaskDraft {
    pub title: String,
    pub prompt: String,
    pub notes: String,
    /// As typed. Empty keeps the task's default `task-<id>` branch.
    pub branch: String,
    pub location: DraftLocation,
    pub agent_surface: TaskAgentSurface,
    pub auto_execute: bool,
}

/// Where a drafted task runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum DraftLocation {
    /// A worktree Start creates, branched from `base` when one is named.
    NewWorktree { base: Option<PathBuf> },
    /// A lane already registered in the task's project.
    ExistingLane(PathBuf),
}

/// Where a task runs, once checked: no base for an existing lane, and a
/// branch only when one was named for a worktree Start creates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct TaskPlacement {
    pub run_in: TaskRunIn,
    pub base: Option<PathBuf>,
    pub branch: Option<String>,
}

/// Why a draft cannot become a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum DraftError {
    TitleEmpty,
    /// The project is not open in this window.
    ProjectNotOpen,
    /// The lane is not one of the project's, so the task would point at a
    /// repository it does not belong to.
    LaneNotInProject,
    /// The branch breaks a git ref-name rule.
    BranchInvalid,
    /// A lane of the project already checks the branch out, so
    /// `git worktree add -b` would refuse it at Start.
    BranchTaken,
}

impl Workspace {
    /// The spec a new task in `project` is created with.
    ///
    /// The branch is checked only for a worktree Start creates: an existing
    /// lane keeps the branch it has, so its draft takes neither the typed
    /// branch nor a base — a branch nobody was shown must not be created if
    /// the task is later moved to a new worktree.
    pub(in crate::workspace) fn task_spec_from(
        &self,
        project: ProjectUuid,
        draft: TaskDraft,
    ) -> Result<TaskSpec, DraftError> {
        let title = draft.title.trim();
        if title.is_empty() {
            return Err(DraftError::TitleEmpty);
        }
        let placement = self.task_placement(project, draft.location, &draft.branch)?;
        let mut spec = TaskSpec::new(
            project,
            title.to_owned(),
            draft.prompt,
            placement.base,
            Utc::now(),
        );
        if let Some(branch) = placement.branch {
            spec.branch_name = branch;
        }
        spec.run_in = placement.run_in;
        spec.notes = draft.notes;
        spec.agent_surface = draft.agent_surface;
        spec.auto_execute = draft.auto_execute;
        Ok(spec)
    }

    /// Where a task of `project` runs and on which branch, checked as every
    /// caller that places one must check it — creating a task and editing a
    /// task that has not started alike.
    pub(in crate::workspace) fn task_placement(
        &self,
        project: ProjectUuid,
        location: DraftLocation,
        branch: &str,
    ) -> Result<TaskPlacement, DraftError> {
        let lanes = self
            .project_by_uuid(project)
            .map(|p| p.lanes.as_slice())
            .ok_or(DraftError::ProjectNotOpen)?;
        Ok(match location {
            DraftLocation::NewWorktree { base } => {
                let branch = match daruda_core::git::validate_branch_name(branch) {
                    Ok(name) if branch_checked_out(lanes, name) => {
                        return Err(DraftError::BranchTaken);
                    }
                    Ok(name) => Some(name.to_owned()),
                    Err(daruda_core::git::BranchNameRule::Empty) => None,
                    Err(_) => return Err(DraftError::BranchInvalid),
                };
                TaskPlacement {
                    run_in: TaskRunIn::NewWorktree,
                    base,
                    branch,
                }
            }
            DraftLocation::ExistingLane(path) => {
                if !lanes
                    .iter()
                    .any(|lane| daruda_core::path::same_path(&lane.path, &path))
                {
                    return Err(DraftError::LaneNotInProject);
                }
                TaskPlacement {
                    run_in: TaskRunIn::ExistingLane { path },
                    base: None,
                    branch: None,
                }
            }
        })
    }

    /// Whether a lane of `project` already checks out `branch`.
    pub(in crate::workspace) fn task_branch_checked_out(
        &self,
        project: Option<ProjectUuid>,
        branch: &str,
    ) -> bool {
        project
            .and_then(|uuid| self.project_by_uuid(uuid))
            .is_some_and(|p| branch_checked_out(&p.lanes, branch))
    }
}

fn branch_checked_out(lanes: &[crate::lane::Lane], branch: &str) -> bool {
    lanes
        .iter()
        .any(|lane| matches!(&lane.kind, LaneKind::Git { branch: Some(b), .. } if b == branch))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gpui::TestAppContext;

    use super::*;

    fn draft(branch: &str, location: DraftLocation) -> TaskDraft {
        TaskDraft {
            title: " Fix login ".into(),
            prompt: "p".into(),
            notes: "n".into(),
            branch: branch.into(),
            location,
            agent_surface: TaskAgentSurface::Terminal,
            auto_execute: false,
        }
    }

    fn new_worktree() -> DraftLocation {
        DraftLocation::NewWorktree {
            base: Some(PathBuf::from("/base")),
        }
    }

    /// A project with one git lane on `main`, and that lane's path.
    fn workspace_with_a_main_lane(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        gpui::Entity<Workspace>,
        ProjectUuid,
        PathBuf,
    ) {
        let root = tempfile::tempdir().expect("tempdir");
        let project = daruda_store::project::Project::from_path(root.path());
        let (_window, ws) = crate::workspace::tests::build_workspace_with(
            cx,
            &daruda_config::Config::default(),
            Some(project),
        );
        let (uuid, path) = ws.update(cx, |ws, _| {
            let id = ws.alloc_id();
            let path = root.path().join("main-lane");
            let project = ws.project_for_mut(ws.active.project).expect("project");
            project.lanes.push(crate::lane::Lane::git(
                id,
                path.clone(),
                Some("main".into()),
                path.clone(),
                path.clone(),
                0,
            ));
            (project.uuid, path)
        });
        (root, ws, uuid, path)
    }

    #[gpui::test]
    fn a_new_worktree_draft_keeps_its_branch_base_and_fields(cx: &mut TestAppContext) {
        let (_root, ws, project, _) = workspace_with_a_main_lane(cx);
        ws.read_with(cx, |ws, _| {
            let spec = ws
                .task_spec_from(project, draft(" feat/x ", new_worktree()))
                .expect("valid");
            assert_eq!(spec.title, "Fix login");
            assert_eq!(spec.branch_name, "feat/x");
            assert_eq!(spec.run_in, TaskRunIn::NewWorktree);
            assert_eq!(spec.base_worktree_path, Some(PathBuf::from("/base")));
            assert_eq!(spec.notes, "n");
            assert_eq!(spec.agent_surface, TaskAgentSurface::Terminal);
            assert!(!spec.auto_execute);
            let default = ws
                .task_spec_from(project, draft("", new_worktree()))
                .expect("an empty branch is the default");
            assert_eq!(
                default.branch_name,
                daruda_store::tasks::branch_name_for(&default.id)
            );
        });
    }

    #[gpui::test]
    fn a_new_worktree_draft_refuses_what_git_would(cx: &mut TestAppContext) {
        let (_root, ws, project, _) = workspace_with_a_main_lane(cx);
        ws.read_with(cx, |ws, _| {
            assert_eq!(
                ws.task_spec_from(project, draft("a..b", new_worktree())),
                Err(DraftError::BranchInvalid)
            );
            assert_eq!(
                ws.task_spec_from(project, draft("main", new_worktree())),
                Err(DraftError::BranchTaken)
            );
            let mut empty = draft("", new_worktree());
            empty.title = "  ".into();
            assert_eq!(
                ws.task_spec_from(project, empty),
                Err(DraftError::TitleEmpty)
            );
            assert_eq!(
                ws.task_spec_from(ProjectUuid::new(), draft("", new_worktree())),
                Err(DraftError::ProjectNotOpen)
            );
        });
    }

    /// An existing lane has its own branch: the draft neither creates one
    /// nor branches from a base, so a taken branch is no refusal there.
    #[gpui::test]
    fn an_existing_lane_draft_needs_a_lane_of_the_project(cx: &mut TestAppContext) {
        let (_root, ws, project, lane) = workspace_with_a_main_lane(cx);
        ws.read_with(cx, |ws, _| {
            let spec = ws
                .task_spec_from(
                    project,
                    draft("main", DraftLocation::ExistingLane(lane.clone())),
                )
                .expect("a lane of the project");
            assert_eq!(spec.run_in, TaskRunIn::ExistingLane { path: lane });
            assert_eq!(spec.base_worktree_path, None);
            assert_eq!(
                spec.branch_name,
                daruda_store::tasks::branch_name_for(&spec.id),
                "the typed branch is not kept for a lane that has its own"
            );
            assert_eq!(
                ws.task_spec_from(
                    project,
                    draft("", DraftLocation::ExistingLane(PathBuf::from("/elsewhere")))
                ),
                Err(DraftError::LaneNotInProject)
            );
        });
    }
}
