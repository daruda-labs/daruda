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
    pub fn resolve<'a>(&self, tasks: &'a super::TasksState) -> Option<&'a super::TaskExecution> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_ownership_survives_serialization() {
        let access = AgentChatAccess::CliSnapshot(ExecutionRef {
            task_id: "task".into(),
            execution_id: "run".into(),
        });
        let saved = serde_json::to_string(&access).unwrap();
        // The wire shape restore depends on; a rename here is a format change.
        assert_eq!(
            saved,
            r#"{"cli_snapshot":{"task_id":"task","execution_id":"run"}}"#
        );
        let restored: AgentChatAccess = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored, access);
        assert!(restored.is_read_only());
        assert!(!AgentChatAccess::default().is_read_only());
    }

    #[test]
    fn legacy_execution_is_agent_chat() {
        let execution: super::super::TaskExecution = serde_json::from_value(serde_json::json!({
            "id": "run", "agent_id": "claude", "account_id": null,
            "cwd": "/tmp", "session_id": "session"
        }))
        .unwrap();
        assert_eq!(execution.source, TaskExecutionSource::AgentChat);
        assert!(execution.chat_available());
    }

    #[test]
    fn cli_execution_opens_chat_only_once_its_session_is_known() {
        let mut execution = super::super::TaskExecution {
            source: TaskExecutionSource::ClaudeCli {
                transcript_path: None,
                process: CliProcessState::Discovering,
            },
            id: "run".into(),
            agent_id: "claude".into(),
            account_id: None,
            cwd: "/tmp".into(),
            session_id: None,
        };
        assert!(!execution.chat_available());
        execution.session_id = Some("session".into());
        assert!(execution.chat_available());
    }

    #[test]
    fn cli_process_states_round_trip() {
        let ended_at = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        for process in [
            CliProcessState::Discovering,
            CliProcessState::Running { pid: 42 },
            CliProcessState::ExitConfirmed { ended_at },
            CliProcessState::Unknown,
        ] {
            let source = TaskExecutionSource::ClaudeCli {
                transcript_path: Some("/tmp/t.jsonl".into()),
                process,
            };
            let saved = serde_json::to_string(&source).unwrap();
            assert_eq!(
                serde_json::from_str::<TaskExecutionSource>(&saved).unwrap(),
                source
            );
        }
    }

    #[test]
    fn a_process_record_follows_only_what_its_own_pid_proves() {
        let at = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let mut process = CliProcessState::Discovering;
        assert!(process.observe_running(10));
        assert!(!process.observe_running(10));
        assert!(!process.confirm_exit(99, at));
        assert_eq!(process, CliProcessState::Running { pid: 10 });
        assert!(process.confirm_exit(10, at));
        assert!(!process.confirm_exit(10, at));
        // A resumed session is written again.
        assert!(process.observe_running(11));
        assert_eq!(process, CliProcessState::Running { pid: 11 });

        let mut orphan = CliProcessState::Discovering;
        assert!(orphan.orphan_discovery());
        assert_eq!(orphan, CliProcessState::Unknown);
        assert!(!process.orphan_discovery());
    }

    #[test]
    fn a_ref_resolves_only_the_run_it_names() {
        let mut tasks = super::super::TasksState::default();
        let mut task = super::super::Task::new(
            crate::project::ProjectUuid::new(),
            "t".into(),
            "p".into(),
            None,
        );
        task.execution = Some(super::super::TaskExecution::begin(
            TaskExecutionSource::AgentChat,
            "claude".into(),
            None,
            "/tmp".into(),
        ));
        let run = ExecutionRef {
            task_id: task.id.clone(),
            execution_id: task.execution.as_ref().unwrap().id.clone(),
        };
        let stale = ExecutionRef {
            execution_id: "earlier".into(),
            ..run.clone()
        };
        tasks.add(task);
        assert!(run.resolve(&tasks).is_some());
        assert!(stale.resolve(&tasks).is_none());
        tasks.remove(&run.task_id);
        assert!(run.resolve(&tasks).is_none());
    }
}
