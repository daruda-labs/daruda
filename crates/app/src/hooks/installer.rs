//! Install / uninstall daruda's entries in `~/.claude/settings.json`.
//!
//! Self-identification: every daruda hook command points at
//! `~/.daruda/hooks/notify.sh`, so we recognise our own entries by
//! the path substring [`MARKER_PATH_FRAGMENT`] in the command string.
//! This lets us cleanly remove just our entries without touching
//! whatever else the user (or another tool) has registered.
//!
//! Atomicity: read-modify-write happens under an exclusive advisory
//! flock on a sibling lock file (`<settings>.daruda.lock`), so
//! concurrent install/uninstall from multiple daruda windows or a
//! settings.json watcher can't lose each other's edits. The actual
//! write itself is also atomic via tempfile + rename.
//!
//! Schema safety: a parse failure or unexpected shape (e.g.
//! `hooks.PreToolUse` written as a string by hand) returns an
//! [`InstallerError`] rather than overwriting the user's data.
//! See the `install_refuses_*` tests for the exact contract.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;
use serde_json::{Value, json};

/// The 9 hook event names daruda subscribes to.
pub const SUBSCRIBED_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
];

/// Substring used to identify daruda's hook entries inside
/// `settings.json`. Every command we register points at
/// `~/.daruda/hooks/notify.sh`, expanded; we match on this stable
/// suffix so dotfile-syncs across machines (with different home
/// paths) still recognise existing entries.
const MARKER_PATH_FRAGMENT: &str = "/.daruda/hooks/notify.sh";

/// Wrapper script source — extracted on first install.
const NOTIFY_SCRIPT: &str = include_str!("notify.sh");

#[derive(Debug)]
pub enum InstallerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NoHome,
    /// `~/.claude/settings.json` parsed as JSON but the structural
    /// shape (e.g. `hooks.<EventName>` not an array) is incompatible
    /// with our merge logic. We refuse to overwrite rather than
    /// silently nuke the user's data.
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
            Self::NoHome => f.write_str("could not resolve user home directory"),
            Self::BadSchema { event, actual_kind } => write!(
                f,
                "settings.json hooks.{event} is {actual_kind}, expected array — \
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
            Self::NoHome | Self::BadSchema { .. } => None,
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

/// Locations resolved from `$HOME`.
pub struct InstallerPaths {
    pub notify_script: PathBuf,
    pub claude_settings: PathBuf,
}

impl InstallerPaths {
    pub fn from_home(home: &Path) -> Self {
        Self {
            notify_script: home.join(".daruda").join("hooks").join("notify.sh"),
            claude_settings: home.join(".claude").join("settings.json"),
        }
    }

    pub fn from_env() -> Result<Self, InstallerError> {
        let home = dirs::home_dir().ok_or(InstallerError::NoHome)?;
        Ok(Self::from_home(&home))
    }
}

/// Install daruda's hook entries. Idempotent — calling repeatedly
/// rewrites the same shape and does not duplicate entries.
pub fn install(paths: &InstallerPaths) -> Result<(), InstallerError> {
    write_notify_script(&paths.notify_script)?;
    update_settings_json(&paths.claude_settings, |settings| {
        merge_daruda_hooks(settings, &paths.notify_script)
    })?;
    Ok(())
}

/// Remove daruda's hook entries. Other tools' entries are preserved.
pub fn uninstall(paths: &InstallerPaths) -> Result<(), InstallerError> {
    update_settings_json(&paths.claude_settings, |settings| {
        strip_daruda_hooks(settings);
        Ok(())
    })?;
    Ok(())
}

/// Quick check — are we registered in `settings.json` for at least
/// one of the subscribed events?
pub fn is_installed(paths: &InstallerPaths) -> bool {
    let Ok(text) = fs::read_to_string(&paths.claude_settings) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let Some(hooks) = value.get("hooks").and_then(|h| h.as_object()) else {
        return false;
    };
    hooks
        .values()
        .filter_map(|v| v.as_array())
        .flat_map(|a| a.iter())
        .any(matcher_contains_daruda_command)
}

// -----------------------------------------------------------------------
// Wrapper script extraction
// -----------------------------------------------------------------------

fn write_notify_script(dst: &Path) -> Result<(), InstallerError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    // Always rewrite so an upgraded daruda picks up wrapper-script
    // changes without the user having to delete the old file.
    fs::write(dst, NOTIFY_SCRIPT)?;
    set_executable(dst)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<(), InstallerError> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<(), InstallerError> {
    Ok(())
}

/// Per-event command string. Claude Code does not forward extra args
/// to the hook command; each event has its own literal `<EventName>`
/// suffix so the wrapper knows what fired.
fn command_for_event(notify_script: &Path, event: &str) -> String {
    format!("\"{}\" {event}", notify_script.display())
}

// -----------------------------------------------------------------------
// settings.json mutation
// -----------------------------------------------------------------------

fn update_settings_json<F>(path: &Path, mutate: F) -> Result<(), InstallerError>
where
    F: FnOnce(&mut Value) -> Result<(), InstallerError>,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Acquire an exclusive advisory lock on a sibling lock file so
    // concurrent installers / uninstallers (and a hypothetical future
    // settings.json self-write watcher) don't lose each other's edits.
    let lock_path = path.with_extension("daruda.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    let result = (|| -> Result<(), InstallerError> {
        let mut value: Value = match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(e) => {
                    // Malformed JSON. We refuse to silently overwrite
                    // — the user's broken file might be salvageable
                    // by hand. Surface the error.
                    return Err(InstallerError::Json(e));
                }
            },
            Ok(_) | Err(_) => Value::Object(Default::default()),
        };
        if !value.is_object() {
            // Top-level isn't an object. settings.json is documented
            // as JSON object — bail rather than nuke a user-authored
            // array/string at the root.
            return Err(InstallerError::BadSchema {
                event: "<root>".to_string(),
                actual_kind: value_kind_name(&value).to_string(),
            });
        }
        mutate(&mut value)?;
        write_atomic_json(path, &value)
    })();
    let _ = FileExt::unlock(&lock);
    result
}

