//! JSON parsers for the two MCP server scopes.
//!
//! Personal: `~/.claude/settings.json` `mcpServers` (along with every
//! sibling key Claude Code uses — `permissions`, `hooks`,
//! `enableAllProjectMcpServers`, etc., all of which we round-trip).
//!
//! Project: `<worktree>/.mcp.json` (usually contains only `mcpServers`,
//! but the spec doesn't forbid sibling keys).
//!
//! Both parsers return `(Vec<McpServer>, serde_json::Value)`. The raw
//! `Value` is what `persist::*` patches on save so non-`mcpServers`
//! keys survive every write-back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{McpScope, McpServer, McpTransport};

/// Filename used at every active worktree root for project-scope MCP.
pub const PROJECT_MCP_FILE: &str = ".mcp.json";

/// Filename of the personal Claude Code settings file (relative to
/// `~/.claude/`).
pub const PERSONAL_SETTINGS_FILE: &str = "settings.json";

/// Errors surfaced by the parsers. `read_to_string` failures other
/// than NotFound bubble out as `Io`. Parse failures keep the bytes that
/// triggered the error so the caller can log a readable diagnostic.
#[derive(Debug)]
pub enum ParseError {
    Io(std::io::Error),
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "{e}"),
            ParseError::Json { path, source } => {
                write!(f, "{}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}

/// Resolve the personal settings path (`~/.claude/settings.json`).
/// `dirs::home_dir()` returning `None` (no `HOME` env) falls back to a
/// relative `.claude/settings.json` so daruda still functions in
/// constrained environments — the watcher's parent-walk handles the
/// missing-directory case from there.
pub fn personal_settings_path() -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".claude").join(PERSONAL_SETTINGS_FILE)
}

/// Resolve the project-scope `.mcp.json` path for a worktree root.
pub fn project_mcp_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(PROJECT_MCP_FILE)
}

/// Read + parse `~/.claude/settings.json`. NotFound is treated as
/// "empty mcpServers" so a fresh install doesn't bubble an error up to
/// `Workspace::new`. Missing files round-trip through an empty
/// `serde_json::Object` so the first save lays down a well-formed
/// `{ "mcpServers": {} }` document instead of overwriting siblings we
/// haven't seen yet.
pub fn parse_personal_settings(path: &Path) -> Result<(Vec<McpServer>, Value), ParseError> {
    parse_settings_like(path, McpScope::Personal)
}

/// Read + parse `<worktree>/.mcp.json`. Same NotFound semantics as
/// `parse_personal_settings`.
pub fn parse_project_mcp(path: &Path) -> Result<(Vec<McpServer>, Value), ParseError> {
    parse_settings_like(path, McpScope::Project)
}

fn parse_settings_like(
    path: &Path,
    scope: McpScope,
) -> Result<(Vec<McpServer>, Value), ParseError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), Value::Object(Map::new())));
        }
        Err(e) => return Err(ParseError::Io(e)),
    };
    let value: Value = if raw.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(&raw).map_err(|e| ParseError::Json {
            path: path.to_path_buf(),
            source: e,
        })?
    };
    let servers = extract_servers(&value, scope);
    Ok((servers, value))
}

