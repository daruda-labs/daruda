//! Tasks that run as a flow: where their run lives, and how the run's start,
//! end and stop reach the task log.
//!
//! A task and its run are joined by the run id the task recorded when it
//! asked for the run, never by a field on the run. A run picked up again
//! after a restart is a new episode with no memory of who asked for it; the
//! id finds the task all the same.

use std::path::{Path, PathBuf};

use daruda_flow::event::RunEnd;
use daruda_store::tasks::machine::{SettleOutcome, StartFailure, StopCause};
use daruda_store::tasks::{
    ExecutionRef, Lifecycle, SessionEndReason, TaskEvent, TaskExecution, TasksBook,
};
use gpui::App;

use super::{GlobalTasks, RunsHost};

/// Per-repository daruda directory, checked in with the repo.
pub(crate) const REPO_DIR: &str = ".daruda";
/// One directory per run. `.gitignore`d by the engine on first run.
const RUNS_DIR: &str = "flow-runs";

/// Where a lane's flow runs live.
pub(crate) fn runs_dir(lane_cwd: &Path) -> PathBuf {
    lane_cwd.join(REPO_DIR).join(RUNS_DIR)
}

/// The run directory of a task's flow run; `None` for any other run.
pub(crate) fn task_run_dir(run: &TaskExecution) -> Option<PathBuf> {
    run.source
        .flow_run_id()
        .map(|run_id| runs_dir(&run.cwd).join(run_id))
}

/// The task whose current run is the flow run in `run_dir`. By the run's
/// id alone: ids are unique, and a worktree's path has many spellings.
fn task_of_run<'a>(run_dir: &Path, book: &'a TasksBook) -> Option<(ExecutionRef, &'a Lifecycle)> {
    let run_id = run_dir.file_name()?.to_str()?;
    book.records().find_map(|record| {
        let run = record.execution.as_ref()?;
        let exec = record.current_exec()?;
        (run.id == exec.execution_id && run.source.flow_run_id() == Some(run_id))
            .then(|| (exec.clone(), &record.lifecycle))
    })
}

/// What a run's end means for the task that started it. `started` says the
/// run got as far as recording its start; one that ended before that was
/// refused under its lock, and failed the task's start rather than its run.
/// `None` leaves the task as it is: a run that stalled or was cut short by a
/// quit resumes, and the task runs on until it ends.
pub(super) fn task_end(
    end: &RunEnd,
    overage: bool,
    started: bool,
    result: Option<String>,
    exec: ExecutionRef,
    at: chrono::DateTime<chrono::Utc>,
) -> Option<TaskEvent> {
    if matches!(end, RunEnd::ShutDown | RunEnd::Stalled { .. }) {
        return None;
    }
    if !started {
        return Some(TaskEvent::DispatchFailed {
            exec,
            cause: StartFailure::FlowRefused,
            at,
        });
    }
    let outcome = match end {
        RunEnd::Done => SettleOutcome::Completed {
            end_reason: SessionEndReason::Stop,
        },
        RunEnd::Canceled { .. } => SettleOutcome::Stopped {
            cause: if overage {
                StopCause::ExtraUsage
            } else {
                StopCause::UserStop
            },
        },
        // Another process holds the worktree: the run is still its.
        RunEnd::LockHeld { .. } => return None,
        RunEnd::Failed { .. }
        | RunEnd::BudgetExhausted { .. }
        | RunEnd::Io { .. }
        | RunEnd::Log { .. }
        | RunEnd::Unprovisioned { .. }
        | RunEnd::Invalid { .. }
        | RunEnd::NotResumed { .. } => SettleOutcome::Errored,
        RunEnd::ShutDown | RunEnd::Stalled { .. } => return None,
    };
    Some(TaskEvent::Settled {
        result,
        exec,
        outcome,
        at,
    })
}

impl RunsHost {
    /// The run in `run_dir` recorded its start: a task waiting on it is now
    /// running.
    pub(super) fn flow_task_started(run_dir: &Path, cx: &mut App) {
        let Some(tasks) = cx.try_global::<GlobalTasks>() else {
            return;
        };
        if let Some((exec, Lifecycle::Dispatching { .. })) = task_of_run(run_dir, tasks.book()) {
            Self::propose_fact(
                TaskEvent::Dispatched {
                    exec,
                    session_id: None,
                    at: chrono::Utc::now(),
                },
                cx,
            );
        }
    }

