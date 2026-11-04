//! Starting a task that runs as a flow.
//!
//! The start is checked and recorded like any other: the flow is resolved and
//! its inputs checked before anything is recorded, so a task whose flow cannot
//! run stays in Backlog. Then the lane is made or found — a new worktree opens
//! no pane, since the run is what works there — `LanePrepared` records where
//! it runs, and the flow is submitted under the run id the start recorded.
//! The run's own start (`RunStarted`) is what moves the task to Running; see
//! `crate::runs::flow_tasks`.

use std::path::{Path, PathBuf};

use chrono::Utc;
use daruda_flow::input::InputValues;
use daruda_store::project::LaneRef;
use daruda_store::tasks::{
    ExecutionRef, Lifecycle, StartedBy, TaskEvent, TaskExecutionSource, TaskFlow, TaskRecord,
    TaskRunIn,
};
use gpui::{Context, Window};

use super::task_existing_lane_ops::ExistingLaneStart;
use super::task_start::{
    Answer, StartedOn, TaskStartError, TaskStarted, WorktreeStart, abandon_start,
};
use crate::runs::{GlobalTasks, RunsHost};
use crate::workspace::Workspace;
use crate::workspace::flow_ops::FlowStartRefused;
use crate::workspace::flow_request::FlowSelection;
use crate::workspace::lane_ops::LaneReveal;
use crate::workspace::task_flow::{TaskFlowProblem, TaskFlowTarget};

/// A flow task's start, checked before it is recorded.
pub(super) struct TaskFlowStart {
    target: TaskFlowTarget,
    inputs: InputValues,
    /// Chosen now so the start can record it: the task finds its run by it.
    run_id: String,
    started_by: StartedBy,
    place: FlowPlace,
}

/// Where the run goes.
enum FlowPlace {
    Existing(ExistingLaneStart),
    /// Holds the repository's lock until the worktree exists.
    New(WorktreeStart),
}

impl TaskFlowStart {
    /// What the start records as driving the run.
    pub(super) fn source(&self) -> TaskExecutionSource {
        TaskExecutionSource::Flow {
            run_id: self.run_id.clone(),
            flow: self.target.label.clone(),
        }
    }

    /// The repository whose lock the start holds, if it makes a worktree.
    pub(super) fn repo_lock(&self) -> Option<&Path> {
        match &self.place {
            FlowPlace::New(start) => Some(&start.plan.repo_root),
            FlowPlace::Existing(_) => None,
        }
    }
}

impl Workspace {
    /// Check `task`'s flow can run where the task runs, before the start is
    /// recorded. A refusal leaves the task in Backlog.
    pub(super) fn prepare_task_flow_start(
        &mut self,
        task: &TaskRecord,
        flow: &TaskFlow,
        reveal: LaneReveal,
        started_by: StartedBy,
        cx: &mut Context<Self>,
    ) -> Result<TaskFlowStart, TaskStartError> {
        let spec = &task.spec;
        if self.project_by_uuid(spec.project).is_none() {
            return Err(TaskStartError::ProjectNotOpen);
        }
        let base = match spec.run_in {
            TaskRunIn::NewWorktree => spec.base_worktree_path.as_deref(),
            TaskRunIn::ExistingLane { .. } => None,
        };
        let target = self
            .resolve_task_flow(spec.project, &spec.run_in, base, flow)
            .map_err(flow_problem_error)?;
        let inputs = target.inputs(&spec.prompt);
        // The prompt is checked as the value the flow will be given: a
        // number input refuses words here, not after a worktree was made.
        if let Err(issues) = daruda_flow::input::resolve(&target.decls, &inputs) {
            return Err(TaskStartError::FlowRefused {
                shown: crate::workspace::flow_ops::issue_report(&issues),
                detail: issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            });
        }
        let place = match &spec.run_in {
            TaskRunIn::ExistingLane { path } => {
                let lane = self.prepare_existing_lane_start(path, reveal)?;
                self.check_task_flow(lane.lane(), &target, cx)?;
                if self.lane_holder(lane.root()).is_some()
                    || self.is_flow_running_in(lane.lane(), cx)
                {
                    return Err(lane_busy());
                }
                FlowPlace::Existing(lane)
            }
            TaskRunIn::NewWorktree => {
                // Checked in the lane the flow was found in: the worktree it
                // will run in does not exist yet.
                if let Some(lane) = self.lane_ref_at(&target.source_lane) {
                    self.check_task_flow(lane, &target, cx)?;
                }
                FlowPlace::New(self.prepare_worktree_start(task, reveal)?)
            }
        };
        Ok(TaskFlowStart {
            target,
            inputs,
            run_id: self.next_run_id(cx),
            started_by,
            place,
        })
    }

