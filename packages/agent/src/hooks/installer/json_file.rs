//! Locked, atomic, change-only edits of an agent's JSON configuration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use super::Change;

#[derive(Debug)]
pub enum InstallerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    /// A TOML configuration that cannot be parsed or merged.
    Toml(String),
    /// Parsed JSON whose `hooks.<EventName>` shape cannot be merged safely.
    BadSchema {
        event: String,
        actual_kind: String,
    },
}

impl std::fmt::Display for InstallerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "installer io: {e}"),
            Self::Json(e) => write!(f, "installer json: {e}"),
            Self::Toml(e) => write!(f, "installer toml: {e}"),
            Self::BadSchema { event, actual_kind } => write!(
                f,
                "hooks.{event} is {actual_kind}, expected array — \
                 refusing to overwrite. Fix the file by hand and try again."
            ),
        }
    }
}

impl std::error::Error for InstallerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Toml(_) | Self::BadSchema { .. } => None,
        }
    }
}

impl From<std::io::Error> for InstallerError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for InstallerError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// The parsed file, or `None` when it is missing or unreadable.
pub(super) fn read(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Apply `mutate` to the JSON object at `path` under a sibling lock and
/// write it back only if it changed. A missing or empty file starts as
/// `{}`; malformed JSON or a non-object root is refused untouched.
pub(super) fn update(
    path: &Path,
    mutate: impl FnOnce(&Value, &mut Value) -> Result<(), InstallerError>,
) -> Result<Change, InstallerError> {
    let path = &resolved(path);
    with_lock(path, || {
        let (mut value, existed) = match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => (serde_json::from_slice::<Value>(&bytes)?, true),
            Ok(_) => (Value::Object(Default::default()), true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Value::Object(Default::default()), false)
            }
            Err(e) => return Err(e.into()),
        };
        if !value.is_object() {
            return Err(InstallerError::BadSchema {
                event: "<root>".to_owned(),
                actual_kind: kind_name(&value).to_owned(),
            });
        }
        let before = value.clone();
        mutate(&before, &mut value)?;
        if existed && value == before {
            return Ok(Change::Unchanged);
        }
        write_atomic(path, &serde_json::to_vec_pretty(&value)?)?;
        Ok(Change::Written)
    })
}

/// Run `edit` while holding an exclusive lock on a sibling of `path`, so
/// concurrent installs (two daruda builds starting at once) cannot lose
/// each other's edits.
pub(super) fn with_lock<T>(
    path: &Path,
    edit: impl FnOnce() -> Result<T, InstallerError>,
) -> Result<T, InstallerError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path.with_extension("daruda.lock"))?;
    lock.lock()?;
    let result = edit();
    // SILENT-OK: the lock is released when the handle drops regardless.
    let _ = lock.unlock();
    result
}

/// The file a write to `path` must replace. A configuration kept as a
/// symlink (into a dotfiles repository, say) is edited at its target; an
/// atomic rename onto the link itself would turn it into a plain file.
pub(super) fn resolved(path: &Path) -> std::path::PathBuf {
    daruda_core::path::canonicalize_or_self(path)
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), InstallerError> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("configuration path has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| InstallerError::Io(e.error))?;
    Ok(())
}

/// The object at `config[key]`, created when absent. A non-object value is
/// the user's and is refused rather than replaced.
pub(super) fn object_entry<'a>(
    config: &'a mut Value,
    key: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, InstallerError> {
    let root = config
        .as_object_mut()
        .expect("update() admits only an object root");
    let entry = root
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Default::default()));
    let kind = kind_name(entry);
    entry.as_object_mut().ok_or(InstallerError::BadSchema {
        event: format!("<{key}>"),
        actual_kind: kind.to_owned(),
    })
}

pub(super) fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
