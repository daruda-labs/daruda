//! The machine's state: every task's definition, lifecycle and run facts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Lifecycle, StartedBy};
use crate::project::ProjectUuid;
use crate::tasks::{
    AgentType, ExecutionRef, SubTask, TaskAgentSurface, TaskExecution, TaskFlow, TaskId, TaskRunIn,
};

/// What a task is, independent of any run. Mirrors the legacy `Task` fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: TaskId,
    pub project: ProjectUuid,
    pub title: String,
    pub prompt: String,
    pub notes: String,
    pub base_worktree_path: Option<PathBuf>,
    pub branch_name: String,
    pub run_in: TaskRunIn,
    pub agent_type: AgentType,
    pub agent_surface: TaskAgentSurface,
    /// Runs as this flow instead of on `agent_surface`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<TaskFlow>,
    pub auto_execute: bool,
    pub created_at: DateTime<Utc>,
}

impl TaskSpec {
    /// A new task's definition: a fresh ULID, and a branch named after it.
    pub fn new(
        project: ProjectUuid,
        title: String,
        prompt: String,
        base_worktree_path: Option<PathBuf>,
        created_at: DateTime<Utc>,
    ) -> Self {
        let id = crate::tasks::new_task_id();
        let branch_name = crate::tasks::branch_name_for(&id);
        Self {
            id,
            project,
            title,
            prompt,
            notes: String::new(),
            base_worktree_path,
            branch_name,
            run_in: TaskRunIn::default(),
            agent_type: AgentType::default(),
            agent_surface: TaskAgentSurface::default(),
            flow: None,
            auto_execute: true,
            created_at,
        }
    }
}

/// The title and prompt accepted when an execution was requested.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionDescription {
    pub title: String,
    pub prompt: String,
    /// A person continued an earlier run's conversation: `prompt` is what
    /// they said, not the task's own prompt.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continued: bool,
}

impl EndedExecution {
    /// Whether a person continuing the conversation started this run, rather
    /// than the task being started.
    pub fn is_continuation(&self) -> bool {
        self.description.as_ref().is_some_and(|d| d.continued)
    }
}

/// A finished execution, retained independently of the next attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndedExecution {
    /// Absent when an older snapshot did not retain the original description.
    #[serde(default)]
    pub description: Option<ExecutionDescription>,
    pub exec: ExecutionRef,
    #[serde(default)]
    pub started_by: Option<StartedBy>,
    pub ended_as: Lifecycle,
    pub execution: Option<TaskExecution>,
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: DateTime<Utc>,
    #[serde(default)]
    pub result: Option<String>,
}

/// Recent ended runs kept per task. The latest run the task itself was
/// started for is kept beyond this, so a conversation continued turn after
/// turn cannot push the task's own result out.
const ENDED_EXECUTIONS_LIMIT: usize = 5;

/// One task as the log knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// Frozen at dispatch; older snapshots and imported rows may not know it.
    #[serde(default)]
    pub description: Option<ExecutionDescription>,
    pub spec: TaskSpec,
    pub lifecycle: Lifecycle,
    /// The last requested run's origin; reopening keeps it until a new start.
    #[serde(default)]
    pub started_by: Option<StartedBy>,
    /// Recorded at `LanePrepared` from the pane the run uses, never edited:
    /// later spec edits never reach a run.
    pub execution: Option<TaskExecution>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub ended_executions: Vec<EndedExecution>,
    /// CLI sessions attached to the running lane.
    pub session_ids: Vec<String>,
    pub subtasks: Vec<SubTask>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl TaskRecord {
    pub(super) fn retain_ended(&mut self, ended: EndedExecution) {
        self.ended_executions.push(ended);
        let len = self.ended_executions.len();
        let recent = len.saturating_sub(ENDED_EXECUTIONS_LIMIT);
        let anchor = self
            .ended_executions
            .iter()
            .rposition(|run| !run.is_continuation());
        let mut index = 0;
        self.ended_executions.retain(|_| {
            let keep = index >= recent || Some(index) == anchor;
            index += 1;
            keep
        });
    }

    pub fn id(&self) -> &str {
        &self.spec.id
    }

    /// `(completed, total)` subtask counts, for the row's progress badge.
    pub fn subtask_progress(&self) -> (usize, usize) {
        let done = self.subtasks.iter().filter(|s| s.completed).count();
        (done, self.subtasks.len())
    }

    /// The execution events must name to act on this record.
    pub fn current_exec(&self) -> Option<&ExecutionRef> {
        self.lifecycle.exec()
    }

    /// Whether a person can continue the last run's conversation as a new
    /// run: it finished, or a person or the usage limit stopped it, in an
    /// Agent Chat whose session is known. A failed run is retried instead.
    pub fn is_continuable(&self) -> bool {
        let stopped_by_choice = match &self.lifecycle {
            super::Lifecycle::Done { .. } => true,
            super::Lifecycle::Interrupted { cause, .. } => matches!(
                cause,
                super::InterruptCause::UserStop | super::InterruptCause::ExtraUsage
            ),
            _ => false,
        };
        stopped_by_choice
            && self.execution.as_ref().is_some_and(|run| {
                matches!(run.source, crate::tasks::TaskExecutionSource::AgentChat)
                    && run.session_id.is_some()
            })
    }
}

/// Every task, keyed by ULID so iteration is creation order. Sealed: only
/// [`TaskMachine`](super::TaskMachine) changes it, through committed events.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksBook {
    records: BTreeMap<TaskId, TaskRecord>,
}

impl TasksBook {
    pub fn get(&self, id: &str) -> Option<&TaskRecord> {
        self.records.get(id)
    }

    /// Records in creation order.
    pub fn records(&self) -> impl Iterator<Item = &TaskRecord> {
        self.records.values()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub(super) fn get_mut(&mut self, id: &str) -> Option<&mut TaskRecord> {
        self.records.get_mut(id)
    }

    pub(super) fn insert(&mut self, record: TaskRecord) {
        self.records.insert(record.spec.id.clone(), record);
    }

    pub(super) fn remove(&mut self, id: &str) {
        self.records.remove(id);
    }
}
