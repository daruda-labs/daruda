//! Atomic write / patch / delete for MCP server JSON files.
//!
//! Every public function here mutates a `serde_json::Value` (the
//! caller's raw tree) and writes the whole file via
//! `tempfile::NamedTempFile` + `persist`. macOS rename(2) is atomic
//! at the inode level so partial reads can't observe a torn file —
//! crucial because Claude Code itself reads + rewrites `~/.claude.json`.
//!
//! Every key outside the targeted `mcpServers` map is preserved by
//! definition: we only touch the `mcpServers[name]` sub-tree at the
//! requested [`McpLocation`] (top-level for `.mcp.json` / User scope,
//! `projects[<dir>]` for the Local scope inside `~/.claude.json`).

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde_json::{Map, Value};
use tempfile::NamedTempFile;

use super::{McpLocation, McpScope, McpServer, McpTransport};

/// Errors callers expect — split from `io::Error` so the modal can
/// route the "name already exists" case to the validation banner
/// instead of a generic "filesystem error".
#[derive(Debug)]
pub enum McpPersistError {
    Io(io::Error),
    Json(serde_json::Error),
    /// Caller tried to add a server whose name is already in the
    /// chosen scope.
    DuplicateName {
        scope: McpScope,
        name: String,
    },
    /// Project-scope target requested but no project root is active.
    NoProjectRoot,
    /// Edit / delete target wasn't found in the in-memory state.
    NotFound {
        scope: McpScope,
        name: String,
    },
}

impl std::fmt::Display for McpPersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpPersistError::Io(e) => write!(f, "{e}"),
            McpPersistError::Json(e) => write!(f, "{e}"),
            McpPersistError::DuplicateName { scope, name } => {
                write!(
                    f,
                    "mcp server `{name}` already exists in {} scope",
                    scope.slug()
                )
            }
            McpPersistError::NoProjectRoot => write!(f, "no active project root"),
            McpPersistError::NotFound { scope, name } => {
                write!(f, "mcp server `{name}` not found in {} scope", scope.slug())
            }
        }
    }
}

impl std::error::Error for McpPersistError {}

impl From<io::Error> for McpPersistError {
    fn from(e: io::Error) -> Self {
        McpPersistError::Io(e)
    }
}

impl From<serde_json::Error> for McpPersistError {
    fn from(e: serde_json::Error) -> Self {
        McpPersistError::Json(e)
    }
}

impl From<super::parse::ParseError> for McpPersistError {
    fn from(e: super::parse::ParseError) -> Self {
        match e {
            super::parse::ParseError::Io(e) => McpPersistError::Io(e),
            super::parse::ParseError::Json { source, .. } => McpPersistError::Json(source),
        }
    }
}

/// Submit-form payload for the AddModal / EditModal. The persist layer
/// turns this into a JSON object slot under `mcpServers[name]`.
#[derive(Clone, Debug)]
pub struct McpServerDraft {
    pub name: String,
    pub transport: McpTransport,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub url: Option<String>,
    pub env: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub disabled: bool,
    /// Round-tripped from the existing entry on Edit; empty on Add.
    pub extra: BTreeMap<String, Value>,
}

impl McpServerDraft {
    /// Build a draft pre-populated from an existing server entry —
    /// EditModal uses this to seed its inputs.
    pub fn from_server(s: &McpServer) -> Self {
        Self {
            name: s.name.clone(),
            transport: s.transport,
            command: s.command.clone(),
            args: s.args.clone(),
            url: s.url.clone(),
            env: s.env.clone(),
            headers: s.headers.clone(),
            disabled: s.disabled,
            extra: s.extra.clone(),
        }
    }

