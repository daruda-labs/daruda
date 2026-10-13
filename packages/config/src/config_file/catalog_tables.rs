//! Hand-built `toml_edit` tables for the agent and session-host catalogs, and
//! the legacy `[agent]` keys both save paths clear.

use crate::{
    AgentEntry, AgentLaunch, AgentSource, Config, SessionHostEntry, SessionHostKind,
    SessionHostTombstone, StatusBarItem,
};

pub(super) fn replace_agents(doc: &mut toml_edit::DocumentMut, entries: &[AgentEntry]) {
    let mut tables = toml_edit::ArrayOfTables::new();
    for entry in entries {
        tables.push(agent_entry_table(entry));
    }
    doc.insert("agents", toml_edit::Item::ArrayOfTables(tables));
}

pub(super) fn replace_session_hosts(
    doc: &mut toml_edit::DocumentMut,
    entries: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) {
    replace_array_of_tables(doc, "session_hosts", entries, session_host_entry_table);
    replace_array_of_tables(
        doc,
        "session_host_tombstones",
        tombstones,
        session_host_tombstone_table,
    );
}

fn replace_array_of_tables<T>(
    doc: &mut toml_edit::DocumentMut,
    key: &str,
    values: &[T],
    table: impl Fn(&T) -> toml_edit::Table,
) {
    if values.is_empty() {
        doc.remove(key);
        return;
    }
    let mut tables = toml_edit::ArrayOfTables::new();
    for value in values {
        tables.push(table(value));
    }
    doc.insert(key, toml_edit::Item::ArrayOfTables(tables));
}

fn session_host_entry_table(entry: &SessionHostEntry) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["id"] = toml_edit::value(entry.id.as_inner().to_string());
    table["label"] = toml_edit::value(entry.label.clone());
    table["kind"] = toml_edit::Item::Table(session_host_kind_table(&entry.kind));
    table
}

fn session_host_tombstone_table(entry: &SessionHostTombstone) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    table["old_id"] = toml_edit::value(entry.old_id.as_inner().to_string());
    table["value"] = toml_edit::value(entry.value.clone());
    table["removed_at"] = toml_edit::value(entry.removed_at as i64);
    if let Some(id) = entry.redirected_to {
        table["redirected_to"] = toml_edit::value(id.as_inner().to_string());
    }
    table["kind"] = toml_edit::Item::Table(session_host_kind_table(&entry.kind));
    table
}

fn session_host_kind_table(kind: &SessionHostKind) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    match kind {
        SessionHostKind::Ssh { target } => {
            table["type"] = toml_edit::value("ssh");
            table["target"] = toml_edit::value(target.clone());
        }
        SessionHostKind::Docker { container } => {
            table["type"] = toml_edit::value("docker");
            table["container"] = toml_edit::value(container.clone());
        }
    }
    table
}

pub(super) fn status_bar_item_slug(item: StatusBarItem) -> &'static str {
    match item {
        StatusBarItem::ProjectBranch => "project_branch",
        StatusBarItem::AccountSlot => "account_slot",
        StatusBarItem::Ports => "ports",
        StatusBarItem::ClaudeUsage => "claude_usage",
        StatusBarItem::Flow => "flow",
    }
}

/// The per-agent transcript-presentation keys, written only where set so an
/// agent that follows the built-in keeps no key of its own. An empty
/// `display_filter` list is a set of its own (nothing visible), so it is
/// written like any other value rather than treated as absent.
fn write_transcript_defaults(
    table: &mut toml_edit::Table,
    fold_mode: Option<&[String]>,
    tail_window: Option<u8>,
    tail_window_calls: Option<u8>,
    display_filter: Option<&[String]>,
) {
    if let Some(fold_mode) = fold_mode {
        table["fold_mode"] = string_array(fold_mode);
    }
    if let Some(tail_window) = tail_window {
        table["tail_window"] = toml_edit::value(i64::from(tail_window));
    }
    if let Some(tail_window_calls) = tail_window_calls {
        table["tail_window_calls"] = toml_edit::value(i64::from(tail_window_calls));
    }
    if let Some(display_filter) = display_filter {
        table["display_filter"] = string_array(display_filter);
    }
}

/// An `env` sub-table for an `[[agents]]` row. Emitted after every scalar key
/// and beside the `ssh` / `docker` sub-table: TOML forbids a value after a
/// table within the same entry.
fn env_table(env: &[(String, String)]) -> toml_edit::Item {
    let mut table = toml_edit::Table::new();
    for (key, value) in env {
        table[key.as_str()] = toml_edit::value(value.clone());
    }
    toml_edit::Item::Table(table)
}

fn string_array(values: &[String]) -> toml_edit::Item {
    toml_edit::value(
        values
            .iter()
            .map(String::as_str)
            .collect::<toml_edit::Array>(),
    )
}

