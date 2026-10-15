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

/// One task as the log knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub spec: TaskSpec,
    pub lifecycle: Lifecycle,
    /// Frozen at `DispatchRequested`: later spec edits never reach a run.
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
