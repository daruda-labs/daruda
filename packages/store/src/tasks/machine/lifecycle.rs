//! Where one task is in its run, and why a run ended the way it did.
//!
//! Causes are enums, never localized text: the app renders the words.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tasks::{ExecutionRef, SessionEndReason, TaskExecutionSource};

/// A task's lifecycle. Active states always name the execution they belong
/// to, so an event from an earlier execution can be told apart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Lifecycle {
    Backlog,
    /// Start accepted; the lane and pane are being prepared.
    Preparing {
        exec: ExecutionRef,
        /// What will drive the run, until `LanePrepared` records the run.
        #[serde(default)]
        source: TaskExecutionSource,
    },
    /// The prompt is handed to the chat or PTY but not yet on the wire.
    Dispatching {
        exec: ExecutionRef,
        worktree: PathBuf,
    },
    Running {
        exec: ExecutionRef,
        worktree: PathBuf,
    },
    Done {
        exec: ExecutionRef,
        worktree: PathBuf,
        end_reason: SessionEndReason,
    },
    /// `exec` is `None` only for an imported row that never recorded one.
    Error {
        exec: Option<ExecutionRef>,
        worktree: Option<PathBuf>,
        cause: ErrorCause,
    },
    /// `worktree` is `None` when the cancel came before the lane existed.
    Cancelled {
        exec: ExecutionRef,
        worktree: Option<PathBuf>,
    },
    Undelivered {
        exec: ExecutionRef,
        worktree: Option<PathBuf>,
        reason: UndeliveredReason,
    },
    /// `exec` is `None` only for an imported row that never recorded one.
    Interrupted {
        exec: Option<ExecutionRef>,
        worktree: PathBuf,
        cause: InterruptCause,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cause", content = "detail", rename_all = "snake_case")]
pub enum ErrorCause {
    StartFailed(StartFailure),
    SessionFailed,
    ToolFailures,
    /// A legacy row's text, kept verbatim: there is no cause to recover.
    Imported {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartFailure {
    NotFound,
    NotBacklog,
    ProjectNotOpen,
    NoGitRepo,
    RepoBusy,
    LaneMissing,
    PaneUnavailable,
    GitAddFailed,
    FinalizeFailed,
    PromptUndelivered,
    WindowClosed,
    AgentMissing,
    ConnectUnavailable,
    AccountUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndeliveredReason {
    QueueFull,
    ConnectFailed,
    SessionTornDown,
    RemovedByUser,
    PaneClosed,
    StartFailed,
    PtyWriteFailed,
    AppRestart,
    Orphaned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptCause {
    UserStop,
    ExtraUsage,
    LaneGone,
    AppRestart,
}

/// The variant of a [`Lifecycle`] without its data, for rejections and tables.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, enum_iterator::Sequence,
)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleKind {
    Backlog,
    Preparing,
    Dispatching,
    Running,
    Done,
    Error,
    Cancelled,
    Undelivered,
    Interrupted,
}

impl LifecycleKind {
    /// A run has ended; only `Reopened` or `Deleted` leave these.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Error | Self::Cancelled | Self::Undelivered | Self::Interrupted
        )
    }
}

impl Lifecycle {
    pub fn kind(&self) -> LifecycleKind {
        match self {
            Self::Backlog => LifecycleKind::Backlog,
            Self::Preparing { .. } => LifecycleKind::Preparing,
            Self::Dispatching { .. } => LifecycleKind::Dispatching,
            Self::Running { .. } => LifecycleKind::Running,
            Self::Done { .. } => LifecycleKind::Done,
            Self::Error { .. } => LifecycleKind::Error,
            Self::Cancelled { .. } => LifecycleKind::Cancelled,
            Self::Undelivered { .. } => LifecycleKind::Undelivered,
            Self::Interrupted { .. } => LifecycleKind::Interrupted,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.kind().is_terminal()
    }

    /// The execution this state belongs to.
    pub fn exec(&self) -> Option<&ExecutionRef> {
        match self {
            Self::Backlog => None,
            Self::Preparing { exec, .. }
            | Self::Dispatching { exec, .. }
            | Self::Running { exec, .. }
            | Self::Done { exec, .. }
            | Self::Cancelled { exec, .. }
            | Self::Undelivered { exec, .. } => Some(exec),
            Self::Error { exec, .. } | Self::Interrupted { exec, .. } => exec.as_ref(),
        }
    }

    pub fn worktree(&self) -> Option<&Path> {
        match self {
            Self::Backlog | Self::Preparing { .. } => None,
            Self::Dispatching { worktree, .. }
            | Self::Running { worktree, .. }
            | Self::Done { worktree, .. }
            | Self::Interrupted { worktree, .. } => Some(worktree),
            Self::Error { worktree, .. }
            | Self::Cancelled { worktree, .. }
            | Self::Undelivered { worktree, .. } => worktree.as_deref(),
        }
    }
}
