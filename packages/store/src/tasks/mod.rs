//! Right-panel Tasks tab — GPUI-free data model + persistence.
//!
//! Storage layout (mirrors `daruda_panels`):
//! ```text
//! ~/.config/daruda/
//! ├── tasks.json   legacy rows, read once by `TaskStore` and never written
//! └── task-log/    the `TaskMachine` run log
//! ```
//!
//! Each `Task` is a 1:1 mapping with a lane:
//! - Backlog       → no lane yet
//! - Running       → lane spawned, claude session(s) running
//! - Done / Error  → terminal, lane preserved (user deletes via dock)
//! - Cancelled     → user-initiated stop, lane preserved
//!
//! `agent_type` is hardcoded to `Claude`; the field exists for
//! forward-compatible codex / gemini / copilot / cursor-agent support (G5).

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
pub use machine::{Lifecycle, TaskEvent, TaskMachine, TaskRecord, TaskRejected, TasksBook};
pub use migrate::migrate_legacy;
pub use persistence::{load_tasks, load_tasks_in, save_tasks, save_tasks_in, tasks_path_in};
pub use prompt_file::{
    build_claude_command, existing_prompt_file_path, prompt_file_path, render_task_prompt,
    write_prompt_file,
};
pub use store::{TaskStore, TaskStoreError};
pub use task::{
    AgentType, SCHEMA_VERSION, SessionEndReason, SubTask, TASK_TOOL_USE_FAILURE_THRESHOLD, Task,
    TaskAgentSurface, TaskExecution, TaskFilter, TaskId, TaskRunIn, TaskScope, TaskState,
    TasksState,
};
