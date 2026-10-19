//! Everything that can happen to a task. The proposer picks every timestamp
//! and id, so replay never reads the clock or generates one.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{InterruptCause, StartFailure, TaskRecord, TaskSpec, UndeliveredReason};
use crate::accounts::AccountId;
use crate::project::ProjectUuid;
use crate::tasks::{
    CliProcessState, ExecutionRef, SessionEndReason, SubTask, TaskAgentSurface,
    TaskExecutionSource, TaskId, TaskRunIn,
};

/// The surface that requested an execution, independent of its agent transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartedBy {
    Desktop,
    Phone,
    Orchestrator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// The legacy `tasks.json` rows; only into an empty book.
    Imported {
        tasks: Vec<TaskRecord>,
    },
    Created {
        spec: TaskSpec,
    },
    Edited {
        id: TaskId,
        edit: TaskEdit,
        at: DateTime<Utc>,
    },
    Deleted {
        id: TaskId,
        at: DateTime<Utc>,
    },
    SubtaskAdded {
        id: TaskId,
        subtask: SubTask,
        at: DateTime<Utc>,
    },
    SubtaskToggled {
        id: TaskId,
        subtask_id: String,
        at: DateTime<Utc>,
    },
    SubtaskRenamed {
        id: TaskId,
        subtask_id: String,
        title: String,
        at: DateTime<Utc>,
    },
    SubtaskDeleted {
        id: TaskId,
        subtask_id: String,
        at: DateTime<Utc>,
    },
    /// A `TodoWrite` payload from one of the task's sessions.
    TodoMerged {
        id: TaskId,
        session_id: String,
        items: Vec<TodoItem>,
        at: DateTime<Utc>,
    },
    /// Freezes what is known before any lane or pane exists: the run's id
    /// and what drives it. The rest of the run is fixed by `LanePrepared`.
    DispatchRequested {
        id: TaskId,
        execution_id: String,
        source: TaskExecutionSource,
        #[serde(default)]
        started_by: Option<StartedBy>,
        at: DateTime<Utc>,
    },
    /// The pane the run uses exists; its identity completes the run.
    LanePrepared {
        exec: ExecutionRef,
        worktree: PathBuf,
        agent_id: String,
        account_id: Option<AccountId>,
        cwd: PathBuf,
        at: DateTime<Utc>,
    },
    DispatchFailed {
        exec: ExecutionRef,
        cause: StartFailure,
        at: DateTime<Utc>,
    },
    Dispatched {
        exec: ExecutionRef,
        session_id: Option<String>,
        at: DateTime<Utc>,
    },
    Undelivered {
        exec: ExecutionRef,
        reason: UndeliveredReason,
        at: DateTime<Utc>,
    },
    /// Names the run's own session once it is known.
    ExecutionSessionBound {
        exec: ExecutionRef,
        session_id: String,
        at: DateTime<Utc>,
    },
    SessionAttached {
        exec: ExecutionRef,
        session_id: String,
        at: DateTime<Utc>,
    },
    SessionDetached {
        exec: ExecutionRef,
        session_id: String,
        at: DateTime<Utc>,
    },
    /// What is known of a CLI run's process; arrives after the run ended too.
    CliProcessObserved {
        exec: ExecutionRef,
        process: CliProcessState,
        at: DateTime<Utc>,
    },
    CliTranscriptFound {
        exec: ExecutionRef,
        transcript_path: PathBuf,
        at: DateTime<Utc>,
    },
    Settled {
        #[serde(default)]
        result: Option<String>,
        exec: ExecutionRef,
        outcome: SettleOutcome,
        at: DateTime<Utc>,
    },
    ToolFailuresEscalated {
        exec: ExecutionRef,
        at: DateTime<Utc>,
    },
    CancelRequested {
        exec: ExecutionRef,
        at: DateTime<Utc>,
    },
    LaneLost {
        exec: ExecutionRef,
        at: DateTime<Utc>,
    },
    /// App-start cleanup of a run the previous process left active.
    Reconciled {
        exec: ExecutionRef,
        at: DateTime<Utc>,
    },
    /// `run_in` is computed by the host, e.g. the lane the last run created.
    Reopened {
        id: TaskId,
        run_in: TaskRunIn,
        at: DateTime<Utc>,
    },
}

/// The editable fields. Launch fields (`branch`, `run_in`, `project`) apply
/// only in Backlog and are ignored otherwise, as the editor always sends them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEdit {
    pub title: String,
    pub prompt: String,
    pub notes: String,
    pub auto_execute: bool,
    pub agent_surface: TaskAgentSurface,
    pub base_worktree_path: Option<PathBuf>,
    pub branch: Option<String>,
    pub run_in: Option<TaskRunIn>,
    pub project: Option<ProjectUuid>,
}

/// One `TodoWrite` entry. `id` is used only if the entry becomes a new row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub completed: bool,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SettleOutcome {
    /// `SessionEndReason::Error` settles as a failed session, as the legacy
    /// hook mapping did.
    Completed {
        end_reason: SessionEndReason,
    },
    Errored,
    /// A stop interrupts the run; it is never `Done`.
    Stopped {
        cause: StopCause,
    },
}

/// The interrupt causes a running session can settle with; lane loss and
/// restarts have their own events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopCause {
    UserStop,
    ExtraUsage,
}