fn value_kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn write_atomic_json(path: &Path, value: &Value) -> Result<(), InstallerError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("settings path has no parent"))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(&bytes)?;
    tmp.flush()?;
    tmp.persist(path).map_err(|e| InstallerError::Io(e.error))?;
    Ok(())
}

/// Add (or replace) daruda's matcher entry for every subscribed
/// event. Other tools' entries inside the same event arrays are
/// preserved. Returns `BadSchema` if any pre-existing
/// `hooks.<EventName>` is structured as something other than an
/// array — refusing to silently overwrite the user's data.
fn merge_daruda_hooks(settings: &mut Value, notify_script: &Path) -> Result<(), InstallerError> {
    let hooks = ensure_object(settings, "hooks");

    for event in SUBSCRIBED_EVENTS {
        let matcher_array = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let array = match matcher_array.as_array_mut() {
            Some(a) => a,
            None => {
                return Err(InstallerError::BadSchema {
                    event: (*event).to_string(),
                    actual_kind: value_kind_name(matcher_array).to_string(),
                });
            }
        };

        // Drop any pre-existing daruda matcher for this event so we
        // overwrite with the current command + matcher shape.
        array.retain(|m| !matcher_contains_daruda_command(m));

        array.push(json!({
            "matcher": notification_matcher_for(event),
            "hooks": [
                {
                    "type": "command",
                    "command": command_for_event(notify_script, event),
                    "timeout": 10,
                }
            ],
        }));
    }
    Ok(())
}

fn strip_daruda_hooks(settings: &mut Value) {
    let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return;
    };
    let mut empty_events: Vec<String> = Vec::new();
    for (event, matchers) in hooks.iter_mut() {
        let Some(array) = matchers.as_array_mut() else {
            continue;
        };
        array.retain(|m| !matcher_contains_daruda_command(m));
        if array.is_empty() {
            empty_events.push(event.clone());
        }
    }
    for event in empty_events {
        hooks.remove(&event);
    }
}

fn matcher_contains_daruda_command(matcher: &Value) -> bool {
    let Some(hooks_array) = matcher.get("hooks").and_then(|h| h.as_array()) else {
        return false;
    };
    hooks_array.iter().any(|h| {
        h.get("command")
            .and_then(|c| c.as_str())
            .is_some_and(|s| s.contains(MARKER_PATH_FRAGMENT))
    })
}

fn ensure_object<'a>(value: &'a mut Value, key: &str) -> &'a mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Default::default());
    }
    let obj = value.as_object_mut().expect("assigned Object above");
    obj.entry(key.to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let entry = obj.get_mut(key).expect("or_insert_with guarantees presence");
    if !entry.is_object() {
        *entry = Value::Object(Default::default());
    }
    entry.as_object_mut().expect("assigned Object to entry above")
}

