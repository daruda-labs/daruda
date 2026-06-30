//! MCP server data model — GPUI-free types describing the on-disk
//! layout of the three MCP server scopes Claude Code reads (matching
//! the official scope table at <https://code.claude.com/docs/en/mcp>):
//!
//! - **User** — `~/.claude.json` top-level `mcpServers` (available
//!   across every project).
//! - **Local** — `~/.claude.json` `projects[<lane>].mcpServers` (private
//!   to the current project).
//! - **Project** — `<lane>/.mcp.json` (team-shared, committable).
//!
//! Precedence (highest first, per the docs): Local > Project > User.
//!
//! User and Local share one physical file (`~/.claude.json`); Project
//! lives in its own `.mcp.json`. Both files round-trip losslessly: known
//! keys land in typed fields, unknown keys are preserved via `extra`
//! (per server) and via the raw `serde_json::Value` tree (every key
//! outside the patched `mcpServers` map). This is mandatory —
//! `~/.claude.json` holds the user's entire Claude Code state (history,
//! per-project data, auth), and silently dropping any of it on a toggle
//! would be catastrophic.
//!
//! No GPUI imports here — `app/src/CLAUDE.md` G2 / G7 forbid them.
//! This module is consumed by the renderer (`workspace/right_dock/tools/`),
//! the watcher (`hooks/mcp_watcher.rs`), and the CRUD modals.

pub mod global;
pub mod parse;
pub mod persist;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use parse::{
    CLAUDE_JSON_FILE, PROJECT_MCP_FILE, claude_json_path, extract_servers_at, parse_project_mcp,
    project_mcp_path, read_json_or_empty,
};
pub use persist::{
    McpPersistError, McpServerDraft, delete_server, set_disabled, update_server, write_server,
};

/// On-disk scope for an MCP server, matching Claude Code's three
/// installation scopes.
///
/// - `User` — `~/.claude.json` top-level `mcpServers` (cross-project).
/// - `Project` — `<lane>/.mcp.json` (team-shared, committable).
/// - `Local` — `~/.claude.json` `projects[<lane>].mcpServers` (private
///   to the current project).
///
/// Precedence (highest first): Local > Project > User.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum McpScope {
    User,
    Project,
    Local,
}

impl McpScope {
    /// Stable kebab slug for serialization / logging.
    pub fn slug(self) -> &'static str {
        match self {
            McpScope::User => "user",
            McpScope::Project => "project",
            McpScope::Local => "local",
        }
    }

    /// True for the scopes stored in `~/.claude.json` (User + Local).
    /// The Project scope lives in a separate `.mcp.json`.
    pub fn in_claude_json(self) -> bool {
        matches!(self, McpScope::User | McpScope::Local)
    }
}

/// Where inside a parsed JSON document the `mcpServers` map lives.
///
/// `.mcp.json` and the User scope use [`McpLocation::TopLevel`]; the
/// Local scope nests under `projects[<dir>]` inside `~/.claude.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpLocation {
    /// Top-level `mcpServers`.
    TopLevel,
    /// `projects[<dir>].mcpServers`, where `<dir>` is the project
    /// directory path as Claude Code keys it in `~/.claude.json`.
    ProjectChild(String),
}

impl McpLocation {
    /// Build the Local-scope location for a lane root, matching the
    /// path-string key Claude Code writes under `projects`.
    pub fn project(root: &Path) -> Self {
        McpLocation::ProjectChild(root.to_string_lossy().into_owned())
    }
}

/// MCP transport. Claude Code accepts `stdio` (default), `sse`, `http`.
/// Daruda treats unknown / missing `type` as `Stdio` if `command` is
/// present, else `Http` if `url` is present (parser convenience —
/// authoritative classification is set by Claude Code at runtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpTransport {
    Stdio,
    Sse,
    Http,
}

impl McpTransport {
    /// Spec wire value for the `type` field.
    pub fn slug(self) -> &'static str {
        match self {
            McpTransport::Stdio => "stdio",
            McpTransport::Sse => "sse",
            McpTransport::Http => "http",
        }
    }

    /// True for `Sse` / `Http` — the variants that use `url` instead of
    /// `command` + `args`.
    pub fn is_remote(self) -> bool {
        matches!(self, McpTransport::Sse | McpTransport::Http)
    }
}

