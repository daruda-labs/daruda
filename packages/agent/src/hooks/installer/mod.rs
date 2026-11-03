//! Install and remove daruda's hook entries in each agent's configuration:
//! Claude Code's `settings.json` and codex's `hooks.json`.
//!
//! Every entry runs [`hook_command`], which does nothing unless the agent
//! runs in a daruda terminal (see [`super::route`]). The command is the same
//! for every build and profile, so a debug and a release install agree on
//! the file's contents instead of overwriting each other.
//!
//! Edits are locked and atomic, rewrite a file only when its contents
//! change, keep every entry that is not daruda's, and refuse a file whose
//! shape cannot be merged safely rather than overwriting it.

use std::path::{Path, PathBuf};

use daruda_core::process_env;
use serde_json::{Value, json};

use super::route::Harness;

mod codex_trust;
mod json_file;

pub use json_file::InstallerError;

/// Claude Code events daruda subscribes to.
pub const CLAUDE_EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
];

/// codex events daruda subscribes to, with each one's matcher. A planning
/// question is the only tool call that waits on the user, and any tool call
/// reasserts that a turn is still working.
pub const CODEX_EVENTS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("SessionEnd", None),
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("^request_user_input$")),
    ("PostToolUse", Some("*")),
    ("Stop", None),
    ("Interrupt", None),
];

/// Claude Code's per-hook timeout, in seconds. The handler exits at once;
/// the bound only caps a wedged filesystem.
const CLAUDE_TIMEOUT_SECS: u64 = 10;

/// The script path every pre-routing daruda install registered. Its entries
/// are daruda's too, so an upgrade replaces them.
const LEGACY_NOTIFY_FRAGMENT: &str = "/.daruda/hooks/notify.sh";

/// One agent configuration home daruda manages hooks in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookTarget {
    pub harness: Harness,
    /// The agent's configuration home: `~/.claude`, `~/.codex`, or a
    /// managed account's directory.
    pub home: PathBuf,
}

impl HookTarget {
    pub fn new(harness: Harness, home: impl Into<PathBuf>) -> Self {
        Self {
            harness,
            home: home.into(),
        }
    }

    /// The ambient home of `harness`, honouring `CLAUDE_CONFIG_DIR` and
    /// `CODEX_HOME`.
    pub fn system(harness: Harness) -> Option<Self> {
        let home = match harness {
            Harness::Claude => crate::accounts::claude::system_claude_home(),
            Harness::Codex => crate::accounts::codex::system_codex_home(),
        }?;
        Some(Self::new(harness, home))
    }

    /// The file that holds the hook entries.
    pub fn hooks_file(&self) -> PathBuf {
        self.home.join(match self.harness {
            Harness::Claude => "settings.json",
            Harness::Codex => "hooks.json",
        })
    }
}

/// Whether an install or uninstall rewrote anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Written,
    Unchanged,
}

/// Merge daruda's entries into `target`, replacing older daruda entries.
pub fn install(target: &HookTarget) -> Result<Change, InstallerError> {
    json_file::update(&target.hooks_file(), |before, config| {
        merge(config, target.harness)?;
        codex_trust::reconcile(target, before, config)
    })
}

/// Remove daruda's entries from `target`, keeping everything else. A
/// missing file stays missing.
pub fn uninstall(target: &HookTarget) -> Result<Change, InstallerError> {
    if !target.hooks_file().exists() {
        return Ok(Change::Unchanged);
    }
    json_file::update(&target.hooks_file(), |before, config| {
        strip(config);
        codex_trust::reconcile(target, before, config)
    })
}

/// Whether `target` carries a daruda entry for every subscribed event.
pub fn is_installed(target: &HookTarget) -> bool {
    let Some(config) = json_file::read(&target.hooks_file()) else {
        return false;
    };
    subscribed(target.harness).all(|(event, _)| {
        config
            .get("hooks")
            .and_then(|hooks| hooks.get(event))
            .and_then(Value::as_array)
            .is_some_and(|groups| groups.iter().any(group_has_daruda_handler))
    })
}