    /// Build a JSON object for `mcpServers[name]`. Optional fields
    /// that don't apply to the chosen transport are omitted entirely
    /// rather than written as empty strings — keeps the on-disk form
    /// matching what a hand-written config looks like.
    pub fn to_value(&self) -> Value {
        let mut obj: Map<String, Value> = Map::new();
        obj.insert("type".into(), Value::String(self.transport.slug().into()));
        match self.transport {
            McpTransport::Stdio => {
                if let Some(cmd) = self.command.as_deref().filter(|s| !s.is_empty()) {
                    obj.insert("command".into(), Value::String(cmd.into()));
                }
                if !self.args.is_empty() {
                    obj.insert(
                        "args".into(),
                        Value::Array(self.args.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
            McpTransport::Sse | McpTransport::Http => {
                if let Some(url) = self.url.as_deref().filter(|s| !s.is_empty()) {
                    obj.insert("url".into(), Value::String(url.into()));
                }
                if !self.headers.is_empty() {
                    obj.insert("headers".into(), Value::Object(string_map(&self.headers)));
                }
            }
        }
        if !self.env.is_empty() {
            obj.insert("env".into(), Value::Object(string_map(&self.env)));
        }
        if self.disabled {
            obj.insert("disabled".into(), Value::Bool(true));
        }
        for (k, v) in &self.extra {
            // `extra` doesn't include any of the known keys (parser
            // strips them) — re-insert verbatim.
            obj.insert(k.clone(), v.clone());
        }
        Value::Object(obj)
    }
}

fn string_map(src: &BTreeMap<String, String>) -> Map<String, Value> {
    src.iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect()
}

/// Toggle (or set) the `disabled` key on `mcpServers[name]` and write
/// the file back. Other keys (other servers, permissions, hooks) are
/// untouched.
///
/// Staged-copy pattern: the patch is applied to a clone of `raw`,
/// `write_atomic` runs against the clone, and only on success is the
/// caller's `raw` swapped to the new tree. A failed disk write leaves
/// every byte of `raw` (and disk) unchanged.
pub fn set_disabled(
    raw: &mut Value,
    path: &Path,
    scope: McpScope,
    location: &McpLocation,
    name: &str,
    disabled: bool,
) -> Result<(), McpPersistError> {
    let mut staged = raw.clone();
    {
        let servers = ensure_mcp_servers_map_at(&mut staged, location)?;
        let entry = servers
            .get_mut(name)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| McpPersistError::NotFound {
                scope,
                name: name.to_string(),
            })?;
        if disabled {
            entry.insert("disabled".into(), Value::Bool(true));
        } else {
            entry.remove("disabled");
        }
    }
    write_atomic(path, &staged)?;
    *raw = staged;
    Ok(())
}

/// Add a brand-new server. Fails with `DuplicateName` if the name is
/// already present so the modal can route to its validation banner.
/// Staged-copy: see [`set_disabled`].
pub fn write_server(
    raw: &mut Value,
    path: &Path,
    scope: McpScope,
    location: &McpLocation,
    draft: &McpServerDraft,
) -> Result<(), McpPersistError> {
    let mut staged = raw.clone();
    {
        let servers = ensure_mcp_servers_map_at(&mut staged, location)?;
        if servers.contains_key(&draft.name) {
            return Err(McpPersistError::DuplicateName {
                scope,
                name: draft.name.clone(),
            });
        }
        servers.insert(draft.name.clone(), draft.to_value());
    }
    write_atomic(path, &staged)?;
    *raw = staged;
    Ok(())
}

/// Replace the entry under `draft.name` with the new payload. Errors
/// when no entry exists — Edit modal targets a known row by definition.
/// Staged-copy: see [`set_disabled`].
pub fn update_server(
    raw: &mut Value,
    path: &Path,
    scope: McpScope,
    location: &McpLocation,
    draft: &McpServerDraft,
) -> Result<(), McpPersistError> {
    let mut staged = raw.clone();
    {
        let servers = ensure_mcp_servers_map_at(&mut staged, location)?;
        if !servers.contains_key(&draft.name) {
            return Err(McpPersistError::NotFound {
                scope,
                name: draft.name.clone(),
            });
        }
        servers.insert(draft.name.clone(), draft.to_value());
    }
    write_atomic(path, &staged)?;
    *raw = staged;
    Ok(())
}

/// Remove the named server. Empty `mcpServers` map is left in place
/// (rather than removing the key) — this matches the way Claude Code
/// generates the file on first use. Staged-copy: see [`set_disabled`].
pub fn delete_server(
    raw: &mut Value,
    path: &Path,
    scope: McpScope,
    location: &McpLocation,
    name: &str,
) -> Result<(), McpPersistError> {
    let mut staged = raw.clone();
    {
        let servers = ensure_mcp_servers_map_at(&mut staged, location)?;
        if servers.remove(name).is_none() {
            return Err(McpPersistError::NotFound {
                scope,
                name: name.to_string(),
            });
        }
    }
    write_atomic(path, &staged)?;
    *raw = staged;
    Ok(())
}

/// Ensure the document has an object at `location`'s `mcpServers` map,
/// creating intermediate containers (`projects`, `projects[<dir>]`) as
/// needed, and return a mutable view for patching.
///
/// Refuses to overwrite a non-object value (e.g. a corrupt
/// `"mcpServers": []`) — bubbling an `InvalidData` error so the modal
/// banner surfaces the malformed file instead of silently destroying
/// it. The renderer's empty-state still applies because
/// [`extract_servers_at`](super::parse::extract_servers_at) reads the
/// same value via `as_object()` which returns `None` for non-objects.
fn ensure_mcp_servers_map_at<'a>(
    root: &'a mut Value,
    location: &McpLocation,
) -> Result<&'a mut Map<String, Value>, McpPersistError> {
    let Some(top) = root.as_object_mut() else {
        return Err(invalid("config file root is not a JSON object"));
    };
    let container: &mut Map<String, Value> = match location {
        McpLocation::TopLevel => top,
        McpLocation::ProjectChild(dir) => {
            let projects = top
                .entry("projects".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| invalid("`projects` is not a JSON object"))?;
            projects
                .entry(dir.clone())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| invalid("`projects[<dir>]` is not a JSON object"))?
        }
    };
    let entry = container
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    entry
        .as_object_mut()
        .ok_or_else(|| invalid("`mcpServers` is not a JSON object"))
}

