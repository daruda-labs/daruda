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
    Some(dirs::home_dir()?.join(".claude").join("sessions"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write_meta(dir: &Path, pid: u32, json_obj: serde_json::Value) {
        let path = dir.join(format!("{pid}.json"));
        std::fs::write(&path, json_obj.to_string()).unwrap();
    }

    #[test]
    fn parses_minimal_three_field_meta() {
        let dir = TempDir::new().unwrap();
        write_meta(
            dir.path(),
            1234,
            json!({
                "pid": 1234,
                "sessionId": "abc-123",
                "cwd": "/tmp/proj"
            }),
        );

        let meta = read_session_meta_in(dir.path(), 1234).unwrap();
        assert_eq!(meta.pid, 1234);
        assert_eq!(meta.session_id, "abc-123");
        assert_eq!(meta.cwd, PathBuf::from("/tmp/proj"));
    }

    #[test]
    fn parses_full_real_world_meta() {
        // Extra keys present in real session files; daruda only consumes 3.
        let dir = TempDir::new().unwrap();
        write_meta(
            dir.path(),
            31134,
            json!({
                "pid": 31134,
                "sessionId": "fd56d7e4-53b4-46d6-b065-03ae50709aab",
                "cwd": "/home/user/project",
                "startedAt": 1777600781920_i64,
                "procStart": "Fri May  1 01:59:40 2026",
                "version": "2.1.123",
                "peerProtocol": 1,
                "kind": "interactive",
                "entrypoint": "cli",
                "status": "active",
                "updatedAt": 1777600782000_i64,
                "waitingFor": null,
            }),
        );

        let meta = read_session_meta_in(dir.path(), 31134).unwrap();
        assert_eq!(meta.session_id, "fd56d7e4-53b4-46d6-b065-03ae50709aab");
    }

    #[test]
    fn missing_file_yields_none() {
        let dir = TempDir::new().unwrap();
        assert!(read_session_meta_in(dir.path(), 99999).is_none());
    }

    #[test]
    fn malformed_json_yields_none() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("777.json"), b"{ not valid").unwrap();
        assert!(read_session_meta_in(dir.path(), 777).is_none());
    }

    #[test]
    fn missing_required_field_yields_none() {
        let dir = TempDir::new().unwrap();
        // No `sessionId` — required by our subset.
        write_meta(
            dir.path(),
            42,
            json!({
                "pid": 42,
                "cwd": "/tmp"
            }),
        );
        assert!(read_session_meta_in(dir.path(), 42).is_none());
    }

    #[test]
    fn default_dir_resolves_under_home() {
        // We don't assert the exact home, only the suffix shape.
        let dir = default_sessions_dir().expect("home dir should resolve");
        let s = dir.to_string_lossy();
        assert!(s.ends_with(".claude/sessions") || s.ends_with(".claude\\sessions"));
    }
}
