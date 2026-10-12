//! Native Codex MCP editing with record-level conflicts and TOML preservation.

use crate::agent::mcp::{McpLocation, McpScope, McpServer, McpServerDraft, extract_servers_at};
use serde_json::Value;
use std::path::Path;
use toml_edit::{DocumentMut, Item};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Trust {
    Trusted,
    Untrusted,
    #[default]
    Unknown,
}

fn invalid() -> std::io::Error {
    std::io::Error::other("Invalid or unsupported MCP configuration")
}

pub(crate) fn read(path: &Path) -> std::io::Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(source) => source.parse().map_err(|_| invalid()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(e),
    }
}

pub(crate) fn trust(doc: &DocumentMut, dirs: &[std::path::PathBuf]) -> Trust {
    for dir in dirs {
        if let Some(value) = doc
            .get("projects")
            .and_then(|p| p.get(dir.to_string_lossy().as_ref()))
            .and_then(|p| p.get("trust_level"))
            .and_then(Item::as_str)
        {
            return match value {
                "trusted" => Trust::Trusted,
                "untrusted" => Trust::Untrusted,
                _ => Trust::Unknown,
            };
        }
    }
    Trust::Unknown
}

pub(crate) fn servers(doc: &DocumentMut, scope: McpScope) -> std::io::Result<Vec<McpServer>> {
    let raw: toml::Value = toml::from_str(&doc.to_string()).map_err(|_| invalid())?;
    let json = serde_json::to_value(raw).map_err(|_| invalid())?;
    let Some(entries) = json.get("mcp_servers") else {
        return Ok(Vec::new());
    };
    let mut entries = entries.as_object().ok_or_else(invalid)?.clone();
    for value in entries.values_mut() {
        let server = value.as_object_mut().ok_or_else(invalid)?;
        validate(server)?;
        if let Some(headers) = server.remove("http_headers") {
            server.insert("headers".into(), headers);
        }
        let disabled = server
            .remove("enabled")
            .and_then(|v| v.as_bool())
            .is_some_and(|v| !v);
        server.insert("disabled".into(), disabled.into());
    }
    Ok(extract_servers_at(
        &serde_json::json!({"mcpServers":entries}),
        &McpLocation::TopLevel,
        scope,
    ))
}

fn validate(server: &serde_json::Map<String, Value>) -> std::io::Result<()> {
    for key in ["command", "url", "bearer_token_env_var", "cwd"] {
        if server.get(key).is_some_and(|v| !v.is_string()) {
            return Err(invalid());
        }
    }
    for key in ["enabled", "required"] {
        if server.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(invalid());
        }
    }
    for key in ["args", "enabled_tools", "disabled_tools", "scopes"] {
        if server.get(key).is_some_and(|v| {
            v.as_array()
                .is_none_or(|v| v.iter().any(|v| !v.is_string()))
        }) {
            return Err(invalid());
        }
    }
    for key in ["env", "http_headers", "env_http_headers"] {
        if server.get(key).is_some_and(|v| {
            v.as_object()
                .is_none_or(|v| v.values().any(|v| !v.is_string()))
        }) {
            return Err(invalid());
        }
    }
    for key in [
        "startup_timeout_sec",
        "startup_timeout_ms",
        "tool_timeout_sec",
    ] {
        if server
            .get(key)
            .is_some_and(|v| v.as_f64().is_none_or(|n| !n.is_finite() || n < 0.0))
        {
            return Err(invalid());
        }
    }
    if server.contains_key("command") == server.contains_key("url") {
        return Err(invalid());
    }
    Ok(())
}

fn merge_item(current: &mut Item, mut replacement: Item) {
    if let (Some(old), Some(new)) = (current.as_table_like_mut(), replacement.as_table_like()) {
        let removed: Vec<_> = old
            .iter()
            .filter(|(key, _)| !new.contains_key(key))
            .map(|(key, _)| key.to_string())
            .collect();
        for key in removed {
            old.remove(&key);
        }
        for (key, item) in new.iter() {
            if let Some(previous) = old.get_mut(key) {
                merge_item(previous, item.clone());
            } else {
                old.insert(key, item.clone());
            }
        }
        return;
    }
    if let (Some(old), Some(new)) = (current.as_value(), replacement.as_value_mut()) {
        if old.to_string() == new.to_string() {
            return;
        }
        *new.decor_mut() = old.decor().clone();
    }
    *current = replacement;
}