/// The command a hook entry runs. Event names are fixed identifiers, so
/// nothing in it needs quoting beyond the variable reference.
pub fn hook_command(harness: Harness, event: &str) -> String {
    let bin = process_env::HOOK_BIN.name();
    format!(
        "[ -n \"${bin}\" ] && \"${bin}\" --hook {event} --harness {} || true",
        harness.as_str()
    )
}

/// `(event, matcher)` pairs daruda registers for `harness`.
fn subscribed(harness: Harness) -> Box<dyn Iterator<Item = (&'static str, Option<&'static str>)>> {
    match harness {
        Harness::Claude => Box::new(
            CLAUDE_EVENTS
                .iter()
                .map(|event| (*event, Some(claude_matcher(event)))),
        ),
        Harness::Codex => Box::new(CODEX_EVENTS.iter().copied()),
    }
}

/// Notification fires only for the subtypes that ask the user something.
/// `idle_prompt` is left out: Claude Code fires it after nearly every
/// response (anthropics/claude-code#12048, #8320), which would turn the
/// desktop push into a per-turn stream.
fn claude_matcher(event: &str) -> &'static str {
    if event == "Notification" {
        "permission_prompt|elicitation_dialog"
    } else {
        ""
    }
}

fn handler(harness: Harness, event: &str) -> Value {
    let mut handler = json!({
        "type": "command",
        "command": hook_command(harness, event),
    });
    // codex trusts a hook by a hash of its normalized definition; leaving
    // the timeout at its default keeps that definition minimal.
    if harness == Harness::Claude {
        handler["timeout"] = json!(CLAUDE_TIMEOUT_SECS);
    }
    handler
}

fn merge(config: &mut Value, harness: Harness) -> Result<(), InstallerError> {
    strip(config);
    let hooks = json_file::object_entry(config, "hooks")?;
    for (event, matcher) in subscribed(harness) {
        let groups = hooks
            .entry(event.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        let kind = json_file::kind_name(groups);
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| InstallerError::BadSchema {
                event: event.to_owned(),
                actual_kind: kind.to_owned(),
            })?;
        let mut group = json!({ "hooks": [handler(harness, event)] });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        groups.push(group);
    }
    Ok(())
}

/// Drop daruda's handlers, then any group or event left empty by that.
/// A group shared with another tool keeps that tool's handlers.
fn strip(config: &mut Value) {
    let Some(hooks) = config.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    hooks.retain(|_, groups| {
        let Some(array) = groups.as_array_mut() else {
            return true;
        };
        let before = array.len();
        array.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            let count = handlers.len();
            handlers.retain(|handler| !is_daruda_handler(handler));
            count == handlers.len() || !handlers.is_empty()
        });
        before == 0 || !array.is_empty()
    });
}

fn group_has_daruda_handler(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|handlers| handlers.iter().any(is_daruda_handler))
}

fn is_daruda_handler(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(is_daruda_command)
}

/// A current entry names the hook binary variable and `--hook`; an entry
/// from before routing names the shared notify script.
fn is_daruda_command(command: &str) -> bool {
    let current = format!("\"${}\" --hook ", process_env::HOOK_BIN.name());
    command.contains(&current) || command.replace('\\', "/").contains(LEGACY_NOTIFY_FRAGMENT)
}

/// The canonical path of `path`, or `path` itself when it does not exist.
fn canonical(path: &Path) -> PathBuf {
    daruda_core::path::canonicalize(path).unwrap_or_else(|_| {
        match (path.parent(), path.file_name()) {
            (Some(parent), Some(name)) => {
                daruda_core::path::canonicalize_or_self(parent).join(name)
            }
            _ => path.to_path_buf(),
        }
    })
}

#[cfg(test)]
mod tests;
