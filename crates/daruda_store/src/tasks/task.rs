//! Task / TaskState / TasksState — the persistence schema for the
//! right-panel Tasks tab.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Persistence schema version. Bump when the structural contract
/// changes in a way `#[serde(default)]` on new fields cannot transparently
/// absorb (e.g. an enum variant whose disappearance must be migrated).
pub const SCHEMA_VERSION: u32 = 1;

/// Number of `PostToolUseFailure` hook events on the same Claude
/// session before daruda escalates the owning task to `Error`. The
/// threshold is intentionally a small integer — the goal is to catch
/// runaway tool failures (rate-limited Bash, repeated permission
/// denials) without overreacting to a single slip.
pub const TASK_TOOL_USE_FAILURE_THRESHOLD: u32 = 5;

/// ULID encoded as string — sortable by creation time, globally unique
/// without coordination.
pub type TaskId = String;

/// Mirrors Claude hook `SessionEnd.end_reason`. `Stop` is a separate
/// hook event upstream but maps to the same task transition, so it
/// shares this enum for ergonomic matching downstream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    Stop,
    PromptInputExit,
    Logout,
    Other,
    Error,
}

/// Phase 1 hardcodes `Claude`; the variant set is reserved for future
/// codex / gemini / copilot / cursor-agent expansion. Stored on every
/// task so historical rows remain interpretable when the agent fleet
/// grows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    #[default]
    Claude,
}

/// Execution surface a started task's lane runs on.
///
/// - `Terminal` — drive the Claude CLI over the lane's PTY (the
///   historical behaviour): a `claude "$(cat prompt-file)"` command is
///   dispatched into a terminal pane.
/// - `AgentChat` — attach an in-app ACP agent-chat session to the lane
///   and deliver the task prompt as an ACP turn.
///
/// `#[default] Terminal` keeps every pre-existing task (and any task
/// created without an explicit choice) on the CLI path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAgentSurface {
    #[default]
    Terminal,
    AgentChat,
}

/// One row in the per-task subtask checklist (R-21). `source_session_id`
/// distinguishes hook-injected items (R-22 TodoWrite merge) from
/// manually-added ones; the two never coalesce — duplicates with the
/// same title are preserved verbatim (plan I-14 namespace policy).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubTask {
    /// ULID — sortable by creation time, globally unique without
    /// coordination.
    pub id: String,
    pub title: String,
    pub completed: bool,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    /// `Some(session_id)` when injected by the R-22 TodoWrite hook;
    /// `None` for manually-added subtasks. UI displays an "auto" vs.
    /// "manual" label off this flag.
    #[serde(default)]
    pub source_session_id: Option<String>,
}

impl SubTask {
    /// Build a fresh manually-added subtask. ULID + `created_at` are
    /// stamped at construction time; the row starts incomplete and
    /// without a `source_session_id`.
    pub fn new(title: String) -> Self {
        Self {
            id: ulid::Ulid::new().to_string(),
            title,
            completed: false,
            created_at: Some(Utc::now()),
            source_session_id: None,
        }
    }
}

/// Lifecycle state of a single task. The `worktree_path` lives on the
/// terminal variants so callers can locate the (possibly stale)
/// checkout without consulting another store.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TaskState {
    Backlog,
    Running {
        worktree_path: PathBuf,
    },
    /// Determined by Claude hook end_reason, never by exit_code.
    Done {
        worktree_path: PathBuf,
        end_reason: SessionEndReason,
    },
    Error {
        worktree_path: PathBuf,
        message: String,
    },
    /// User-initiated stop. Lane is preserved so the user can
    /// inspect / resume manually.
    Cancelled {
        worktree_path: PathBuf,
    },
}

impl TaskState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done { .. } | Self::Error { .. } | Self::Cancelled { .. }
        )
    }

    pub fn worktree_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Running { worktree_path }
            | Self::Done { worktree_path, .. }
            | Self::Error { worktree_path, .. }
            | Self::Cancelled { worktree_path } => Some(worktree_path),
            Self::Backlog => None,
        }
    }
}

