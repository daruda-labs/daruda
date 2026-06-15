//! JSON parsers for the three MCP server scopes.
//!
//! User: `~/.claude.json` top-level `mcpServers`.
//! Local: `~/.claude.json` `projects[<lane>].mcpServers`.
//! Project: `<lane>/.mcp.json` (usually contains only `mcpServers`,
//! but the spec doesn't forbid sibling keys).
//!
//! User and Local share one file (`~/.claude.json`); callers read it
//! once via [`read_json_or_empty`] and pull each scope's servers out
//! with [`extract_servers_at`]. The raw `Value` is what `persist::*`
//! patches on save so every key outside the patched `mcpServers` map
//! survives the write-back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{McpLocation, McpScope, McpServer, McpTransport};

/// Filename used at every active lane root for project-scope MCP.
pub const PROJECT_MCP_FILE: &str = ".mcp.json";

/// Filename of the Claude Code config file (relative to `~/`) that holds
/// the User and Local scopes.
pub const CLAUDE_JSON_FILE: &str = ".claude.json";

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

/// Resolve the Claude Code config path (`~/.claude.json`) that holds
/// the User and Local scopes. `dirs::home_dir()` returning `None` (no
/// `HOME` env) falls back to a relative `.claude.json` so daruda still
/// functions in constrained environments — the watcher's parent-walk
/// handles the missing-file case from there.
pub fn claude_json_path() -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(CLAUDE_JSON_FILE)
}

/// Resolve the project-scope `.mcp.json` path for a lane root.
pub fn project_mcp_path(worktree_root: &Path) -> PathBuf {
    worktree_root.join(PROJECT_MCP_FILE)
}

/// Read a config file's raw bytes. NotFound is treated as empty content
/// (an empty document downstream) so a fresh install doesn't error.
pub fn read_bytes_or_empty(path: &Path) -> Result<Vec<u8>, ParseError> {
    match std::fs::read(path) {
        Ok(b) => Ok(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(ParseError::Io(e)),
    }
}

/// Parse raw bytes into a JSON `Value`. Empty / whitespace-only content
/// yields an empty object so the first save lays down a well-formed
/// document instead of overwriting keys we haven't seen.
pub fn parse_value(bytes: &[u8], path: &Path) -> Result<Value, ParseError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_slice(bytes).map_err(|e| ParseError::Json {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Stable 64-bit content hash. Used to skip re-parsing an unchanged
/// (often multi-megabyte) `~/.claude.json` when the watcher fires on a
/// write that left the bytes identical (self-write echoes, coalesced
/// FSEvents). Reading the bytes is unavoidable; this only avoids the
/// far costlier `serde_json` parse + `Value` tree build + clone. Takes
/// `&[u8]` (not a path) so the caller reads the file once and reuses
/// the same bytes for `parse_value` on a cache miss.
pub fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Read + parse a JSON config file into a `serde_json::Value` (NotFound
/// → empty object). Convenience for callers that don't need the
/// hash gate (small files like `.mcp.json`).
pub fn read_json_or_empty(path: &Path) -> Result<Value, ParseError> {
    parse_value(&read_bytes_or_empty(path)?, path)
}

/// Read + parse `<lane>/.mcp.json` into `(servers, raw)`. Project scope
/// always reads the top-level `mcpServers` map.
pub fn parse_project_mcp(path: &Path) -> Result<(Vec<McpServer>, Value), ParseError> {
    let value = read_json_or_empty(path)?;
    let servers = extract_servers_at(&value, &McpLocation::TopLevel, McpScope::Project);
    Ok((servers, value))
}

/// Extract the `mcpServers` map at `location` from a parsed document
/// and turn it into typed [`McpServer`]s. Anything daruda doesn't model
/// is preserved in `extra` for round-trip. Returns empty when the
/// location (or its `mcpServers` map) is absent.
pub fn extract_servers_at(top: &Value, location: &McpLocation, scope: McpScope) -> Vec<McpServer> {
    let map = match location {
        McpLocation::TopLevel => top.get("mcpServers").and_then(Value::as_object),
        McpLocation::ProjectChild(dir) => top
            .get("projects")
            .and_then(Value::as_object)
            .and_then(|projects| projects.get(dir))
            .and_then(Value::as_object)
            .and_then(|proj| proj.get("mcpServers"))
            .and_then(Value::as_object),
    };
    let Some(map) = map else {
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

    fn top() -> McpLocation {
        McpLocation::TopLevel
    }

    #[test]
    fn extract_servers_sorts_alphabetically() {
        let raw = serde_json::json!({
            "mcpServers": {
                "zulip": { "command": "npx", "args": ["-y", "z"] },
                "alpha": { "command": "node", "args": [] },
            }
        });
        let servers = extract_servers_at(&raw, &top(), McpScope::User);
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
        let s = extract_servers_at(&raw, &top(), McpScope::User);
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
        let s = extract_servers_at(&raw, &top(), McpScope::User);
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
        let s = extract_servers_at(&raw, &top(), McpScope::User);
        assert!(!s.iter().find(|s| s.name == "x").unwrap().disabled);
        assert!(s.iter().find(|s| s.name == "y").unwrap().disabled);
    }

    #[test]
    fn missing_top_level_mcp_servers_returns_empty() {
        let raw = serde_json::json!({"permissions": {"allow": []}});
        let s = extract_servers_at(&raw, &top(), McpScope::User);
        assert!(s.is_empty());
    }

    #[test]
    fn local_scope_reads_projects_child_map() {
        // Local scope lives under `projects[<dir>].mcpServers`.
        let raw = serde_json::json!({
            "mcpServers": { "user_server": { "command": "u" } },
            "projects": {
                "/repo/a": { "mcpServers": { "local_a": { "command": "a" } } },
                "/repo/b": { "mcpServers": { "local_b": { "command": "b" } } }
            }
        });
        let a = extract_servers_at(
            &raw,
            &McpLocation::ProjectChild("/repo/a".into()),
            McpScope::Local,
        );
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "local_a");
        // A project without an entry yields no Local servers.
        let none = extract_servers_at(
            &raw,
            &McpLocation::ProjectChild("/repo/missing".into()),
            McpScope::Local,
        );
        assert!(none.is_empty());
    }

    #[test]
    fn content_hash_stable_and_sensitive() {
        // Same bytes → same hash (gate stays closed on unchanged files).
        assert_eq!(content_hash(b"{}"), content_hash(b"{}"));
        // One byte different → different hash (gate opens on real edits).
        assert_ne!(content_hash(b"{}"), content_hash(b"{ }"));
    }

    #[test]
    fn parse_value_empty_and_whitespace_yield_empty_object() {
        let p = Path::new("x");
        assert_eq!(parse_value(b"", p).unwrap(), Value::Object(Map::new()));
        assert_eq!(
            parse_value(b"  \n\t", p).unwrap(),
            Value::Object(Map::new())
        );
        // Malformed content still errors (not silently empty).
        assert!(parse_value(b"{bad", p).is_err());
    }

    #[test]
    fn empty_file_path_returns_empty_object_value() {
        // Sanity check that callers can build a state from an absent
        // file. `read_json_or_empty` returns an empty object on NotFound;
        // here we simulate the post-condition.
        let value = Value::Object(Map::new());
        assert!(extract_servers_at(&value, &top(), McpScope::User).is_empty());
    }
}
