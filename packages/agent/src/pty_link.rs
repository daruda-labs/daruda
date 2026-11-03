//! PID → session_id resolution via `~/.claude/sessions/<pid>.json`.
//!
//! Claude Code writes one JSON file per running interactive session,
//! keyed by process PID, containing the authoritative `sessionId`.
//! Hooks inspect their ancestry and read exactly one file to prove the
//! agent PID. Restored run claims use the same per-PID proof; no directory
//! scans or pane discovery use this module.

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

/// The sessions directory a `claude` running with `config_dir` as its
/// config home writes into — `CLAUDE_CONFIG_DIR` for a managed account.
pub fn sessions_dir_in(config_dir: &std::path::Path) -> PathBuf {
    config_dir.join("sessions")
}

/// Read a single PID's session identity in the supplied account directory.
/// Missing or malformed files are inconclusive to the hook's ancestry search.
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
