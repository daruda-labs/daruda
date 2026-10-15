//! Durable CLI execution provenance and fail-closed chat access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What drives a task run. Records written before CLI runs were tracked
/// carry no source and restore as `AgentChat`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutionSource {
    #[default]
    AgentChat,
    ClaudeCli {
        transcript_path: Option<PathBuf>,
        process: CliProcessState,
    },
}

/// What is known about a CLI run's OS process. Only `ExitConfirmed` — the
/// observed PID gone from the process table — lets its chat be continued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliProcessState {
    Discovering,
    Running {
        pid: u32,
    },
    ExitConfirmed {
        ended_at: DateTime<Utc>,
    },
    #[default]
    Unknown,
}

impl TaskExecutionSource {
    pub fn cli_process(&self) -> Option<&CliProcessState> {
        match self {
            Self::AgentChat => None,
            Self::ClaudeCli { process, .. } => Some(process),
        }
    }

    /// The CLI record behind a run, for the observations that change it.
    pub fn cli_mut(&mut self) -> Option<(&mut Option<PathBuf>, &mut CliProcessState)> {
        match self {
            Self::AgentChat => None,
            Self::ClaudeCli {
                transcript_path,
                process,
            } => Some((transcript_path, process)),
        }
    }
}

impl CliProcessState {
    /// A binding for the run's session: a process is writing it. That holds
    /// after a confirmed exit too — `claude --resume` keeps the session id —
    /// so the record follows what is observed. `true` when it changed.
    pub fn observe_running(&mut self, pid: u32) -> bool {
        let next = Self::Running { pid };
        if *self == next {
            return false;
        }
        *self = next;
        true
    }

    /// Only the PID this record names leaving proves exit; another PID's is
    /// a different process. `true` when it changed.
    pub fn confirm_exit(&mut self, pid: u32, ended_at: DateTime<Utc>) -> bool {
        if *self != (Self::Running { pid }) {
            return false;
        }
        *self = Self::ExitConfirmed { ended_at };
        true
    }

    /// A run left `Discovering` across a restart lost the pane that would
    /// have bound it; what it became is unknown. `true` when it changed.
    pub fn orphan_discovery(&mut self) -> bool {
        if *self != Self::Discovering {
            return false;
        }
        *self = Self::Unknown;
        true
    }
}

/// One run of one task — the pair a pane holds to own it, and a snapshot
/// holds to mirror it. A re-run task gets a new execution id, so a stale ref
/// resolves to nothing rather than to the wrong run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRef {
    pub task_id: super::TaskId,
    pub execution_id: String,
}

impl ExecutionRef {
    pub fn resolve<'a>(&self, tasks: &'a super::TasksBook) -> Option<&'a super::TaskExecution> {
        tasks
            .get(&self.task_id)?
            .execution
            .as_ref()
            .filter(|run| run.id == self.execution_id)
    }
}

/// Whether an Agent Chat pane may drive its session. A snapshot names the
/// CLI run it mirrors, so a deleted or re-run task still resolves read-only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChatAccess {
    #[default]
    Interactive,
    CliSnapshot(ExecutionRef),
}

impl AgentChatAccess {
    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::CliSnapshot(_))
    }
}