    /// The flow's own static checks — the ones `Validate Flow…` runs — before
    /// the start is recorded, so a flow that will not run leaves the task in
    /// Backlog and makes no worktree.
    fn check_task_flow(
        &mut self,
        lane: LaneRef,
        target: &TaskFlowTarget,
        cx: &mut Context<Self>,
    ) -> Result<(), TaskStartError> {
        match self.check_flow(lane, &target.path, target.profile.as_deref(), cx) {
            Ok(issues) if issues.is_empty() => Ok(()),
            Ok(issues) => Err(TaskStartError::FlowRefused {
                shown: crate::workspace::flow_ops::issue_report(&issues),
                detail: format!("the flow has {} validation problem(s)", issues.len()),
            }),
            Err(error) => Err(submit_refused(error)),
        }
    }

    /// The start is recorded: make or find the lane, then launch the run.
    pub(super) fn continue_task_flow_start(
        &mut self,
        start: TaskFlowStart,
        exec: ExecutionRef,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TaskFlowStart {
            target,
            inputs,
            run_id,
            started_by,
            place,
        } = start;
        let launch = FlowLaunch {
            target,
            inputs,
            run_id,
            started_by,
        };
        match place {
            FlowPlace::Existing(lane) => {
                if lane.reveal() == LaneReveal::Foreground {
                    self.activate_lane(lane.lane(), window, cx);
                }
                let root = lane.root().to_path_buf();
                self.launch_task_flow(exec, lane.lane(), root, launch, answer, cx);
            }
            FlowPlace::New(start) => {
                self.create_flow_task_worktree(start, exec, launch, answer, window, cx)
            }
        }
    }

    /// `git worktree add` off the main thread, then register the lane with
    /// nothing open in it and launch the run there.
    fn create_flow_task_worktree(
        &mut self,
        start: WorktreeStart,
        exec: ExecutionRef,
        launch: FlowLaunch,
        answer: Answer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let WorktreeStart {
            plan,
            project_id,
            reveal,
            ..
        } = start;
        cx.spawn_in(window, async move |ws, cx| {
            let added = cx
                .background_executor()
                .spawn({
                    let plan = plan.clone();
                    async move {
                        crate::lane::git::add_lane(
                            &plan.repo_root,
                            &plan.new_path,
                            Some(&plan.branch),
                            plan.base_ref.as_deref(),
                        )
                        .map_err(|e| e.to_string())
                    }
                })
                .await;
            let continued = ws.update_in(cx, |ws, window, cx| {
                // Released before registering, as a chat task's start does:
                // the worktree is on disk either way.
                ws.release_repo_lock(&plan.repo_root);
                if let Err(detail) = added {
                    ws.finish_task_start(
                        Err(TaskStartError::GitAddFailed { detail }),
                        Some(&exec),
                        &answer,
                        cx,
                    );
                    return;
                }
                if ws.project_for(project_id).is_none() {
                    ws.finish_task_start(
                        Err(TaskStartError::ProjectNotOpen),
                        Some(&exec),
                        &answer,
                        cx,
                    );
                    return;
                }
                let worktree = plan.new_path.clone();
                let lane = ws.register_created_lane(plan, project_id, None, reveal, window, cx);
                let mut launch = launch;
                launch.target.path = on_branch(&launch.target, &worktree);
                ws.launch_task_flow(exec.clone(), lane, worktree, launch, answer.clone(), cx);
            });
            if continued.is_err() {
                abandon_start(&exec, &answer, cx);
            }
        })
        .detach();
    }

