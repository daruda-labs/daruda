//! File-based persistence for panels state. Read/write plumbing lives
//! in [`crate::persistence`]; this module owns only the path resolver
//! and the schema-version policy.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! └── panels.json     # entire PanelsState — daruda is sole writer
//! ```

use std::path::{Path, PathBuf};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;
use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

use super::{PanelsState, SCHEMA_VERSION};

/// `panels.json` path under `data_dir`. Public so callers (file watcher)
/// can subscribe to the exact path daruda writes.
pub fn panels_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("panels.json")
}

/// Load `panels.json` from `data_dir`. Missing file → `None`. Parse
/// error → logged via the shared loader and treated as `None` so the
/// caller falls back to seeding defaults (the corruption is
/// nevertheless visible in the daruda log).
///
/// A `schema_version` strictly greater than `SCHEMA_VERSION` is rejected
/// rather than silently dropping fields the older daruda cannot preserve.
pub fn load_panels_in(data_dir: &Path) -> Option<PanelsState> {
    let path = panels_path_in(data_dir);
    let state: PanelsState = match load_json_file::<PanelsState>("panels", &path) {
        LoadOutcome::Parsed(s) => s,
        LoadOutcome::Missing | LoadOutcome::Corrupt => return None,
    };
    if state.schema_version > SCHEMA_VERSION {
        LogWriter::log(
            ErrorReport::new("panels.json from a newer daruda — refusing to load")
                .severity(ErrorSeverity::Warning)
                .message(format!(
                    "panels.json schema_version {} > supported {}",
                    state.schema_version, SCHEMA_VERSION,
                ))
                .at(file!(), line!())
                .with_context("path", redact_home(&path))
                .with_context("found", state.schema_version.to_string())
                .with_context("supported", SCHEMA_VERSION.to_string())
                .dedup("panels.schema.too_new")
                .build(),
        );
        return None;
    }
    Some(state)
}

/// Save `panels.json` atomically — same-FS tempfile + rename.
pub fn save_panels_in(data_dir: &Path, state: &PanelsState) -> std::io::Result<()> {
    let path = panels_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

/// Production convenience — load from the default data dir.
pub fn load_panels() -> Option<PanelsState> {
    load_panels_in(&default_data_dir())
}

/// Production convenience — save to the default data dir.
pub fn save_panels(state: &PanelsState) -> std::io::Result<()> {
    save_panels_in(&default_data_dir(), state)
}

/// Back-compat shim — new code should call
/// [`crate::persistence::default_data_dir`] directly.
pub fn default_data_dir() -> PathBuf {
    crate::persistence::default_data_dir()
}