/// Filter Notification events to only the subtypes that mean the
/// user is actually being asked something. Other events use `""`
/// (match-all).
fn notification_matcher_for(event: &str) -> &'static str {
    if event == "Notification" {
        "permission_prompt|idle_prompt|elicitation_dialog"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> InstallerPaths {
        let home = dir.path();
        InstallerPaths {
            notify_script: home.join(".daruda").join("hooks").join("notify.sh"),
            claude_settings: home.join(".claude").join("settings.json"),
        }
    }

    fn read_settings(p: &Path) -> Value {
        let bytes = fs::read(p).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn install_writes_notify_script_executable() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        install(&paths).unwrap();

        let bytes = fs::read(&paths.notify_script).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("#!/usr/bin/env bash"));
        assert!(text.contains("--hook"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&paths.notify_script)
                .unwrap()
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "notify.sh should be executable");
        }
    }

    #[test]
    fn install_creates_settings_with_all_events() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        install(&paths).unwrap();

        let settings = read_settings(&paths.claude_settings);
        let hooks = settings["hooks"].as_object().unwrap();
        for event in SUBSCRIBED_EVENTS {
            let array = hooks[*event].as_array().unwrap();
            assert_eq!(array.len(), 1, "{event}");
            let matcher = &array[0];
            let cmd = matcher["hooks"][0]["command"].as_str().unwrap();
            assert!(cmd.contains(MARKER_PATH_FRAGMENT), "command={cmd}");
        }
    }

    #[test]
    fn notification_matcher_filters_subtypes() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        install(&paths).unwrap();
        let settings = read_settings(&paths.claude_settings);
        let m = settings["hooks"]["Notification"][0]["matcher"]
            .as_str()
            .unwrap();
        assert_eq!(m, "permission_prompt|idle_prompt|elicitation_dialog");
        // Stop has no matcher (fires always).
        let m = settings["hooks"]["Stop"][0]["matcher"].as_str().unwrap();
        assert_eq!(m, "");
    }

    #[test]
    fn install_preserves_other_tools_entries() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        // Pre-seed settings with a foreign hook + an unrelated key.
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        let pre = json!({
            "model": "sonnet",
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "/usr/local/bin/other-tool.sh" }
                        ]
                    }
                ],
                "PostToolUse": [
                    {
                        "matcher": "",
                        "hooks": [
                            { "type": "command", "command": "/usr/local/bin/other-tool.sh" }
                        ]
                    }
                ]
            }
        });
        fs::write(
            &paths.claude_settings,
            serde_json::to_vec_pretty(&pre).unwrap(),
        )
        .unwrap();

        install(&paths).unwrap();

        let settings = read_settings(&paths.claude_settings);
        // Foreign tool entries still present.
        assert_eq!(settings["model"], "sonnet");
        let pre_array = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_array.len(), 2, "foreign + daruda entry both present");
        assert!(
            pre_array
                .iter()
                .any(|m| !matcher_contains_daruda_command(m))
        );
        assert!(pre_array.iter().any(matcher_contains_daruda_command));
    }

    #[test]
    fn install_is_idempotent() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        install(&paths).unwrap();
        install(&paths).unwrap();
        install(&paths).unwrap();

        let settings = read_settings(&paths.claude_settings);
        for event in SUBSCRIBED_EVENTS {
            let array = settings["hooks"][*event].as_array().unwrap();
            assert_eq!(array.len(), 1, "{event} got duplicated");
        }
    }

    #[test]
    fn uninstall_removes_only_daruda_entries() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);

        // Pre-existing foreign hooks.
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        let pre = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "/usr/local/bin/other.sh" }
                        ]
                    }
                ]
            }
        });
        fs::write(
            &paths.claude_settings,
            serde_json::to_vec_pretty(&pre).unwrap(),
        )
        .unwrap();

        install(&paths).unwrap();
        assert!(is_installed(&paths));
        uninstall(&paths).unwrap();
        assert!(!is_installed(&paths));

        let settings = read_settings(&paths.claude_settings);
        let pre_array = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_array.len(), 1, "foreign entry survives");
        assert!(!matcher_contains_daruda_command(&pre_array[0]));
        // Events that became empty after uninstall are pruned.
        assert!(settings["hooks"].get("PostToolUse").is_none());
    }

    #[test]
    fn is_installed_negative_on_missing_or_malformed_file() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        assert!(!is_installed(&paths));

        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, b"{ malformed").unwrap();
        assert!(!is_installed(&paths));
    }

    #[test]
    fn install_refuses_when_event_value_is_not_array() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        // User has hand-written hooks.PreToolUse as a *string* (wrong
        // schema). We must not silently overwrite the field.
        let pre = json!({
            "hooks": {
                "PreToolUse": "broken-string-value"
            }
        });
        fs::write(
            &paths.claude_settings,
            serde_json::to_vec_pretty(&pre).unwrap(),
        )
        .unwrap();

        let err = install(&paths).expect_err("install should refuse bad schema");
        match err {
            InstallerError::BadSchema {
                event,
                actual_kind: _,
            } => {
                assert_eq!(event, "PreToolUse");
            }
            other => panic!("expected BadSchema, got {other:?}"),
        }
        // Original file untouched.
        let after = read_settings(&paths.claude_settings);
        assert_eq!(after["hooks"]["PreToolUse"], "broken-string-value");
    }

    #[test]
    fn install_refuses_when_top_level_is_not_object() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, b"[1, 2, 3]").unwrap();
        let err = install(&paths).expect_err("install should refuse non-object top-level");
        assert!(matches!(err, InstallerError::BadSchema { .. }));
    }

    #[test]
    fn install_refuses_malformed_json() {
        let home = TempDir::new().unwrap();
        let paths = paths_in(&home);
        fs::create_dir_all(paths.claude_settings.parent().unwrap()).unwrap();
        fs::write(&paths.claude_settings, b"{ malformed json").unwrap();
        let err = install(&paths).expect_err("install should refuse unparseable JSON");
        assert!(matches!(err, InstallerError::Json(_)));
    }
}