fn wire(draft: &McpServerDraft) -> std::io::Result<toml::Table> {
    if draft.transport == crate::agent::mcp::McpTransport::Sse {
        return Err(invalid());
    }
    let mut value = draft.to_value();
    let object = value.as_object_mut().ok_or_else(invalid)?;
    object.remove("type");
    object.remove("disabled");
    object.insert("enabled".into(), (!draft.disabled).into());
    if let Some(headers) = object.remove("headers") {
        object.insert("http_headers".into(), headers);
    }
    validate(object)?;
    let table: toml::Table =
        serde_json::from_value(Value::Object(object.clone())).map_err(|_| invalid())?;
    Ok(table)
}

/// A missing expected record means create; a missing draft means delete.
/// Unrelated changes are merged into the latest source. No cross-process lock
/// is claimed: external writers can still race the atomic replacement.
pub(crate) fn write(
    path: &Path,
    name: &str,
    expected: Option<&McpServerDraft>,
    draft: Option<&McpServerDraft>,
) -> std::io::Result<()> {
    let physical = match daruda_core::path::canonicalize(path) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => return Err(e),
    };
    let mut doc = read(&physical)?;
    let current = servers(&doc, McpScope::User)?
        .into_iter()
        .find(|s| s.name == name);
    let current = current.as_ref().map(McpServerDraft::from_server);
    if current.as_ref().map(McpServerDraft::to_value) != expected.map(McpServerDraft::to_value) {
        return Err(std::io::Error::other(
            "MCP source changed; reload before saving",
        ));
    }
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = Item::Table(toml_edit::Table::new());
    }
    let servers = doc["mcp_servers"].as_table_like_mut().ok_or_else(invalid)?;
    if let Some(draft) = draft {
        let desired = wire(draft)?;
        let previous = expected.map(wire).transpose()?.unwrap_or_default();
        if servers.get(name).is_none() {
            servers.insert(name, Item::Table(toml_edit::Table::new()));
        }
        let record = servers
            .get_mut(name)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(invalid)?;
        for key in previous.keys() {
            if !desired.contains_key(key) {
                record.remove(key);
            }
        }
        for (key, value) in &desired {
            if previous.get(key) == Some(value) {
                continue;
            }
            let fragment = toml::to_string(&toml::Table::from_iter([(key.clone(), value.clone())]))
                .map_err(|_| invalid())?;
            let parsed: DocumentMut = fragment.parse().map_err(|_| invalid())?;
            let item = parsed[key].clone();
            if let Some(old) = record.get_mut(key) {
                merge_item(old, item);
            } else {
                record.insert(key, item);
            }
        }
    } else {
        servers.remove(name);
    }
    let parent = physical.parent().ok_or_else(invalid)?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temp.write_all(doc.to_string().as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(&physical).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_patch_preserves_extensions_comments_and_unrelated_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let source = "# config\nmodel = 'sample'\n[mcp_servers.docs]\nurl = 'https://example.com/mcp' # connection\nbearer_token_env_var = 'TOKEN'\nstartup_timeout_sec = 42\n[projects.'/repo']\ntrust_level = 'untrusted'\n";
        std::fs::write(&path, source).unwrap();
        let server = servers(&read(&path).unwrap(), McpScope::User)
            .unwrap()
            .remove(0);
        let expected = McpServerDraft::from_server(&server);
        let mut draft = expected.clone();
        draft.url = Some("https://example.com/new".into());
        write(&path, "docs", Some(&expected), Some(&draft)).unwrap();
        let updated = std::fs::read_to_string(&path).unwrap();
        for preserved in [
            "# config",
            "# connection",
            "bearer_token_env_var = 'TOKEN'",
            "startup_timeout_sec = 42",
            "trust_level = 'untrusted'",
        ] {
            assert!(updated.contains(preserved), "{preserved}");
        }
        assert!(write(&path, "docs", Some(&expected), None).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
    }
}
