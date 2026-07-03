//! Right-panel Tasks tab — GPUI-free data model + persistence.
//!
//! Storage layout (mirrors `daruda_panels`):
//! ```text
//! ~/.config/daruda/
//! └── tasks.json
//! ```
//!
//! Each `Task` is a 1:1 mapping with a lane:
//! - Backlog       → no lane yet
//! - Running       → lane spawned, claude session(s) running
//! - Done / Error  → terminal, lane preserved (user deletes via dock)
//! - Cancelled     → user-initiated stop, lane preserved
//!
//! Phase 1 hardcodes `agent_type = Claude`; the field is reserved for
//! future codex / gemini / copilot / cursor-agent expansion (G5
//! forward-only compatibility).
//!
//! See `daruda/docs/Right-Panel-Tasks-Plan.md` for the full spec.

pub mod branch;
pub mod persistence;
pub mod prompt_file;
pub mod sanitize;
pub mod task;

#[cfg(test)]
mod tests;

pub use branch::derive_branch_name;
pub use persistence::{load_tasks, load_tasks_in, save_tasks, save_tasks_in, tasks_path_in};
pub use prompt_file::{
    build_claude_command, render_task_prompt, task_prompt_file_path, write_prompt_file,
};
pub use task::{
    AgentType, SCHEMA_VERSION, SessionEndReason, SubTask, TASK_TOOL_USE_FAILURE_THRESHOLD, Task,
    TaskAgentSurface, TaskFilter, TaskId, TaskState, TasksState,
};