/// One MCP server entry from disk. Built by the parsers. `PartialEq`
/// backs the reload change-detection that suppresses re-renders when a
/// `~/.claude.json` write didn't touch any MCP server.
#[derive(Clone, Debug, PartialEq)]
pub struct McpServer {
    pub name: String,
    pub scope: McpScope,
    pub transport: McpTransport,
    /// `command` for `Stdio` transport. `None` for remote transports.
    pub command: Option<String>,
    /// `args` for `Stdio` transport. Empty for remote transports.
    pub args: Vec<String>,
    /// `url` for `Sse` / `Http` transports. `None` for `Stdio`.
    pub url: Option<String>,
    /// Environment variables for the spawned process (Stdio) or
    /// custom auth headers source (some setups). `BTreeMap` for stable
    /// key order on round-trip.
    pub env: BTreeMap<String, String>,
    /// HTTP headers for `Sse` / `Http` transports.
    pub headers: BTreeMap<String, String>,
    pub disabled: bool,
    /// `mcpServers[name]` keys daruda doesn't model (e.g. future
    /// schema extensions). Preserved verbatim on round-trip.
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl McpServer {
    /// True when required fields for the chosen transport are absent
    /// (stdio without `command`, sse/http without `url`). Surfaces a
    /// `⚠ malformed` chip on the row.
    pub fn is_malformed(&self) -> bool {
        match self.transport {
            McpTransport::Stdio => self.command.as_deref().is_none_or(str::is_empty),
            McpTransport::Sse | McpTransport::Http => self.url.as_deref().is_none_or(str::is_empty),
        }
    }

    /// Second-line preview shown beneath the command/url line in the
    /// Tools tab row. Returns `None` when there is nothing to show.
    ///
    /// Stdio: `args` (when non-empty) joined by spaces, optionally
    /// followed by `· env: KEY1, KEY2` when env vars are also set.
    /// Showing both at once means env vars stay visible even when args
    /// are populated — earlier revisions hid env behind args.
    ///
    /// Sse / Http: `headers: KEY1, KEY2` listing only the key names
    /// (values may be secret tokens).
    pub fn second_line_preview(&self) -> Option<String> {
        match self.transport {
            McpTransport::Stdio => {
                let mut parts: Vec<String> = Vec::with_capacity(2);
                if !self.args.is_empty() {
                    parts.push(format!("args: {}", self.args.join(" ")));
                }
                if !self.env.is_empty() {
                    parts.push(format!(
                        "env: {}",
                        self.env.keys().cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join(" · "))
                }
            }
            McpTransport::Sse | McpTransport::Http => {
                if self.headers.is_empty() {
                    None
                } else {
                    Some(format!(
                        "headers: {}",
                        self.headers.keys().cloned().collect::<Vec<_>>().join(", ")
                    ))
                }
            }
        }
    }

    /// Stable per-row DOM id used by `gpui::div().id(...)`. Centralised
    /// so render keeps a single `format!()`-free path (G2).
    pub fn row_dom_id(&self) -> String {
        format!("mcp-row-{}-{}", self.scope.slug(), self.name)
    }

    /// 60-char truncated preview shown in the row's command/url column.
    pub fn command_preview(&self) -> String {
        let raw = match self.transport {
            McpTransport::Stdio => self.command.clone().unwrap_or_default(),
            McpTransport::Sse | McpTransport::Http => self.url.clone().unwrap_or_default(),
        };
        if raw.chars().count() <= PREVIEW_MAX_CHARS {
            raw
        } else {
            let mut out: String = raw.chars().take(PREVIEW_MAX_CHARS - 1).collect();
            out.push('…');
            out
        }
    }
}

/// Hard cap for the row's command/url preview. Long node-modules paths
/// would otherwise blow past the panel width; full value lives in the
/// edit modal.
pub const PREVIEW_MAX_CHARS: usize = 60;

/// Per-lane project-scope MCP state. Stored inside
/// [`McpState::project`], one entry per lane root path.
#[derive(Clone, Debug, Default)]
pub struct ProjectMcp {
    pub servers: Vec<McpServer>,
    /// Whole `.mcp.json` parsed tree. Usually has only `mcpServers`,
    /// but the spec doesn't forbid sibling keys. `persist::*` patches
    /// `mcpServers[name]` in-place so unrelated keys survive.
    pub raw: serde_json::Value,
}

/// App-wide MCP server state. Lives as a GPUI Global registered at
/// bootstrap (`global::init`). The User-scope vector and the shared
/// `~/.claude.json` raw tree are global; Local- and Project-scope
/// state is partitioned by lane root path so multiple Workspace
/// windows observing different lanes never collide. Renderers and
/// modals consume a per-lane [`McpSnapshot`] via
/// [`McpState::snapshot_for`].
///
/// Mirrors Zed's `SettingsStore::local_settings` pattern (a single
/// Global with `BTreeMap`s keyed by lane-relative location).
///
/// `claude_json_raw` carries the entire parsed `~/.claude.json` tree so
/// every key outside the patched `mcpServers` map survives every
/// write-back unchanged. This is mandatory — `~/.claude.json` holds the
/// user's complete Claude Code state, and silently dropping any of it
/// on a toggle would be catastrophic.
#[derive(Clone, Debug, Default)]
pub struct McpState {
    /// User scope — `~/.claude.json` top-level `mcpServers`. Shared
    /// across every lane.
    pub user: Vec<McpServer>,
    /// Local scope — `~/.claude.json` `projects[<root>].mcpServers`,
    /// keyed by the lane's absolute root path.
    pub local: BTreeMap<PathBuf, Vec<McpServer>>,
    /// Project scope — `<root>/.mcp.json`, keyed by the lane's
    /// absolute root path. An entry exists for every lane scanned at
    /// least once; opening a different lane adds a new key without
    /// disturbing the others.
    pub project: BTreeMap<PathBuf, ProjectMcp>,
    /// Whole `~/.claude.json` parsed tree. User and Local scopes are
    /// both derived from — and persisted into — this single tree.
    pub claude_json_raw: serde_json::Value,
    /// Content hash of `~/.claude.json` as last parsed into
    /// `claude_json_raw`. `reload_claude_json` skips the expensive
    /// re-parse when a fresh read hashes to the same value.
    pub claude_json_hash: Option<u64>,
    /// Last successful scan timestamp across any scope. `None` until
    /// the first load lands.
    pub last_scanned: Option<SystemTime>,
}

impl McpState {
    /// Reload `~/.claude.json` (User + Local scopes). The whole file is
    /// re-parsed into `claude_json_raw`; User comes from the top-level
    /// `mcpServers`, Local from `projects[<lane>].mcpServers` for the
    /// given lane (other lanes' Local caches are left untouched).
    ///
    /// Returns `true` when the MCP-relevant content (User servers, or
    /// the given lane's Local servers) actually changed — callers use
    /// this to suppress re-renders when an unrelated `~/.claude.json`
    /// write (history append, etc.) fired the watcher. `~/.claude.json`
    /// can be multi-megabyte and Claude Code rewrites it constantly, so
    /// this guard matters.
    ///
    /// Hash gate: the raw bytes are read and hashed every call, but the
    /// `serde_json` parse + `Value` tree build is skipped when the
    /// content is byte-identical to the last parse. On a cache hit only
    /// the requested lane's Local view is (re)derived from the cached
    /// tree, which is cheap.
    pub fn reload_claude_json(&mut self, lane: Option<&Path>) -> Result<bool, parse::ParseError> {
        let path = parse::claude_json_path();
        self.reload_claude_json_at(&path, lane)
    }

    /// Path-injectable core of [`reload_claude_json`] — tests point it
    /// at a temp file instead of the real `~/.claude.json`.
    fn reload_claude_json_at(
        &mut self,
        path: &Path,
        lane: Option<&Path>,
    ) -> Result<bool, parse::ParseError> {
        let bytes = parse::read_bytes_or_empty(path)?;
        let hash = parse::content_hash(&bytes);
        if self.claude_json_hash == Some(hash) {
            // Content unchanged since the last parse — reuse the cached
            // tree. The only thing that can differ is the requested
            // lane's Local view (e.g. a lane we haven't projected yet).
            self.last_scanned = Some(SystemTime::now());
            return Ok(self.refresh_local_from_cache(lane));
        }
        let raw = parse::parse_value(&bytes, path)?;
        let new_user = parse::extract_servers_at(&raw, &McpLocation::TopLevel, McpScope::User);
        let mut changed = new_user != self.user;
        self.user = new_user;
        if let Some(root) = lane {
            let new_local =
                parse::extract_servers_at(&raw, &McpLocation::project(root), McpScope::Local);
            changed |= self.local.get(root) != Some(&new_local);
            self.local.insert(root.to_path_buf(), new_local);
        }
        self.claude_json_raw = raw;
        self.claude_json_hash = Some(hash);
        self.last_scanned = Some(SystemTime::now());
        Ok(changed)
    }

    /// Re-derive one lane's Local servers from the already-parsed
    /// `claude_json_raw` without touching disk. Returns whether that
    /// lane's Local view changed. Used on a hash-gate cache hit.
    fn refresh_local_from_cache(&mut self, lane: Option<&Path>) -> bool {
        let Some(root) = lane else {
            return false;
        };
        let new_local = parse::extract_servers_at(
            &self.claude_json_raw,
            &McpLocation::project(root),
            McpScope::Local,
        );
        let changed = self.local.get(root) != Some(&new_local);
        self.local.insert(root.to_path_buf(), new_local);
        changed
    }

    /// Reload the Project scope (`<lane>/.mcp.json`) for one lane.
    /// Returns `true` when the lane's project servers changed.
    pub fn reload_project(&mut self, lane: Option<&Path>) -> Result<bool, parse::ParseError> {
        let mut changed = false;
        if let Some(w) = lane {
            let path = parse::project_mcp_path(w);
            let (servers, raw) = parse_project_mcp(&path)?;
            changed = self.project.get(w).map(|p| &p.servers) != Some(&servers);
            self.project
                .insert(w.to_path_buf(), ProjectMcp { servers, raw });
        }
        self.last_scanned = Some(SystemTime::now());
        Ok(changed)
    }

    /// Drop a lane's Local + Project entries. Call when a lane is
    /// closed so the `BTreeMap`s don't grow unbounded across the
    /// session.
    pub fn forget_lane(&mut self, root: &Path) {
        self.local.remove(root);
        self.project.remove(root);
    }

    /// Build an owned per-lane view for the renderer / modals.
    /// Carrying it by value keeps the panel render closure off the
    /// Global (no re-entrancy hazard).
    ///
    /// Project scope merges every `.mcp.json` in `project_dirs`,
    /// matching how Claude Code discovers project config by searching
    /// upward from its cwd. Callers pass the lane root, its ancestors up
    /// to the git repo root, and the focused terminal's cwd chain — so a
    /// repo-root `.mcp.json` shows even when the lane is a subdirectory,
    /// without the user having to `cd`.
    ///
    /// `project_dirs` is consulted in order; on a same-name collision
    /// the earlier (nearest-to-lane) entry wins. `lane` itself drives
    /// the Local scope and the write target (`path_for(Project)` →
    /// `lane/.mcp.json`), so it must be the first element.
    pub fn snapshot_for(&self, lane: Option<&Path>, project_dirs: &[PathBuf]) -> McpSnapshot {
        let mut project: Vec<McpServer> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for dir in project_dirs {
            if let Some(p) = self.project.get(dir) {
                for s in &p.servers {
                    if seen.insert(s.name.clone()) {
                        project.push(s.clone());
                    }
                }
            }
        }
        project.sort_by(|a, b| a.name.cmp(&b.name));
        let local = lane
            .and_then(|w| self.local.get(w))
            .cloned()
            .unwrap_or_default();
        McpSnapshot {
            user: self.user.clone(),
            local,
            project,
            project_root: lane.map(Path::to_path_buf),
            project_mcp_path: lane.map(parse::project_mcp_path),
            claude_json_path: parse::claude_json_path(),
            last_scanned: self.last_scanned,
        }
    }
}

/// Owned per-lane projection of [`McpState`] consumed by the
/// renderer and CRUD modals. Carries the project server Vec for
/// *one* lane along with the user-global personal vector. The raw
/// JSON trees stay on [`McpState`] (read directly by the persist
/// layer) and are deliberately absent here — the renderer never
/// reads them, so cloning them per frame would be pure waste.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct McpSnapshot {
    /// User scope (`~/.claude.json` top-level).
    pub user: Vec<McpServer>,
    /// Local scope for the active lane (`~/.claude.json`
    /// `projects[<root>]`).
    pub local: Vec<McpServer>,
    /// Project scope for the active lane (`<root>/.mcp.json`).
    pub project: Vec<McpServer>,
    /// Lane root whose Project + Local servers are carried here.
    /// `None` when the workspace has no active lane.
    pub project_root: Option<PathBuf>,
    /// `<project_root>/.mcp.json`. `None` when `project_root` is `None`.
    pub project_mcp_path: Option<PathBuf>,
    /// Cached `~/.claude.json` resolution (User + Local target).
    pub claude_json_path: PathBuf,
    pub last_scanned: Option<SystemTime>,
}

impl McpSnapshot {
    pub fn servers(&self, scope: McpScope) -> &[McpServer] {
        match scope {
            McpScope::User => &self.user,
            McpScope::Project => &self.project,
            McpScope::Local => &self.local,
        }
    }

