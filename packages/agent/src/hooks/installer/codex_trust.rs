//! codex runs a configured hook only once it is trusted: `config.toml`
//! carries `[hooks.state."<hooks.json>:<event>:<group>:<handler>"]` with a
//! `trusted_hash` of the hook's normalized definition. Untrusted hooks are
//! skipped silently, so every daruda entry written to `hooks.json` gets its
//! trust recorded here, at the position it was written to. Only daruda's
//! handlers are trusted; other handlers retain their trust when positions move.
//!
//! The key and hash follow codex's own (undocumented) normalization:
//! the canonical `hooks.json` path, the snake-case event name, and a
//! SHA-256 over key-sorted compact JSON of
//! `{event_name, matcher?, hooks: [{type, command, timeout, async}]}`.
//! Verified against codex-cli 0.166 by running a hook trusted this way.

use std::fs;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table};

use super::{HookTarget, InstallerError, canonical, is_daruda_handler, json_file};
use crate::hooks::route::Harness;

/// codex's default hook timeout, which a definition without one carries.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Where one daruda handler sits in `hooks.json`, with its trust hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Position {
    event: String,
    group: usize,
    handler: usize,
    hash: String,
    definition: Value,
}

/// Move retained handlers' trust with their definitions while the hooks
/// file lock is held, then trust the current daruda handlers.
pub(super) fn reconcile(
    target: &HookTarget,
    before: &Value,
    after: &Value,
) -> Result<(), InstallerError> {
    if target.harness != Harness::Codex {
        return Ok(());
    }
    let previous = positions(before);
    let current = positions(after);
    if current.is_empty() && previous.is_empty() {
        return Ok(());
    }
    let prefix = canonical(&target.hooks_file())
        .to_string_lossy()
        .into_owned();
    let config = json_file::resolved(&target.home.join("config.toml"));
    json_file::with_lock(&config, || {
        let text = match fs::read_to_string(&config) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| InstallerError::Toml(e.to_string()))?;
        let state = state_table(&mut doc)?;
        let mut retained = Vec::new();
        for (position, owned) in &previous {
            let entry = state.remove(&key(&prefix, position));
            if !owned {
                retained.push((position.definition.clone(), entry));
            }
        }
        for (position, owned) in &current {
            let entry = if *owned {
                let mut entry = Table::new();
                entry.insert("trusted_hash", toml_edit::value(position.hash.clone()));
                Some(Item::Table(entry))
            } else {
                retained
                    .iter()
                    .position(|(definition, _)| definition == &position.definition)
                    .and_then(|index| retained.remove(index).1)
            };
            if let Some(entry) = entry {
                state.insert(&key(&prefix, position), entry);
            }
        }
        let updated = doc.to_string();
        if updated != text {
            json_file::write_atomic(&config, updated.as_bytes())?;
        }
        Ok(())
    })
}

/// `hooks.state`, created as implicit tables so only the per-hook headers
/// are written. A non-table value there is the user's and is refused.
fn state_table(doc: &mut DocumentMut) -> Result<&mut Table, InstallerError> {
    let hooks = implicit_table(doc.as_table_mut(), "hooks")?;
    implicit_table(hooks, "state")
}

fn implicit_table<'a>(parent: &'a mut Table, name: &str) -> Result<&'a mut Table, InstallerError> {
    let item = parent.entry(name).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    item.as_table_mut()
        .ok_or_else(|| InstallerError::Toml(format!("`{name}` is not a table")))
}

fn key(prefix: &str, position: &Position) -> String {
    format!(
        "{prefix}:{}:{}:{}",
        snake_case(&position.event),
        position.group,
        position.handler
    )
}

fn positions(config: &Value) -> Vec<(Position, bool)> {
    let Some(hooks) = config.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut positions = Vec::new();
    for (event, groups) in hooks {
        for (group_index, group) in groups.as_array().into_iter().flatten().enumerate() {
            let handlers = group.get("hooks").and_then(Value::as_array);
            for (handler_index, handler) in handlers.into_iter().flatten().enumerate() {
                {
                    positions.push((
                        Position {
                            event: event.clone(),
                            group: group_index,
                            handler: handler_index,
                            hash: trust_hash(event, group.get("matcher"), handler),
                            definition: json!({"event": event, "matcher": group.get("matcher"), "handler": handler}),
                        },
                        is_daruda_handler(handler),
                    ));
                }
            }
        }
    }
    positions
}

/// The hash codex expects for `handler` under `event`, normalized the way
/// codex reads it: defaults filled in, absent optional fields left out.
fn trust_hash(event: &str, matcher: Option<&Value>, handler: &Value) -> String {
    let mut normalized = json!({
        "type": "command",
        "command": handler.get("command").cloned().unwrap_or(Value::Null),
        "timeout": handler.get("timeout").cloned().unwrap_or(json!(DEFAULT_TIMEOUT_SECS)),
        "async": handler.get("async").cloned().unwrap_or(json!(false)),
    });
    for optional in ["commandWindows", "statusMessage"] {
        if let Some(value) = handler.get(optional) {
            normalized[optional] = value.clone();
        }
    }
    let mut identity = json!({
        "event_name": snake_case(event),
        "hooks": [normalized],
    });
    if let Some(matcher) = matcher {
        identity["matcher"] = matcher.clone();
    }
    let bytes = serde_json::to_vec(&sorted(identity)).expect("JSON values serialize");
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// `value` with every object's keys in ascending order.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// `PreToolUse` → `pre_tool_use`.
fn snake_case(event: &str) -> String {
    let mut out = String::with_capacity(event.len() + 4);
    for (i, ch) in event.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition and hash codex-cli 0.166 accepted as trusted in a
    /// live run (a SessionStart hook with no matcher).
    #[test]
    fn the_hash_matches_one_codex_accepted() {
        let handler = json!({
            "type": "command",
            "command": "echo trusted-ok >> /tmp/codex-hook-exp/out-t1.log",
        });
        assert_eq!(
            trust_hash("SessionStart", None, &handler),
            "sha256:564f6cb86f3d2c20be83992123f27ec38ebbfd84110e3a73b33dfbbb6921b483"
        );
    }

    #[test]
    fn event_names_become_snake_case() {
        assert_eq!(snake_case("SessionStart"), "session_start");
        assert_eq!(snake_case("PreToolUse"), "pre_tool_use");
        assert_eq!(snake_case("UserPromptSubmit"), "user_prompt_submit");
        assert_eq!(snake_case("Interrupt"), "interrupt");
    }

    #[test]
    fn the_matcher_is_part_of_the_identity() {
        let handler = json!({ "type": "command", "command": "x" });
        assert_ne!(
            trust_hash("PreToolUse", None, &handler),
            trust_hash("PreToolUse", Some(&json!("a")), &handler)
        );
    }
}
