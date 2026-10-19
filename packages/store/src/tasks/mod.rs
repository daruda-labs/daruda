//! Task specifications, execution facts, and the TaskStore run log.
//!
//! Storage below the profile data directory:
//! ```text
//! <data_dir>/
//! ├── tasks.json   read-only legacy input, imported once by TaskStore
//! └── task-log/    the `TaskMachine` run log
//! ```
//!
//! Task and TasksState are retained to decode legacy migration inputs.

pub mod branch;
pub mod cli;
pub mod machine;
pub mod migrate;
pub mod store;
pub use cli::{AgentChatAccess, CliProcessState, ExecutionRef, TaskExecutionSource};
pub mod persistence;
pub mod prompt_file;
/// Branch-name rules live in `daruda_core::git` — shared with the app so a
/// form's diagnostics and this crate's silent filter cannot drift apart.
pub use daruda_core::git::sanitize_branch_name;
pub mod task;

pub use branch::{branch_name_for, random_branch_name};
pub use daruda_runlog::{AfterCommit, CommitError, Committed, Machine};
pub use machine::{
    Lifecycle, LifecycleKind, StartedBy, TaskEvent, TaskMachine, TaskRecord, TaskRejected,
    TaskSpec, TasksBook,
};
pub use migrate::migrate_legacy;
pub use persistence::{load_tasks, load_tasks_in, tasks_path_in};
pub use prompt_file::{
    build_claude_command, existing_prompt_file_path, prompt_file_path, render_task_prompt,
    write_prompt_file,
};
pub use store::{ReconcileBlocked, Reconciling, TaskStore, TaskStoreError};
pub use task::{
    AgentType, SCHEMA_VERSION, SessionEndReason, SubTask, TASK_TOOL_USE_FAILURE_THRESHOLD, Task,
    TaskAgentSurface, TaskExecution, TaskFilter, TaskId, TaskRunIn, TaskScope, TaskState,
    TasksState, new_task_id,
};
