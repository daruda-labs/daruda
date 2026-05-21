//! MCP server data model — GPUI-free types describing the on-disk
//! layout of the two MCP server scopes Claude Code reads:
//!
//! - **Project** — `<lane>/.mcp.json`
//! - **Personal** — `~/.claude/settings.json` `mcpServers`
//!
//! Both files round-trip losslessly: known keys land in typed fields,
//! unknown keys are preserved via `extra` (per server) and via the raw
//! `serde_json::Value` tree (top-level non-`mcpServers` keys like
//! `permissions`, `hooks`, `enableAllProjectMcpServers`). This is
//! mandatory — the personal settings file is shared with Claude Code
//! itself, and silently dropping a `permissions.allow` entry on toggle
//! would be catastrophic.
//!
//! No GPUI imports here — `app/src/CLAUDE.md` G2 / G7 forbid them.
//! This module is consumed by the renderer (`workspace/right_panel/tools/`),
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
    PERSONAL_SETTINGS_FILE, PROJECT_MCP_FILE, parse_personal_settings, parse_project_mcp,
    personal_settings_path, project_mcp_path,
};
pub use persist::{
    McpPersistError, McpServerDraft, delete_server, set_disabled, update_server, write_server,
};

/// On-disk scope for an MCP server.
///
/// - `Project` — `<lane>/.mcp.json` (team-shared, committable; only
///   loaded by Claude Code when `enableAllProjectMcpServers=true` in
///   personal settings).
/// - `Personal` — `~/.claude/settings.json` `mcpServers` (always loaded).
///
/// Same name in both scopes → daruda surfaces `(overrides personal)` on
/// the project row, mirroring the Skills tab's project-overrides-personal
/// UX.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum McpScope {
    Project,
    Personal,
}

impl McpScope {
    /// Stable kebab slug for serialization / logging.
    pub fn slug(self) -> &'static str {
        match self {
            McpScope::Project => "project",
            McpScope::Personal => "personal",
        }
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

/// One MCP server entry from disk. Built by the parsers.
#[derive(Clone, Debug)]
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
/// bootstrap (`global::init`). User-scope vectors (`personal`,
/// `personal_raw`) are shared across every Workspace; project-scope
/// state is partitioned by lane root path so multiple Workspace
/// windows observing different lanes never collide on a single
/// `project_root` field. Renderers and modals consume a per-lane
/// [`McpSnapshot`] via [`McpState::snapshot_for`].
///
/// Mirrors Zed's `SettingsStore::local_settings` pattern (a single
/// Global with a `BTreeMap` keyed by lane-relative location).
///
/// `*_raw` carries the entire parsed JSON tree so non-`mcpServers`
/// keys (permissions, hooks, top-level project settings, etc.) survive
/// every write-back unchanged. This is mandatory — the personal
/// settings file is shared with Claude Code itself, and silently
/// dropping a `permissions.allow` entry on toggle would be catastrophic.
#[derive(Clone, Debug, Default)]
pub struct McpState {
    pub personal: Vec<McpServer>,
    /// Whole `~/.claude/settings.json` parsed tree.
    pub personal_raw: serde_json::Value,
    /// Per-lane project-scope state, keyed by the lane's
    /// absolute root path (what `Workspace::active_worktree_root`
    /// returns). An entry exists for every lane that has been
    /// scanned at least once; opening a different lane adds a new
    /// key without disturbing the others.
    pub project: BTreeMap<PathBuf, ProjectMcp>,
    /// Last successful scan timestamp across any scope. `None` until
    /// the first load lands.
    pub last_scanned: Option<SystemTime>,
}

impl McpState {
    /// Reload one scope from disk. `lane` is required for
    /// `McpScope::Project` and ignored otherwise. Project entries are
    /// inserted into the `project` map at the lane's path.
    pub fn reload_scope(
        &mut self,
        scope: McpScope,
        lane: Option<&Path>,
    ) -> Result<(), parse::ParseError> {
        match scope {
            McpScope::Project => {
                if let Some(w) = lane {
                    let path = parse::project_mcp_path(w);
                    let (servers, raw) = parse_project_mcp(&path)?;
                    self.project
                        .insert(w.to_path_buf(), ProjectMcp { servers, raw });
                }
            }
            McpScope::Personal => {
                let path = parse::personal_settings_path();
                let (servers, raw) = parse_personal_settings(&path)?;
                self.personal = servers;
                self.personal_raw = raw;
            }
        }
        self.last_scanned = Some(SystemTime::now());
        Ok(())
    }

