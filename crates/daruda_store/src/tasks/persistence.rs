//! File-based persistence for the Tasks tab. Read/write plumbing lives
//! in [`crate::persistence`]; this module owns only the path resolver
//! and the schema-version policy.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! └── tasks.json
//! ```

use std::path::{Path, PathBuf};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;
use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

use super::task::{SCHEMA_VERSION, TasksState};

/// `tasks.json` path under `data_dir`. Public so a future file-watcher
/// can subscribe to the exact path daruda writes.
pub fn tasks_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("tasks.json")
}

/// Load `tasks.json` from `data_dir`. Returns `None` on missing file
/// or parse error; the shared loader logs the latter once with a
/// `daruda_tasks:` prefix so corrupt files are observable instead of
/// silently dropped (CLAUDE.md G5 forward-only compatibility note —
/// data loss must not be quiet).
///
/// A `schema_version` strictly greater than `SCHEMA_VERSION` is
/// rejected rather than silently dropping fields the older daruda
/// cannot preserve. Lower versions load through unchanged — every
/// variant of v1 is backward-compatible with `#[serde(default)]`
/// fallbacks, so a v0 file (currently nonexistent) would deserialize
/// as long as the v0 shape is a strict subset of the v1 shape. Bump
/// `SCHEMA_VERSION` + add an explicit migration the moment that stops
/// being true.
pub fn load_tasks_in(data_dir: &Path) -> Option<TasksState> {
    let path = tasks_path_in(data_dir);
    let state: TasksState = match load_json_file::<TasksState>("tasks", &path) {
        LoadOutcome::Parsed(s) => s,
        LoadOutcome::Missing | LoadOutcome::Corrupt => return None,
    };
    if state.schema_version > SCHEMA_VERSION {
        LogWriter::log(
            ErrorReport::new("tasks.json from a newer daruda — refusing to load")
                .severity(ErrorSeverity::Warning)
                .message(format!(
                    "tasks.json schema_version {} > supported {}",
                    state.schema_version, SCHEMA_VERSION,
                ))
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .with_context("found", state.schema_version.to_string())
                .with_context("supported", SCHEMA_VERSION.to_string())
                .dedup("tasks.schema.too_new")
                .build(),
        );
        return None;
    }
    Some(state)
}

/// Save `tasks.json` atomically — same-FS tempfile + rename.
pub fn save_tasks_in(data_dir: &Path, state: &TasksState) -> std::io::Result<()> {
    let path = tasks_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

/// Production convenience — load from `default_data_dir()`.
pub fn load_tasks() -> Option<TasksState> {
    load_tasks_in(&default_data_dir())
}

/// Production convenience — save to `default_data_dir()`.
pub fn save_tasks(state: &TasksState) -> std::io::Result<()> {
    save_tasks_in(&default_data_dir(), state)
}

/// Back-compat shim — new code should call
/// [`crate::persistence::default_data_dir`] directly. Kept here so
/// existing `daruda_store::tasks::persistence::default_data_dir()`
/// callers still resolve.
pub fn default_data_dir() -> PathBuf {
    crate::persistence::default_data_dir()
}