    /// Record where the run goes, then submit it. Answers once the run is
    /// handed to the process; its start or refusal arrives on its own.
    fn launch_task_flow(
        &mut self,
        exec: ExecutionRef,
        lane: LaneRef,
        worktree: PathBuf,
        launch: FlowLaunch,
        answer: Answer,
        cx: &mut Context<Self>,
    ) {
        // A cancel that landed while the lane was made ends the start here.
        if !task_is(&exec, cx, |lifecycle| {
            matches!(lifecycle, Lifecycle::Preparing { .. })
        }) {
            self.finish_task_start(Err(TaskStartError::NotBacklog), None, &answer, cx);
            return;
        }
        let prepared = RunsHost::propose(
            TaskEvent::LanePrepared {
                terminal_id: None,
                exec: exec.clone(),
                worktree: worktree.clone(),
                agent_id: String::new(),
                account_id: None,
                cwd: worktree.clone(),
                at: Utc::now(),
            },
            cx,
        );
        cx.spawn(async move |ws, cx| {
            let recorded = prepared.await;
            let finished = ws.update(cx, |ws, cx| {
                let outcome = match recorded {
                    Ok(_) => ws.submit_task_flow(&exec, lane, worktree, launch, cx),
                    Err(error) => Err(TaskStartError::NotRecorded {
                        detail: error.to_string(),
                    }),
                };
                ws.finish_task_start(outcome, Some(&exec), &answer, cx);
            });
            if finished.is_err() {
                abandon_start(&exec, &answer, cx);
            }
        })
        .detach();
    }

    fn submit_task_flow(
        &mut self,
        exec: &ExecutionRef,
        lane: LaneRef,
        worktree: PathBuf,
        launch: FlowLaunch,
        cx: &mut Context<Self>,
    ) -> Result<TaskStarted, TaskStartError> {
        if !task_is(exec, cx, |lifecycle| {
            matches!(lifecycle, Lifecycle::Dispatching { .. })
        }) {
            return Err(TaskStartError::NotBacklog);
        }
        let FlowLaunch {
            target,
            inputs,
            run_id,
            started_by,
        } = launch;
        let selection = FlowSelection {
            inputs: Some(inputs),
            run_id: Some(run_id),
            ..FlowSelection::default()
        };
        let submitter = (flow_started_by(started_by), false);
        self.submit_flow_run(
            lane,
            &target.path,
            target.profile.as_deref(),
            &selection,
            submitter,
            cx,
        )
        .map_err(|refused| match refused {
            FlowStartRefused::Submit(error) => submit_refused(error),
            FlowStartRefused::AlreadyRunning => lane_busy(),
        })?;
        cx.notify();
        Ok(TaskStarted {
            exec: exec.clone(),
            worktree,
            on: StartedOn::Flow,
        })
    }
}

/// What the run is started with, once its lane exists.
struct FlowLaunch {
    target: TaskFlowTarget,
    inputs: InputValues,
    run_id: String,
    started_by: StartedBy,
}

/// Whether `exec` is still the task's current run, in a state `is` accepts.
fn task_is(exec: &ExecutionRef, cx: &gpui::App, is: impl Fn(&Lifecycle) -> bool) -> bool {
    cx.global::<GlobalTasks>()
        .get(&exec.task_id)
        .is_some_and(|task| task.current_exec() == Some(exec) && is(&task.lifecycle))
}