    /// Drop a lane's project entry. Call when a lane is
    /// closed so the `BTreeMap` doesn't grow unbounded across the
    /// session.
    pub fn forget_lane(&mut self, root: &Path) {
        self.project.remove(root);
    }

    /// Build an owned per-lane view for the renderer / modals.
    /// Carrying it by value keeps the panel render closure off the
    /// Global (no re-entrancy hazard).
    pub fn snapshot_for(&self, lane: Option<&Path>) -> McpSnapshot {
        let (project, project_raw) = match lane.and_then(|w| self.project.get(w)) {
            Some(p) => (p.servers.clone(), p.raw.clone()),
            None => (Vec::new(), serde_json::Value::Object(Default::default())),
        };
        McpSnapshot {
            project,
            personal: self.personal.clone(),
            project_root: lane.map(Path::to_path_buf),
            project_mcp_path: lane.map(parse::project_mcp_path),
            personal_settings_path: parse::personal_settings_path(),
            project_raw,
            personal_raw: self.personal_raw.clone(),
            last_scanned: self.last_scanned,
        }
    }
}

/// Owned per-lane projection of [`McpState`] consumed by the
/// renderer and CRUD modals. Carries the project Vec / raw tree for
/// *one* lane along with the user-global personal vectors.
#[derive(Clone, Debug, Default)]
pub struct McpSnapshot {
    pub project: Vec<McpServer>,
    pub personal: Vec<McpServer>,
    /// Lane root whose project servers are carried in `project`.
    /// `None` when the workspace has no active lane.
    pub project_root: Option<PathBuf>,
    /// `<project_root>/.mcp.json`. `None` when `project_root` is `None`.
    pub project_mcp_path: Option<PathBuf>,
    /// Cached `~/.claude/settings.json` resolution.
    pub personal_settings_path: PathBuf,
    pub project_raw: serde_json::Value,
    pub personal_raw: serde_json::Value,
    pub last_scanned: Option<SystemTime>,
}

impl McpSnapshot {
    pub fn servers(&self, scope: McpScope) -> &[McpServer] {
        match scope {
            McpScope::Project => &self.project,
            McpScope::Personal => &self.personal,
        }
    }

    pub fn find(&self, scope: McpScope, name: &str) -> Option<&McpServer> {
        self.servers(scope).iter().find(|s| s.name == name)
    }

    /// Duplicate-name check against the captured Project / Personal
    /// vectors.
    pub fn name_exists(&self, scope: McpScope, name: &str) -> bool {
        self.servers(scope).iter().any(|s| s.name == name)
    }

    /// True when a project server has the same name as a personal
    /// server.
    pub fn project_overrides_personal(&self, project_name: &str) -> bool {
        self.personal.iter().any(|s| s.name == project_name)
    }

    /// Path the given scope writes to. `None` for `Project` when
    /// there is no active project root.
    pub fn path_for(&self, scope: McpScope) -> Option<&Path> {
        match scope {
            McpScope::Project => self.project_mcp_path.as_deref(),
            McpScope::Personal => Some(self.personal_settings_path.as_path()),
        }
    }

    /// `(scope, name)` pairs across both scopes — used for duplicate
    /// validation in the AddModal.
    pub fn all_names(&self) -> Vec<(McpScope, String)> {
        let mut out = Vec::with_capacity(self.project.len() + self.personal.len());
        for s in &self.project {
            out.push((McpScope::Project, s.name.clone()));
        }
        for s in &self.personal {
            out.push((McpScope::Personal, s.name.clone()));
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
