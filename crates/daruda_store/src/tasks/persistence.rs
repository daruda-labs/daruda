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
use serde::Deserialize;

use super::task::{SCHEMA_VERSION, TasksState};

/// `tasks.json` path under `data_dir`. Public so a future file-watcher
/// can subscribe to the exact path daruda writes.
pub fn tasks_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("tasks.json")
}

/// Load `tasks.json` from `data_dir`. Returns `None` on missing file
/// or parse error; the shared loader logs the latter once with a
/// `daruda_tasks:` prefix so corrupt files are observable instead of
/// silently dropped (data loss must not be quiet).
///
/// A `schema_version` strictly greater than `SCHEMA_VERSION` is
/// rejected rather than silently dropping fields the older daruda
/// cannot preserve. A lower one loads as an empty list: v1 tasks name no
/// project, and there is nothing to recover one from. The version is read
/// first because a v1 row would not parse as a v2 `Task` at all.
pub fn load_tasks_in(data_dir: &Path) -> Option<TasksState> {
    let path = tasks_path_in(data_dir);
    let version = match load_json_file::<VersionProbe>("tasks", &path) {
        LoadOutcome::Parsed(probe) => probe.schema_version,
        LoadOutcome::Missing | LoadOutcome::Corrupt => return None,
    };
    if version > SCHEMA_VERSION {
        LogWriter::log(
            ErrorReport::new("tasks.json from a newer daruda — refusing to load")
                .severity(ErrorSeverity::Warning)
                .message(format!(
                    "tasks.json schema_version {version} > supported {SCHEMA_VERSION}",
                ))
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .with_context("found", version.to_string())
                .with_context("supported", SCHEMA_VERSION.to_string())
                .dedup("tasks.schema.too_new")
                .build(),
        );
        return None;
    }
    if version < SCHEMA_VERSION {
        let dropped = match load_json_file::<RowCount>("tasks", &path) {
            LoadOutcome::Parsed(rows) => rows.tasks.len(),
            LoadOutcome::Missing | LoadOutcome::Corrupt => 0,
        };
        LogWriter::log(
            ErrorReport::new("tasks.json predates project-scoped tasks — starting empty")
                .severity(ErrorSeverity::Info)
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .with_context("found", version.to_string())
                .with_context("dropped", dropped.to_string())
                .dedup("tasks.schema.dropped_unscoped")
                .build(),
        );
        return Some(TasksState::default());
    }
    load_json_file::<TasksState>("tasks", &path).into_option()
}

/// Just the version, so it can be read from a file whose rows no longer
/// parse.
#[derive(Deserialize)]
struct VersionProbe {
    schema_version: u32,
}

/// Just the row count of a file being discarded, for the log line.
#[derive(Deserialize)]
struct RowCount {
    #[serde(default)]
    tasks: Vec<serde::de::IgnoredAny>,
}

/// Save `tasks.json` atomically — same-FS tempfile + rename.
pub fn save_tasks_in(data_dir: &Path, state: &TasksState) -> std::io::Result<()> {
    let path = tasks_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

/// Production convenience — load from `crate::persistence::default_data_dir()`.
pub fn load_tasks() -> Option<TasksState> {
    load_tasks_in(&crate::persistence::default_data_dir())
}

/// Production convenience — save to `crate::persistence::default_data_dir()`.
pub fn save_tasks(state: &TasksState) -> std::io::Result<()> {
    save_tasks_in(&crate::persistence::default_data_dir(), state)
}