    pub fn find(&self, scope: McpScope, name: &str) -> Option<&McpServer> {
        self.servers(scope).iter().find(|s| s.name == name)
    }

    /// Duplicate-name check against the captured scope vector.
    pub fn name_exists(&self, scope: McpScope, name: &str) -> bool {
        self.servers(scope).iter().any(|s| s.name == name)
    }

    /// Path the given scope writes to. `None` for `Project` / `Local`
    /// when there is no active project root (those need a lane).
    pub fn path_for(&self, scope: McpScope) -> Option<&Path> {
        match scope {
            McpScope::User => Some(self.claude_json_path.as_path()),
            McpScope::Local => self
                .project_root
                .as_ref()
                .map(|_| self.claude_json_path.as_path()),
            McpScope::Project => self.project_mcp_path.as_deref(),
        }
    }

    /// `(scope, name)` pairs across every scope — used for duplicate
    /// validation in the AddModal.
    pub fn all_names(&self) -> Vec<(McpScope, String)> {
        let mut out = Vec::with_capacity(self.user.len() + self.project.len() + self.local.len());
        for s in &self.user {
            out.push((McpScope::User, s.name.clone()));
        }
        for s in &self.project {
            out.push((McpScope::Project, s.name.clone()));
        }
        for s in &self.local {
            out.push((McpScope::Local, s.name.clone()));
        }
        out
    }
}

/// Validation errors for an MCP server name. The modal renders these
/// as inline banners; the rule mirrors Anthropic's recommendation
/// (alphanumeric / `_` / `-`, 1–63 chars, must start with alphanumeric).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong { max: usize, got: usize },
    InvalidChar { ch: char, position: usize },
    InvalidLeading { ch: char },
    DuplicateInScope { scope: McpScope },
}

