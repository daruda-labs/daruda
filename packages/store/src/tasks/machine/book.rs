//! The machine's state: every task's definition, lifecycle and run facts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Lifecycle;
use crate::project::ProjectUuid;
use crate::tasks::{
    AgentType, ExecutionRef, SubTask, TaskAgentSurface, TaskExecution, TaskId, TaskRunIn,
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
            auto_execute: true,
            created_at,
        }
    }
}

/// One task as the log knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub spec: TaskSpec,
    pub lifecycle: Lifecycle,
    /// Recorded at `LanePrepared` from the pane the run uses, never edited:
    /// later spec edits never reach a run.
    pub execution: Option<TaskExecution>,
    /// CLI sessions attached to the running lane.
    pub session_ids: Vec<String>,
    pub subtasks: Vec<SubTask>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl TaskRecord {
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