/// Extract the `mcpServers` map from a parsed top-level document and
/// turn it into typed [`McpServer`]s. Anything daruda doesn't model
/// is preserved in `extra` for round-trip.
pub fn extract_servers(top: &Value, scope: McpScope) -> Vec<McpServer> {
    let Some(map) = top.get("mcpServers").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<McpServer> = map
        .iter()
        .map(|(name, v)| parse_one(name, v, scope))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn parse_one(name: &str, v: &Value, scope: McpScope) -> McpServer {
    let obj = v.as_object().cloned().unwrap_or_default();

    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let url = obj.get("url").and_then(Value::as_str).map(str::to_owned);
    let env = obj
        .get("env")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let transport = match obj.get("type").and_then(Value::as_str) {
        Some("sse") => McpTransport::Sse,
        Some("http") => McpTransport::Http,
        Some("stdio") => McpTransport::Stdio,
        // Type omitted: infer from command/url presence.
        _ => {
            if command.is_some() {
                McpTransport::Stdio
            } else if url.is_some() {
                McpTransport::Http
            } else {
                McpTransport::Stdio
            }
        }
    };
    let disabled = obj
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // `headers` is transport-specific (`Sse`/`Http` only). For remote
    // transports we extract it into the typed field and exclude it from
    // `extra` so saving doesn't double-emit. For `Stdio` the key (if
    // present at all — corrupted or future-schema files) is left in
    // `extra` for verbatim round-trip; we never want to silently drop
    // unknown user data.
    let mut extra = collect_extra(&obj);
    let headers: BTreeMap<String, String> = if transport.is_remote() {
        let h = obj
            .get("headers")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        extra.remove("headers");
        h
    } else {
        BTreeMap::new()
    };

    McpServer {
        name: name.to_owned(),
        scope,
        transport,
        command,
        args,
        url,
        env,
        headers,
        disabled,
        extra,
    }
}

/// Keys daruda models as typed fields. `headers` is *not* in this
/// list — it is transport-conditional and handled in `parse_one`. Any
/// other key is preserved verbatim through `extra`.
const KNOWN_KEYS: &[&str] = &["command", "args", "url", "env", "type", "disabled"];

fn collect_extra(obj: &Map<String, Value>) -> BTreeMap<String, Value> {
    obj.iter()
        .filter(|(k, _)| !KNOWN_KEYS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn extract_servers_sorts_alphabetically() {
        let raw = serde_json::json!({
            "mcpServers": {
                "zulip": { "command": "npx", "args": ["-y", "z"] },
                "alpha": { "command": "node", "args": [] },
            }
        });
        let servers = extract_servers(&raw, McpScope::Personal);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "alpha");
        assert_eq!(servers[1].name, "zulip");
    }

    #[test]
    fn missing_type_infers_from_command_or_url() {
        let raw = serde_json::json!({
            "mcpServers": {
                "stdio_one": { "command": "node" },
                "http_one":  { "url": "https://example.com/mcp" },
                "empty":     {}
            }
        });
        let s = extract_servers(&raw, McpScope::Personal);
        assert_eq!(
            s.iter().find(|s| s.name == "stdio_one").unwrap().transport,
            McpTransport::Stdio
        );
        assert_eq!(
            s.iter().find(|s| s.name == "http_one").unwrap().transport,
            McpTransport::Http
        );
        assert_eq!(
            s.iter().find(|s| s.name == "empty").unwrap().transport,
            McpTransport::Stdio
        );
    }

    #[test]
    fn unknown_keys_land_in_extra() {
        let raw = serde_json::json!({
            "mcpServers": {
                "future": {
                    "command": "node",
                    "experimentalFlag": true,
                    "policy": { "k": 1 }
                }
            }
        });
        let s = extract_servers(&raw, McpScope::Personal);
        let one = &s[0];
        assert!(one.extra.contains_key("experimentalFlag"));
        assert!(one.extra.contains_key("policy"));
        assert!(!one.extra.contains_key("command"));
    }

    #[test]
    fn disabled_default_false() {
        let raw = serde_json::json!({
            "mcpServers": {
                "x": { "command": "node" },
                "y": { "command": "node", "disabled": true }
            }
        });
        let s = extract_servers(&raw, McpScope::Personal);
        assert!(!s.iter().find(|s| s.name == "x").unwrap().disabled);
        assert!(s.iter().find(|s| s.name == "y").unwrap().disabled);
    }

    #[test]
    fn missing_top_level_mcp_servers_returns_empty() {
        let raw = serde_json::json!({"permissions": {"allow": []}});
        let s = extract_servers(&raw, McpScope::Personal);
        assert!(s.is_empty());
    }

    #[test]
    fn empty_file_path_returns_empty_object_value() {
        // Sanity check that callers can build a state from an absent
        // file. parse_settings_like uses NotFound branch internally;
        // here we simulate the post-condition.
        let value = Value::Object(Map::new());
        assert!(extract_servers(&value, McpScope::Personal).is_empty());
    }
}