pub const MAX_NAME_LEN: usize = 63;

/// Validate a server name against the spec recommendation. Duplicate
/// detection is layered separately via [`McpState::name_exists`] so
/// this stays purely syntactic.
pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong {
            max: MAX_NAME_LEN,
            got: name.len(),
        });
    }
    for (i, ch) in name.chars().enumerate() {
        let ok = ch.is_ascii_alphanumeric() || ch == '-' || ch == '_';
        if !ok {
            return Err(NameError::InvalidChar { ch, position: i });
        }
        if i == 0 && (ch == '-' || ch == '_') {
            return Err(NameError::InvalidLeading { ch });
        }
    }
    Ok(())
}

/// Validation errors for transport-specific fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldError {
    /// `command` empty for stdio transport.
    CommandRequired,
    /// `url` empty for sse / http transport.
    UrlRequired,
    /// URL didn't start with `http://` / `https://`.
    UrlInvalidScheme,
    /// Env line not in `KEY=VALUE` form.
    EnvInvalidLine { line: String },
}

pub fn validate_command(s: &str, transport: McpTransport) -> Result<(), FieldError> {
    if transport == McpTransport::Stdio && s.trim().is_empty() {
        return Err(FieldError::CommandRequired);
    }
    Ok(())
}

pub fn validate_url(s: &str, transport: McpTransport) -> Result<(), FieldError> {
    if !transport.is_remote() {
        return Ok(());
    }
    let s = s.trim();
    if s.is_empty() {
        return Err(FieldError::UrlRequired);
    }
    if !(s.starts_with("http://") || s.starts_with("https://")) {
        return Err(FieldError::UrlInvalidScheme);
    }
    Ok(())
}

/// Parse `KEY=VALUE` lines (one per line) into a sorted map. Empty
/// lines and lines starting with `#` are treated as comments. Both
/// the key and the value are trimmed of surrounding whitespace —
/// `TOKEN=  abc  ` lands in the map as `("TOKEN", "abc")`. Embedded
/// whitespace (`KEY=foo bar`) is preserved verbatim. An empty value
/// (`KEY=`) is allowed and stored as the empty string — Claude Code
/// writes a literal empty env var in this case.
pub fn parse_env_lines(text: &str) -> Result<BTreeMap<String, String>, FieldError> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| FieldError::EnvInvalidLine {
                line: line.to_string(),
            })?;
        let k = k.trim();
        if k.is_empty() {
            return Err(FieldError::EnvInvalidLine {
                line: line.to_string(),
            });
        }
        map.insert(k.to_string(), v.trim().to_string());
    }
    Ok(map)
}

/// Render an env map back to the textarea form used by the modal.
pub fn format_env_lines(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}