fn default_auto_execute() -> bool {
    true
}

/// One row in the Tasks tab. `branch_name` is derived once at creation
/// time and stays stable across Reopen / Retry so the lane path
/// remains predictable for the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub prompt: String,
    pub state: TaskState,

    /// Claude session IDs currently attached to this task's lane.
    #[serde(default)]
    pub session_ids: Vec<String>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Stop / SessionEnd timestamp. Used to display run duration.
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,

    #[serde(default)]
    pub notes: String,

    /// Lane to branch from. `None` = the active lane at Start
    /// time (resolved by the caller).
    #[serde(default)]
    pub base_worktree_path: Option<PathBuf>,

    /// Sanitized title + ULID 4-char suffix. Stable across Reopen /
    /// Retry — never regenerated.
    pub branch_name: String,

    #[serde(default)]
    pub agent_type: AgentType,

    /// Execution surface for this task's lane — Terminal CLI (default)
    /// or in-app Agent Chat (ACP). `#[serde(default)]` so task JSON
    /// written before this field existed loads as `Terminal`.
    #[serde(default)]
    pub agent_surface: TaskAgentSurface,

    /// `false` skips the trailing newline so the user must press Enter
    /// in the terminal to dispatch the command (UX opt-out).
    #[serde(default = "default_auto_execute")]
    pub auto_execute: bool,

    /// Checklist of finer-grained steps under the task (R-21). Empty
    /// vec is the default for both fresh tasks and pre-R-21 rows
    /// loaded from disk (`#[serde(default)]`).
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
}

impl Task {
    /// Build a fresh task in the `Backlog` state. `branch_name` is
    /// derived from `title` + ULID; callers may overwrite it before
    /// inserting if they need a custom value.
    pub fn new(title: String, prompt: String, base_worktree_path: Option<PathBuf>) -> Self {
        let now = Utc::now();
        let id = ulid::Ulid::new().to_string();
        let branch_name = super::branch::derive_branch_name(&title, &id);
        Self {
            id,
            title,
            prompt,
            state: TaskState::Backlog,
            session_ids: Vec::new(),
            created_at: now,
            updated_at: now,
            finished_at: None,
            notes: String::new(),
            base_worktree_path,
            branch_name,
            agent_type: AgentType::default(),
            agent_surface: TaskAgentSurface::default(),
            auto_execute: true,
            subtasks: Vec::new(),
        }
    }

    /// `(completed, total)` subtask counts — drives the row progress
    /// badge (`☑done/total`) and the TaskEdit pane's section header.
    /// Returned as `usize` so the renderer can format directly.
    pub fn subtask_progress(&self) -> (usize, usize) {
        let done = self.subtasks.iter().filter(|s| s.completed).count();
        (done, self.subtasks.len())
    }
}

/// Top-level state — the entire `tasks.json` file deserializes into this.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksState {
    pub schema_version: u32,
    #[serde(default)]
    pub tasks: Vec<Task>,
}

impl Default for TasksState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tasks: Vec::new(),
        }
    }
}

impl TasksState {
    /// Append `task` and return a reference to the inserted row.
    pub fn add(&mut self, task: Task) -> &Task {
        self.tasks.push(task);
        self.tasks.last().expect("just pushed; vec is non-empty")
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    pub fn remove(&mut self, id: &str) {
        self.tasks.retain(|t| t.id != id);
    }

    pub fn filter_by_state(&self, filter: TaskFilter) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(move |t| filter.matches(&t.state))
    }
}

/// Filter shown in the tab header. Default is `All` (D-11).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaskFilter {
    #[default]
    All,
    Backlog,
    Running,
    /// Aggregates Done + Error + Cancelled — anything `is_terminal()`.
    Done,
}

impl TaskFilter {
    pub fn matches(self, state: &TaskState) -> bool {
        match self {
            Self::All => true,
            Self::Backlog => matches!(state, TaskState::Backlog),
            Self::Running => matches!(state, TaskState::Running { .. }),
            Self::Done => state.is_terminal(),
        }
    }
}