    /// The run in `run_dir` ended as `end`: settle the task it is the run
    /// of, with `result`, the run's report. `overage` says a cancel was the
    /// usage limit's, not a person's; `saw_start` that this episode recorded
    /// its start.
    ///
    /// Whether the run started is the pump's to say, not the task book's:
    /// the `Dispatched` its start proposed may not have reached the book
    /// yet, and the log takes proposals in order, so a settle proposed now
    /// lands after it.
    pub(super) fn flow_task_ended(
        run_dir: &Path,
        end: &RunEnd,
        overage: bool,
        saw_start: bool,
        result: Option<String>,
        cx: &mut App,
    ) {
        let Some(tasks) = cx.try_global::<GlobalTasks>() else {
            return;
        };
        let event = task_of_run(run_dir, tasks.book()).and_then(|(exec, lifecycle)| {
            let started = match lifecycle {
                Lifecycle::Dispatching { .. } => saw_start,
                // A resumed run records no start of its own.
                Lifecycle::Running { .. } => true,
                _ => return None,
            };
            task_end(end, overage, started, result, exec, chrono::Utc::now())
        });
        if let Some(event) = event {
            Self::propose_fact(event, cx);
        }
    }

    /// A task's flow run was cancelled: stop the run, if this process holds
    /// it. Only the run the task started — a later one in the same worktree
    /// is someone else's.
    pub(super) fn stop_task_flow(exec: &ExecutionRef, cx: &mut App) {
        let Some(run_dir) = cx
            .try_global::<GlobalTasks>()
            .and_then(|tasks| exec.resolve(tasks.book()))
            .and_then(|run| Some((task_run_dir(run)?, run.cwd.clone())))
        else {
            return;
        };
        let (run_dir, worktree) = run_dir;
        if Self::flow_run(&worktree, cx)
            .is_some_and(|run| run.view().run_dir().file_name() == run_dir.file_name())
        {
            Self::stop_flow(&worktree, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flow_runs_directory_is_named_by_its_run_id() {
        let run = TaskExecution {
            source: daruda_store::tasks::TaskExecutionSource::Flow {
                run_id: "r1".into(),
                flow: "review.yaml".into(),
            },
            ..TaskExecution::begin(
                Default::default(),
                String::new(),
                None,
                PathBuf::from("/lane"),
            )
        };
        assert_eq!(
            task_run_dir(&run),
            Some(PathBuf::from("/lane/.daruda/flow-runs/r1"))
        );
        let chat = TaskExecution::begin(Default::default(), String::new(), None, "/lane".into());
        assert_eq!(task_run_dir(&chat), None);
    }

    fn exec() -> ExecutionRef {
        ExecutionRef {
            task_id: "t".into(),
            execution_id: "e".into(),
        }
    }

    fn outcome(end: RunEnd, overage: bool) -> Option<SettleOutcome> {
        match task_end(&end, overage, true, None, exec(), chrono::Utc::now())? {
            TaskEvent::Settled { outcome, .. } => Some(outcome),
            other => panic!("a running task settles: {other:?}"),
        }
    }

    #[test]
    fn a_runs_end_settles_its_task_as_the_run_ended() {
        assert_eq!(
            outcome(RunEnd::Done, false),
            Some(SettleOutcome::Completed {
                end_reason: SessionEndReason::Stop
            })
        );
        assert_eq!(
            outcome(RunEnd::Canceled { node: None }, false),
            Some(SettleOutcome::Stopped {
                cause: StopCause::UserStop
            })
        );
        assert_eq!(
            outcome(RunEnd::Canceled { node: None }, true),
            Some(SettleOutcome::Stopped {
                cause: StopCause::ExtraUsage
            })
        );
        assert_eq!(
            outcome(
                RunEnd::NotResumed {
                    message: String::new()
                },
                false
            ),
            Some(SettleOutcome::Errored)
        );
    }

    /// A run that can still be resumed leaves its task running.
    #[test]
    fn a_resumable_run_leaves_its_task_running() {
        assert_eq!(outcome(RunEnd::ShutDown, false), None);
        assert_eq!(outcome(RunEnd::Stalled { nodes: vec![] }, false), None);
        assert_eq!(
            outcome(
                RunEnd::LockHeld {
                    holder: daruda_flow::lock::LockHolder {
                        pid: 1,
                        run_id: String::new(),
                        started_unix_secs: 0,
                    }
                },
                false
            ),
            None
        );
    }

    /// A run that ended before it recorded its start was refused: the task
    /// failed to start rather than ran and failed.
    #[test]
    fn a_run_refused_before_it_started_fails_the_tasks_start() {
        let invalid = RunEnd::Invalid { issues: vec![] };
        assert!(matches!(
            task_end(&invalid, false, false, None, exec(), chrono::Utc::now()),
            Some(TaskEvent::DispatchFailed {
                cause: StartFailure::FlowRefused,
                ..
            })
        ));
    }
}
