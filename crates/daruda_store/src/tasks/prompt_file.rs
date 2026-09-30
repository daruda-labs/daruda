//! Prompt-file delivery — write the rendered task prompt under the
//! lane's `.daruda/` directory and build the matching shell command
//! that pipes it into `claude` via `cat`. This avoids every shell
//! quoting hazard (newlines, single / double quotes, `$`, backticks,
//! length limits) by handing Claude a file path instead of an inline
//! argv.
//!
//! Mirrors superset-desktop's
//! `terminal-adapter.ts::writeTaskPromptFile` + `agent-command.ts::buildFileCommand`.

use std::fs;
use std::path::{Path, PathBuf};

use super::task::Task;

/// Subdirectory inside the lane where prompt files live.
pub const PROMPT_DIR_NAME: &str = ".daruda";

/// Filename prefix for task prompt files. Final form is `task-<id>.md`.
pub const PROMPT_FILE_PREFIX: &str = "task-";

/// Filename extension used for task prompt files (markdown).
pub const PROMPT_FILE_EXT: &str = "md";

/// Where `task`'s prompt file is written inside `worktree`. Pure — the
/// directory is *not* created here. Named by the task id: a lane may host
/// several tasks, and a task's branch and lane can change while its id cannot.
pub fn prompt_file_path(task: &Task, worktree: &Path) -> PathBuf {
    prompt_file_named(worktree, &task.id)
}

/// The prompt file to read for `task` — [`prompt_file_path`], unless only a
/// file an older build wrote is on disk, which it named by the branch.
pub fn existing_prompt_file_path(task: &Task, worktree: &Path) -> PathBuf {
    let path = prompt_file_path(task, worktree);
    let by_branch = prompt_file_named(worktree, &task.branch_name);
    if !path.exists() && by_branch.exists() {
        by_branch
    } else {
        path
    }
}

fn prompt_file_named(worktree: &Path, stem: &str) -> PathBuf {
    worktree
        .join(PROMPT_DIR_NAME)
        .join(format!("{PROMPT_FILE_PREFIX}{stem}.{PROMPT_FILE_EXT}"))
}

/// Wrap the user's raw prompt with task metadata + a closing instruction
/// telling Claude to update the task on completion. Mirrors superset's
/// `agent-prompt-template.ts`. `branch` is what the header names — the
/// caller resolves it, since an existing lane's branch is read at Start.
pub fn render_task_prompt(task: &Task, branch: &str) -> String {
    let state_label = match &task.state {
        super::task::TaskState::Backlog => "Backlog",
        super::task::TaskState::Running { .. } => "Running",
        super::task::TaskState::Done { .. } => "Done",
        super::task::TaskState::Error { .. } => "Error",
        super::task::TaskState::Cancelled { .. } => "Cancelled",
    };

    format!(
        "Task: \"{title}\" ({branch})\n\
         Status: {status}\n\
         \n\
         {body}\n\
         \n\
         Work in the current workspace. Inspect the relevant code, make the needed \
         changes, verify them when practical, and update task \"{id}\" with a short \
         summary when done.\n",
        title = task.title,
        status = state_label,
        body = task.prompt,
        id = task.id,
    )
}

/// Materialize the prompt file at `path` (from [`prompt_file_path`]),
/// creating its directory. Overwrites any existing file so Reopen / Retry
/// always reflects the latest prompt.
pub fn write_prompt_file(path: &Path, rendered_prompt: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, rendered_prompt)
}

/// Build the shell line daruda sends to the active pane's PTY. The
/// resulting string is ready to write verbatim — including the trailing
/// `\n` when `auto_execute` is true.
///
/// Single-quote escaping uses the standard POSIX `'\\''` trick so paths
/// with literal `'` are safe (`/Users/al'fred/repo` -> `/Users/al'\\''fred/repo`).
pub fn build_claude_command(prompt_file: &Path, auto_execute: bool) -> String {
    let path = prompt_file.to_string_lossy();
    let escaped = path.replace('\'', "'\\''");
    let base = format!(
        "claude --dangerously-skip-permissions \"$(cat '{}')\"",
        escaped
    );
    if auto_execute {
        format!("{base}\n")
    } else {
        base
    }
}