impl From<StopCause> for InterruptCause {
    fn from(cause: StopCause) -> Self {
        match cause {
            StopCause::UserStop => Self::UserStop,
            StopCause::ExtraUsage => Self::ExtraUsage,
        }
    }
}

/// The variant of a [`TaskEvent`] without its data.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, enum_iterator::Sequence,
)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Imported,
    Created,
    Edited,
    Deleted,
    SubtaskAdded,
    SubtaskToggled,
    SubtaskRenamed,
    SubtaskDeleted,
    TodoMerged,
    DispatchRequested,
    LanePrepared,
    DispatchFailed,
    Dispatched,
    Undelivered,
    ExecutionSessionBound,
    SessionAttached,
    SessionDetached,
    CliProcessObserved,
    CliTranscriptFound,
    Settled,
    ToolFailuresEscalated,
    CancelRequested,
    LaneLost,
    Reconciled,
    Reopened,
}

impl TaskEvent {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::Imported { .. } => EventKind::Imported,
            Self::Created { .. } => EventKind::Created,
            Self::Edited { .. } => EventKind::Edited,
            Self::Deleted { .. } => EventKind::Deleted,
            Self::SubtaskAdded { .. } => EventKind::SubtaskAdded,
            Self::SubtaskToggled { .. } => EventKind::SubtaskToggled,
            Self::SubtaskRenamed { .. } => EventKind::SubtaskRenamed,
            Self::SubtaskDeleted { .. } => EventKind::SubtaskDeleted,
            Self::TodoMerged { .. } => EventKind::TodoMerged,
            Self::DispatchRequested { .. } => EventKind::DispatchRequested,
            Self::LanePrepared { .. } => EventKind::LanePrepared,
            Self::DispatchFailed { .. } => EventKind::DispatchFailed,
            Self::Dispatched { .. } => EventKind::Dispatched,
            Self::Undelivered { .. } => EventKind::Undelivered,
            Self::ExecutionSessionBound { .. } => EventKind::ExecutionSessionBound,
            Self::SessionAttached { .. } => EventKind::SessionAttached,
            Self::SessionDetached { .. } => EventKind::SessionDetached,
            Self::CliProcessObserved { .. } => EventKind::CliProcessObserved,
            Self::CliTranscriptFound { .. } => EventKind::CliTranscriptFound,
            Self::Settled { .. } => EventKind::Settled,
            Self::ToolFailuresEscalated { .. } => EventKind::ToolFailuresEscalated,
            Self::CancelRequested { .. } => EventKind::CancelRequested,
            Self::LaneLost { .. } => EventKind::LaneLost,
            Self::Reconciled { .. } => EventKind::Reconciled,
            Self::Reopened { .. } => EventKind::Reopened,
        }
    }

    /// The execution an event acts on, for those bound to one run.
    pub(super) fn exec(&self) -> Option<&ExecutionRef> {
        match self {
            Self::LanePrepared { exec, .. }
            | Self::DispatchFailed { exec, .. }
            | Self::Dispatched { exec, .. }
            | Self::Undelivered { exec, .. }
            | Self::ExecutionSessionBound { exec, .. }
            | Self::SessionAttached { exec, .. }
            | Self::SessionDetached { exec, .. }
            | Self::CliProcessObserved { exec, .. }
            | Self::CliTranscriptFound { exec, .. }
            | Self::Settled { exec, .. }
            | Self::ToolFailuresEscalated { exec, .. }
            | Self::CancelRequested { exec, .. }
            | Self::LaneLost { exec, .. }
            | Self::Reconciled { exec, .. } => Some(exec),
            Self::Imported { .. }
            | Self::Created { .. }
            | Self::Edited { .. }
            | Self::Deleted { .. }
            | Self::SubtaskAdded { .. }
            | Self::SubtaskToggled { .. }
            | Self::SubtaskRenamed { .. }
            | Self::SubtaskDeleted { .. }
            | Self::TodoMerged { .. }
            | Self::DispatchRequested { .. }
            | Self::Reopened { .. } => None,
        }
    }

    /// The task an event targets; `None` for `Imported`.
    pub(super) fn task_id(&self) -> Option<&str> {
        match self {
            Self::Imported { .. } => None,
            Self::Created { spec } => Some(&spec.id),
            Self::Edited { id, .. }
            | Self::Deleted { id, .. }
            | Self::SubtaskAdded { id, .. }
            | Self::SubtaskToggled { id, .. }
            | Self::SubtaskRenamed { id, .. }
            | Self::SubtaskDeleted { id, .. }
            | Self::TodoMerged { id, .. }
            | Self::DispatchRequested { id, .. }
            | Self::Reopened { id, .. } => Some(id),
            Self::LanePrepared { exec, .. }
            | Self::DispatchFailed { exec, .. }
            | Self::Dispatched { exec, .. }
            | Self::Undelivered { exec, .. }
            | Self::ExecutionSessionBound { exec, .. }
            | Self::SessionAttached { exec, .. }
            | Self::SessionDetached { exec, .. }
            | Self::CliProcessObserved { exec, .. }
            | Self::CliTranscriptFound { exec, .. }
            | Self::Settled { exec, .. }
            | Self::ToolFailuresEscalated { exec, .. }
            | Self::CancelRequested { exec, .. }
            | Self::LaneLost { exec, .. }
            | Self::Reconciled { exec, .. } => Some(&exec.task_id),
        }
    }
}
