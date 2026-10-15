//! Fixture task runs for tests and screenshot scenarios, built only from
//! events the task machine accepts.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use daruda_store::tasks::machine::{SettleOutcome, StartFailure, TaskSpec};
use daruda_store::tasks::{
    CliProcessState, ExecutionRef, SessionEndReason, TaskEvent, TaskExecutionSource,
};
use gpui::App;

use super::RunsHost;

/// One run of a seeded task, from its start request to where it stands.
pub(crate) struct SeedRun {
    pub(crate) source: TaskExecutionSource,
    pub(crate) agent_id: String,
    pub(crate) cwd: PathBuf,
    pub(crate) session_id: Option<String>,
    pub(crate) execution_id: String,
    /// CLI sessions attached once the run is up.
    pub(crate) attached: Vec<String>,
}

impl SeedRun {
    pub(crate) fn chat(cwd: impl Into<PathBuf>) -> Self {
        Self {
            source: TaskExecutionSource::AgentChat,
            agent_id: "claude".into(),
            cwd: cwd.into(),
            session_id: None,
            execution_id: uuid::Uuid::new_v4().to_string(),
            attached: Vec::new(),
        }
    }

    pub(crate) fn cli(cwd: impl Into<PathBuf>, process: CliProcessState) -> Self {
        Self {
            source: TaskExecutionSource::ClaudeCli {
                transcript_path: None,
                process,
            },
            ..Self::chat(cwd)
        }
    }

    pub(crate) fn agent(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = agent_id.into();
        self
    }

    pub(crate) fn session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn attach(mut self, session_id: impl Into<String>) -> Self {
        self.attached.push(session_id.into());
        self
    }

    pub(crate) fn exec(&self, task: &str) -> ExecutionRef {
        ExecutionRef {
            task_id: task.to_owned(),
            execution_id: self.execution_id.clone(),
        }
    }
}

/// Where a seeded run stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedEnd {
    Preparing,
    Dispatching,
    Running,
    Done,
    Errored,
    Cancelled,
    Interrupted,
    Undelivered,
    StartFailed,
}

/// Events that create `spec`.
pub(crate) fn created(spec: TaskSpec) -> Vec<TaskEvent> {
    vec![TaskEvent::Created { spec }]
}

/// Events that take a Backlog task `id` through `run` to `end`.
pub(crate) fn run(id: &str, run: &SeedRun, end: SeedEnd) -> Vec<TaskEvent> {
    let at = |n: i64| -> DateTime<Utc> { Utc::now() + chrono::Duration::milliseconds(n) };
    let exec = run.exec(id);
    let mut events = vec![TaskEvent::DispatchRequested {
        id: id.to_owned(),
        execution_id: run.execution_id.clone(),
        source: match &run.source {
            TaskExecutionSource::AgentChat => TaskExecutionSource::AgentChat,
            TaskExecutionSource::ClaudeCli { .. } => TaskExecutionSource::ClaudeCli {
                transcript_path: None,
                process: CliProcessState::Discovering,
            },
        },
        at: at(0),
    }];
    if end == SeedEnd::Preparing {
        return events;
    }
    if end == SeedEnd::StartFailed {
        events.push(TaskEvent::DispatchFailed {
            exec,
            cause: StartFailure::GitAddFailed,
            at: at(1),
        });
        return events;
    }
    events.push(TaskEvent::LanePrepared {
        exec: exec.clone(),
        worktree: run.cwd.clone(),
        agent_id: run.agent_id.clone(),
        account_id: None,
        cwd: run.cwd.clone(),
        at: at(1),
    });
    if let TaskExecutionSource::ClaudeCli {
        transcript_path,
        process,
    } = &run.source
    {
        if *process != CliProcessState::Discovering {
            events.push(TaskEvent::CliProcessObserved {
                exec: exec.clone(),
                process: process.clone(),
                at: at(2),
            });
        }
        if let Some(path) = transcript_path {
            events.push(TaskEvent::CliTranscriptFound {
                exec: exec.clone(),
                transcript_path: path.clone(),
                at: at(2),
            });
        }
    }
    match end {
        SeedEnd::Dispatching => {
            if let Some(session_id) = &run.session_id {
                events.push(TaskEvent::ExecutionSessionBound {
                    exec,
                    session_id: session_id.clone(),
                    at: at(3),
                });
            }
            return events;
        }
        SeedEnd::Undelivered => {
            events.push(TaskEvent::Undelivered {
                exec,
                reason: daruda_store::tasks::machine::UndeliveredReason::PaneClosed,
                at: at(3),
            });
            return events;
        }
        _ => {}
    }
    events.push(TaskEvent::Dispatched {
        exec: exec.clone(),
        session_id: run.session_id.clone(),
        at: at(3),
    });
    events.extend(
        run.attached
            .iter()
            .map(|session_id| TaskEvent::SessionAttached {
                exec: exec.clone(),
                session_id: session_id.clone(),
                at: at(3),
            }),
    );
    let settle = |outcome| TaskEvent::Settled {
        exec: exec.clone(),
        outcome,
        at: at(4),
    };
    match end {
        SeedEnd::Done => events.push(settle(SettleOutcome::Completed {
            end_reason: SessionEndReason::Stop,
        })),
        SeedEnd::Errored => events.push(settle(SettleOutcome::Errored)),
        SeedEnd::Interrupted => events.push(settle(SettleOutcome::Stopped {
            cause: daruda_store::tasks::machine::StopCause::UserStop,
        })),
        SeedEnd::Cancelled => events.push(TaskEvent::CancelRequested {
            exec: exec.clone(),
            at: at(4),
        }),
        _ => {}
    }
    events
}

