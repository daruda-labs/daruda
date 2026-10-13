//! PID → session_id resolution via `~/.claude/sessions/<pid>.json`.
//!
//! Claude Code writes one JSON file per running interactive session,
//! keyed by process PID, containing the authoritative `sessionId`.
//! daruda's PTY tracker (`app/src/hooks/pty_tracker.rs`) walks
//! descendants of each pane's PTY, finds the `claude` process, and
//! looks up its session_id here so the left dock can:
//!
//! - Highlight the session matching the focused tab (active badge)
//! - Drop a session from the in-memory store as soon as its `claude`
//!   process disappears (no waiting for `SessionEnd` or TTL)
//!
//! This module is GPUI-free and IO-only — pure file read + serde.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Subset of `~/.claude/sessions/<pid>.json` daruda cares about.
/// Forward-compat: extra keys (`version`, `kind`, `entrypoint`,
/// `peerProtocol`, `startedAt`, `updatedAt`, `status`, `waitingFor`,
/// `procStart`) are ignored.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PidSessionMeta {
    pub pid: u32,
    pub session_id: String,
    /// Directory the `claude` process was launched from. Useful as a
    /// sanity check against the `cwd` recorded in our hook payloads.
    pub cwd: PathBuf,
}

/// Resolve `~/.claude/sessions/`. Returns `None` if `dirs::home_dir`
/// fails (extremely rare on a real macOS install).
pub fn default_sessions_dir() -> Option<PathBuf> {
    Some(sessions_dir_in(&dirs::home_dir()?.join(".claude")))
}

/// The sessions directory a `claude` running with `config_dir` as its
/// config home writes into — `CLAUDE_CONFIG_DIR` for a managed account.
pub fn sessions_dir_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("sessions")
}

/// Read `~/.claude/sessions/<pid>.json` for the given PID. Missing or
/// malformed files yield `None` so the caller can treat both as
/// "this PID is not currently a registered Claude session" without
/// special-casing IO errors.
pub fn read_session_meta(pid: u32) -> Option<PidSessionMeta> {
    read_session_meta_in(&default_sessions_dir()?, pid)
}

/// Like [`read_session_meta`] with an explicit sessions directory —
/// used by tests to point at a tempdir without monkey-patching `$HOME`.
pub fn read_session_meta_in(dir: &Path, pid: u32) -> Option<PidSessionMeta> {
    let path = dir.join(format!("{pid}.json"));
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Whether `pid` is still the process writing `session_id`. `claude` keeps
/// `<dir>/<pid>.json` for its whole life, so a missing file or one naming
/// another session means that process is gone — the PID reused, or the CLI
/// moved on to a new session. A file present but unreadable (mid-write)
/// proves nothing and keeps the claim.
pub fn pid_holds_session(dir: &Path, pid: u32, session_id: &str) -> bool {
    match fs::read(dir.join(format!("{pid}.json"))) {
        Ok(bytes) => serde_json::from_slice::<PidSessionMeta>(&bytes)
            .map_or(true, |meta| meta.session_id == session_id),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}