fn invalid(msg: &'static str) -> McpPersistError {
    McpPersistError::Io(io::Error::new(io::ErrorKind::InvalidData, msg))
}

/// Pretty-print + atomic rename. Parent directory is created if
/// needed (first write to `~/.claude/` on a fresh install).
pub fn write_atomic(path: &Path, value: &Value) -> Result<(), McpPersistError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = NamedTempFile::new_in(parent)?;
    let formatted = serde_json::to_string_pretty(value)?;
    use std::io::Write as _;
    tmp.as_file_mut().write_all(formatted.as_bytes())?;
    tmp.as_file_mut().write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .map_err(|e| McpPersistError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod persist_tests {
    use super::*;
    use serde_json::json;

    /// Shorthand for the top-level location used by most tests.
    const TL: McpLocation = McpLocation::TopLevel;

    fn fresh_root_with_perms() -> Value {
        json!({
            "permissions": { "allow": ["Read", "Write"] },
            "hooks": { "Stop": [] },
            "mcpServers": {
                "alpha": { "type": "stdio", "command": "node", "args": ["a.js"] }
            }
        })
    }

    #[test]
    fn set_disabled_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut root = fresh_root_with_perms();
        write_atomic(&path, &root).unwrap();

        set_disabled(&mut root, &path, McpScope::User, &TL, "alpha", true).unwrap();
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["mcpServers"]["alpha"]["disabled"],
            Value::Bool(true)
        );
        // Sibling keys preserved.
        assert!(on_disk.get("permissions").is_some());
        assert!(on_disk.get("hooks").is_some());

        set_disabled(&mut root, &path, McpScope::User, &TL, "alpha", false).unwrap();
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(on_disk["mcpServers"]["alpha"].get("disabled").is_none());

        // Failed set_disabled (NotFound) must leave raw untouched.
        let snapshot = root.clone();
        let err = set_disabled(&mut root, &path, McpScope::User, &TL, "missing", true);
        assert!(matches!(err, Err(McpPersistError::NotFound { .. })));
        assert_eq!(root, snapshot, "failed write must leave raw untouched");
    }

    #[test]
    fn invalid_root_object_errors_without_destroying_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // Hand-edit a file whose root is an array → write_server errors.
        std::fs::write(&path, b"[]\n").unwrap();
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let err = write_server(
            &mut root,
            &path,
            McpScope::User,
            &TL,
            &McpServerDraft {
                name: "x".into(),
                transport: McpTransport::Stdio,
                command: Some("node".into()),
                args: vec![],
                url: None,
                env: BTreeMap::new(),
                headers: BTreeMap::new(),
                disabled: false,
                extra: BTreeMap::new(),
            },
        );
        assert!(matches!(err, Err(McpPersistError::Io(_))));
        // Disk must still be the original `[]\n`.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]\n");
    }

    #[test]
    fn write_server_rejects_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut root = fresh_root_with_perms();
        write_atomic(&path, &root).unwrap();

        let draft = McpServerDraft {
            name: "alpha".into(),
            transport: McpTransport::Stdio,
            command: Some("node".into()),
            args: vec![],
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            disabled: false,
            extra: BTreeMap::new(),
        };
        let err = write_server(&mut root, &path, McpScope::User, &TL, &draft).unwrap_err();
        assert!(matches!(err, McpPersistError::DuplicateName { .. }));
    }

    #[test]
    fn local_scope_writes_under_projects_child() {
        // Local scope nests servers under `projects[<dir>].mcpServers`
        // and must not disturb the top-level `mcpServers` (User scope)
        // or any other project entry in the shared `~/.claude.json`.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        let mut root = json!({
            "mcpServers": { "user_server": { "type": "stdio", "command": "u" } },
            "projects": {
                "/repo/other": { "mcpServers": { "keep": { "command": "k" } }, "history": [1, 2] }
            }
        });
        write_atomic(&path, &root).unwrap();

        let loc = McpLocation::ProjectChild("/repo/a".into());
        let draft = McpServerDraft {
            name: "local_a".into(),
            transport: McpTransport::Stdio,
            command: Some("node".into()),
            args: vec![],
            url: None,
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            disabled: false,
            extra: BTreeMap::new(),
        };
        write_server(&mut root, &path, McpScope::Local, &loc, &draft).unwrap();

        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // New Local server landed under projects[/repo/a].
        assert_eq!(
            on_disk["projects"]["/repo/a"]["mcpServers"]["local_a"]["command"],
            Value::String("node".into())
        );
        // User scope + other project entries untouched.
        assert!(on_disk["mcpServers"].get("user_server").is_some());
        assert!(
            on_disk["projects"]["/repo/other"]["mcpServers"]
                .get("keep")
                .is_some()
        );
        assert_eq!(on_disk["projects"]["/repo/other"]["history"], json!([1, 2]));

        // Toggle + delete round-trip at the same location.
        set_disabled(&mut root, &path, McpScope::Local, &loc, "local_a", true).unwrap();
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["projects"]["/repo/a"]["mcpServers"]["local_a"]["disabled"],
            Value::Bool(true)
        );
        delete_server(&mut root, &path, McpScope::Local, &loc, "local_a").unwrap();
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            on_disk["projects"]["/repo/a"]["mcpServers"]
                .get("local_a")
                .is_none()
        );
    }

    #[test]
    fn delete_preserves_other_servers_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut root = json!({
            "permissions": { "allow": [] },
            "mcpServers": {
                "alpha": { "type": "stdio", "command": "a" },
                "beta":  { "type": "stdio", "command": "b" }
            }
        });
        write_atomic(&path, &root).unwrap();

        delete_server(&mut root, &path, McpScope::User, &TL, "alpha").unwrap();
        let on_disk: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(on_disk["mcpServers"].get("alpha").is_none());
        assert!(on_disk["mcpServers"].get("beta").is_some());
        assert!(on_disk.get("permissions").is_some());
    }

    #[test]
    fn write_atomic_creates_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("nested")
            .join("subdir")
            .join("settings.json");
        write_atomic(&path, &json!({"mcpServers": {}})).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn draft_to_value_omits_irrelevant_fields_for_remote() {
        let draft = McpServerDraft {
            name: "remote".into(),
            transport: McpTransport::Http,
            command: Some("ignored".into()),
            args: vec!["ignored".into()],
            url: Some("https://example.com".into()),
            env: BTreeMap::new(),
            headers: [("Authorization".to_string(), "Bearer x".to_string())]
                .into_iter()
                .collect(),
            disabled: false,
            extra: BTreeMap::new(),
        };
        let v = draft.to_value();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.get("type").unwrap(), "http");
        assert!(obj.get("command").is_none());
        assert!(obj.get("args").is_none());
        assert_eq!(obj.get("url").unwrap(), "https://example.com");
        assert!(obj.get("headers").is_some());
    }

    #[test]
    fn ensure_servers_map_creates_empty_when_missing() {
        let mut root = json!({"permissions": {"allow": []}});
        let _ = ensure_mcp_servers_map_at(&mut root, &TL);
        assert!(root["mcpServers"].is_object());
        assert!(root["permissions"].is_object());
    }
}