/// One `[[agents]]` table as [`patch_config_file_to`] writes it. Hand-built
/// rather than serialized because the surrounding document is a `toml_edit`
/// tree that preserves the user's comments and formatting — which means this
/// function is the real persistence boundary for the catalog and has to mirror
/// [`AgentEntry`]'s shape exactly: a reference writes `preset` plus only the
/// overrides that are set, so an untouched field keeps tracking the preset, and
/// an entry daruda cannot resolve is written back unchanged instead of pruned.
///
/// Scalars are written before the `ssh` / `docker` sub-tables — TOML gives a
/// value after a table to that table.
fn agent_entry_table(entry: &AgentEntry) -> toml_edit::Table {
    let mut table = toml_edit::Table::new();
    // Same rule as the serde path: only a switched-off entry says so.
    if !entry.enabled {
        table["enabled"] = toml_edit::value(false);
    }
    match &entry.source {
        AgentSource::Preset { preset, overrides } => {
            table["preset"] = toml_edit::value(preset.clone());
            if let Some(id) = &overrides.id {
                table["id"] = toml_edit::value(id.clone());
            }
            if let Some(name) = &overrides.name {
                table["name"] = toml_edit::value(name.clone());
            }
            if let Some(command) = &overrides.command {
                table["command"] = toml_edit::value(command.clone());
            }
            if let Some(default_mode) = &overrides.default_mode {
                table["default_mode"] = toml_edit::value(default_mode.clone());
            }
            if let Some(default_model) = &overrides.default_model {
                table["default_model"] = toml_edit::value(default_model.clone());
            }
            write_transcript_defaults(
                &mut table,
                overrides.fold_mode.as_deref(),
                overrides.tail_window,
                overrides.tail_window_calls,
                overrides.display_filter.as_deref(),
            );
            // Written whenever the row states one at all: an empty table is
            // the preset's environment cleared, not the absence of an
            // override (see `PresetOverrides::env`).
            if let Some(env) = &overrides.env {
                table["env"] = env_table(env);
            }
        }
        AgentSource::Custom(agent) => {
            table["id"] = toml_edit::value(agent.id.clone());
            table["name"] = toml_edit::value(agent.name.clone());
            // A remote launch has no flat `command` key; it yields the sub-table
            // held back until every scalar key is in place.
            let remote = match &agent.launch {
                AgentLaunch::Raw(command) => {
                    table["command"] = toml_edit::value(command.clone());
                    None
                }
                AgentLaunch::Ssh {
                    adapter_command,
                    host,
                } => {
                    let mut ssh = toml_edit::Table::new();
                    ssh["adapter_command"] = toml_edit::value(adapter_command.clone());
                    ssh["host"] = toml_edit::value(host.clone());
                    Some(("ssh", ssh))
                }
                AgentLaunch::Docker {
                    adapter_command,
                    container,
                } => {
                    let mut docker = toml_edit::Table::new();
                    docker["adapter_command"] = toml_edit::value(adapter_command.clone());
                    docker["container"] = toml_edit::value(container.clone());
                    Some(("docker", docker))
                }
            };
            if let Some(default_mode) = &agent.default_mode {
                table["default_mode"] = toml_edit::value(default_mode.clone());
            }
            if let Some(default_model) = &agent.default_model {
                table["default_model"] = toml_edit::value(default_model.clone());
            }
            write_transcript_defaults(
                &mut table,
                agent.fold_mode.as_deref(),
                agent.tail_window,
                agent.tail_window_calls,
                agent.display_filter.as_deref(),
            );
            // Same rule as a preset override above: written whenever the
            // definition states one at all, so an empty table survives as
            // "stated, and empty".
            if let Some(env) = &agent.env {
                table["env"] = env_table(env);
            }
            if let Some((key, sub_table)) = remote {
                table[key] = toml_edit::Item::Table(sub_table);
            }
        }
    }

    table
}

/// The `[agent]` keys [`Config::clamp`] migrates into the agent catalog. Listed
/// once because both save paths have to clear the same set: left in place they
/// re-migrate on the next load and undo the edit that was just saved.
const LEGACY_AGENT_KEYS: [&str; 4] = [
    "default_permission_mode",
    "fold_mode",
    "tail_window",
    "display_filter",
];

/// Clear the migrated keys from an `[agent]` table, reporting whether any was
/// there.
pub(super) fn remove_legacy_agent_keys(table: &mut dyn toml_edit::TableLike) -> bool {
    // A loop rather than `any`, which short-circuits and would leave every key
    // after the first hit in the file.
    let mut removed = false;
    for key in LEGACY_AGENT_KEYS {
        removed |= table.remove(key).is_some();
    }
    removed
}

/// The incremental Settings save patches in place, so it clears the same keys
/// the full save does — and rewrites `[[agents]]`, which is where the values
/// they carried have just landed.
pub(super) fn remove_legacy_agent_keys_from(doc: &mut toml_edit::DocumentMut, config: &Config) {
    let removed = doc
        .get_mut("agent")
        .and_then(toml_edit::Item::as_table_like_mut)
        .is_some_and(remove_legacy_agent_keys);
    if removed {
        replace_agents(doc, &config.agents);
    }
}