/// Commit `events` now, on this thread, and project the result. For
/// fixtures that seed inside an update; panics on a refused event.
pub(crate) fn commit_now(events: Vec<TaskEvent>, cx: &mut App) {
    for event in events {
        RunsHost::commit_blocking(event, cx).expect("a seeded task event commits");
    }
}

/// A record as a pure view function sees it; no log involved. For tests of
/// code that only reads records.
#[cfg(test)]
pub(crate) fn record(
    project: daruda_store::project::ProjectUuid,
    title: &str,
    lifecycle: daruda_store::tasks::Lifecycle,
) -> daruda_store::tasks::TaskRecord {
    let spec = TaskSpec::new(project, title.into(), String::new(), None, Utc::now());
    daruda_store::tasks::TaskRecord {
        spec,
        lifecycle,
        execution: None,
        session_ids: Vec::new(),
        subtasks: Vec::new(),
        updated_at: Utc::now(),
        finished_at: None,
    }
}

/// A lifecycle of `kind` naming a placeholder run in `/tmp`.
#[cfg(test)]
pub(crate) fn lifecycle(
    kind: daruda_store::tasks::LifecycleKind,
) -> daruda_store::tasks::Lifecycle {
    use daruda_store::tasks::machine::{ErrorCause, InterruptCause, UndeliveredReason};
    use daruda_store::tasks::{Lifecycle, LifecycleKind as K};
    let exec = ExecutionRef {
        task_id: "seed".into(),
        execution_id: "seed-run".into(),
    };
    let worktree = PathBuf::from("/tmp");
    match kind {
        K::Backlog => Lifecycle::Backlog,
        K::Preparing => Lifecycle::Preparing {
            exec,
            source: TaskExecutionSource::AgentChat,
        },
        K::Dispatching => Lifecycle::Dispatching { exec, worktree },
        K::Running => Lifecycle::Running { exec, worktree },
        K::Done => Lifecycle::Done {
            exec,
            worktree,
            end_reason: SessionEndReason::Stop,
        },
        K::Error => Lifecycle::Error {
            exec: Some(exec),
            worktree: Some(worktree),
            cause: ErrorCause::SessionFailed,
        },
        K::Cancelled => Lifecycle::Cancelled {
            exec,
            worktree: Some(worktree),
        },
        K::Undelivered => Lifecycle::Undelivered {
            exec,
            worktree: Some(worktree),
            reason: UndeliveredReason::PaneClosed,
        },
        K::Interrupted => Lifecycle::Interrupted {
            exec: Some(exec),
            worktree,
            cause: InterruptCause::UserStop,
        },
    }
}

/// A book holding `records`, as an import would leave it.
#[cfg(test)]
pub(crate) fn book(
    records: Vec<daruda_store::tasks::TaskRecord>,
) -> daruda_store::tasks::TasksBook {
    use daruda_store::tasks::Machine as _;
    let mut book = daruda_store::tasks::TasksBook::default();
    daruda_store::tasks::TaskMachine::apply(&mut book, &TaskEvent::Imported { tasks: records });
    book
}

/// The book `events` leave, each checked as the log would.
#[cfg(test)]
pub(crate) fn replay(events: Vec<TaskEvent>) -> daruda_store::tasks::TasksBook {
    use daruda_store::tasks::Machine as _;
    let mut book = daruda_store::tasks::TasksBook::default();
    for event in events {
        daruda_store::tasks::TaskMachine::check(&book, &event)
            .unwrap_or_else(|e| panic!("{event:?}: {e}"));
        daruda_store::tasks::TaskMachine::apply(&mut book, &event);
    }
    book
}

/// Commit a task from `spec`, taken through `run` to its end when given.
/// Answers the task's id.
#[cfg(test)]
pub(crate) fn task(spec: TaskSpec, run: Option<(&SeedRun, SeedEnd)>, cx: &mut App) -> String {
    let id = spec.id.clone();
    let mut events = created(spec);
    if let Some((seed, end)) = run {
        events.extend(self::run(&id, seed, end));
    }
    commit_now(events, cx);
    id
}

/// A Backlog task's definition in `project`, titled `title`.
#[cfg(test)]
pub(crate) fn spec(project: daruda_store::project::ProjectUuid, title: &str) -> TaskSpec {
    TaskSpec::new(project, title.into(), "P".into(), None, Utc::now())
}

#[cfg(test)]
mod tests {}
