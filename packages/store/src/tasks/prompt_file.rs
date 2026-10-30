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

use super::machine::{LifecycleKind, TaskRecord, TaskSpec};

/// Subdirectory inside the lane where prompt files live.
pub const PROMPT_DIR_NAME: &str = ".daruda";

/// Filename prefix for task prompt files. Final form is `task-<id>.md`.
pub const PROMPT_FILE_PREFIX: &str = "task-";

/// Filename extension used for task prompt files (markdown).
pub const PROMPT_FILE_EXT: &str = "md";

/// Where `task`'s prompt file is written inside `worktree`. Pure — the
/// directory is *not* created here. Named by the task id: a lane may host
/// several tasks, and a task's branch and lane can change while its id cannot.
pub fn prompt_file_path(task: &TaskSpec, worktree: &Path) -> PathBuf {
    prompt_file_named(worktree, &task.id)
}

/// The prompt file to read for `task` — [`prompt_file_path`], unless only a
/// file an older build wrote is on disk, which it named by the branch.
pub fn existing_prompt_file_path(task: &TaskSpec, worktree: &Path) -> PathBuf {
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
/// asking for a final summary. Lane agents have no tool that updates a task;
/// their last answer is what the run records as the task's result. `branch`
/// is what the header names — the caller resolves it, since an existing
/// lane's branch is read at Start.
pub fn render_task_prompt(task: &TaskRecord, branch: &str) -> String {
    let state_label = match task.lifecycle.kind() {
        LifecycleKind::Backlog => "Backlog",
        LifecycleKind::Preparing | LifecycleKind::Dispatching => "Starting",
        LifecycleKind::Running => "Running",
        LifecycleKind::Done => "Done",
        LifecycleKind::Error => "Error",
        LifecycleKind::Cancelled => "Cancelled",
        LifecycleKind::Undelivered => "Undelivered",
        LifecycleKind::Interrupted => "Interrupted",
    };
    let spec = &task.spec;
    let description = task.description.as_ref().filter(|_| {
        matches!(
            task.lifecycle.kind(),
            LifecycleKind::Preparing | LifecycleKind::Dispatching | LifecycleKind::Running
        )
    });
    let (title, prompt) = description.map_or((&spec.title, &spec.prompt), |description| {
        (&description.title, &description.prompt)
    });
    format!(
        "Task: \"{title}\" ({branch})\n\
         Status: {status}\n\
         \n\
         {body}\n\
         \n\
         {CLOSING_INSTRUCTION}\n",
        title = title,
        status = state_label,
        body = prompt,
    )
}

const CLOSING_INSTRUCTION: &str = "Work in the current workspace. Inspect the relevant \
    code, make the needed changes, and verify them when practical. When done, end with \
    a short summary of what you changed and how you verified it; that final answer is \
    recorded as the task's result.";

/// The user's prompt inside text [`render_task_prompt`] produced, so an edit
/// to the prompt file reaches the task without its frame. Each frame part is
/// dropped only where it is still recognizable; text without one comes back
/// whole. Expects LF line endings.
pub fn task_prompt_body(rendered: &str) -> &str {
    let body = after_header(rendered).unwrap_or(rendered);
    before_closing(body).unwrap_or(body)
}

/// Past the `Task:` line, and the `Status:` line and blank line when a hand
/// edit left them, so a trimmed header still never reaches the prompt.
fn after_header(text: &str) -> Option<&str> {
    let (_, rest) = text.strip_prefix("Task: ")?.split_once('\n')?;
    let rest = match rest.strip_prefix("Status: ") {
        Some(status) => status.split_once('\n').map_or("", |(_, rest)| rest),
        None => rest,
    };
    Some(rest.strip_prefix('\n').unwrap_or(rest))
}

fn before_closing(text: &str) -> Option<&str> {
    let rest = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .strip_suffix(CLOSING_INSTRUCTION)?;
    Some(
        rest.strip_suffix("\n\n")
            .or_else(|| rest.strip_suffix('\n'))
            .unwrap_or(rest),
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