/// The flow run's record of who started it, from the task's.
fn flow_started_by(started_by: StartedBy) -> daruda_flow::request::StartedBy {
    match started_by {
        StartedBy::Desktop => daruda_flow::request::StartedBy::Desktop,
        StartedBy::Phone => daruda_flow::request::StartedBy::Phone,
        StartedBy::Orchestrator => daruda_flow::request::StartedBy::Orchestrator,
    }
}

/// A flow the task cannot run as described, as a start refusal.
fn flow_problem_error(problem: TaskFlowProblem) -> TaskStartError {
    match problem {
        TaskFlowProblem::NotFound => TaskStartError::FlowMissing,
        other => TaskStartError::FlowRefused {
            shown: crate::workspace::task_flow::problem_text(other),
            detail: other.to_string(),
        },
    }
}

/// A run the host would not assemble, in the person's words and the
/// control surface's.
fn submit_refused(error: crate::workspace::flow_request::FlowSubmitError) -> TaskStartError {
    let detail = crate::workspace::control_flow_ops::submit_problem(&error);
    let (title, more) = crate::workspace::flow_ops::flow_refusal_text(error);
    TaskStartError::FlowRefused {
        shown: if more.is_empty() {
            title
        } else {
            format!("{title}\n{more}")
        },
        detail,
    }
}

/// The flow a task runs in its new worktree: the branch's own copy of a
/// repository flow, which is what its relative files resolve against. A flow
/// the branch does not have — the person's own, or one not yet committed —
/// is run from where it was found.
fn on_branch(target: &TaskFlowTarget, worktree: &Path) -> PathBuf {
    let flows = crate::workspace::flow_paths::flows_dir(&target.source_lane);
    daruda_core::path::strip_root(&target.path, &flows)
        .map(|relative| crate::workspace::flow_paths::flows_dir(worktree).join(relative))
        .filter(|copy| copy.is_file())
        .unwrap_or_else(|| target.path.clone())
}

fn lane_busy() -> TaskStartError {
    TaskStartError::FlowRefused {
        shown: crate::surface::strings::task::error_flow_lane_busy(),
        detail: "a flow is already running in the worktree".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(source_lane: &Path, path: PathBuf) -> TaskFlowTarget {
        TaskFlowTarget {
            path,
            label: "review.yaml".into(),
            profile: None,
            input: None,
            decls: Vec::new(),
            source_lane: source_lane.to_path_buf(),
        }
    }

    /// A repository flow runs from the new branch's own copy, so its
    /// relative files are the branch's; one the branch lacks runs from where
    /// it was found.
    #[test]
    fn a_new_worktree_runs_its_own_copy_of_a_repository_flow() {
        let main = tempfile::tempdir().unwrap();
        let branch = tempfile::tempdir().unwrap();
        let flows = crate::workspace::flow_paths::flows_dir(main.path());
        let found = flows.join("review.yaml");
        let copy = crate::workspace::flow_paths::flows_dir(branch.path()).join("review.yaml");
        assert_eq!(
            on_branch(&target(main.path(), found.clone()), branch.path()),
            found
        );
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, "version: 1\nnodes: []\n").unwrap();
        assert_eq!(on_branch(&target(main.path(), found), branch.path()), copy);
        let global = PathBuf::from("/data/flows/review.yaml");
        assert_eq!(
            on_branch(&target(main.path(), global.clone()), branch.path()),
            global
        );
    }

    /// Two unrelated domain enums with the same variants: a swapped arm
    /// would compile, so the mapping is pinned here.
    #[test]
    fn a_tasks_starter_is_the_runs_starter() {
        use daruda_flow::request::StartedBy as Flow;
        assert_eq!(flow_started_by(StartedBy::Desktop), Flow::Desktop);
        assert_eq!(flow_started_by(StartedBy::Phone), Flow::Phone);
        assert_eq!(flow_started_by(StartedBy::Orchestrator), Flow::Orchestrator);
    }
}
