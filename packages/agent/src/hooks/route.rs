//! The route from an agent hook back to the daruda process that owns the
//! terminal the agent runs in.
//!
//! Every terminal PTY carries [`terminal_env`]. A hook command reads it, so
//! an event names its terminal instead of being matched to one by process
//! ancestry, runs the build that spawned the terminal, and writes into that
//! build's profile. Outside a daruda terminal the route is absent and the
//! hook does nothing.

use std::path::{Path, PathBuf};

use daruda_core::process_env;
use serde::{Deserialize, Serialize};

/// Longest accepted terminal id; a minted id is a 32-digit hex string.
const TERMINAL_ID_MAX_LEN: usize = 64;

/// Opaque identity of one terminal PTY. Minted per spawn, so a shell
/// restarted in the same pane gets a new id and late events from the old
/// one name a terminal no pane owns.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TerminalId(String);

impl TerminalId {
    /// Accept 1–64 ASCII alphanumerics or `-`: everything a minted id can
    /// be, and nothing that could escape a file name or a log line.
    pub fn parse(value: &str) -> Option<Self> {
        let valid = !value.is_empty()
            && value.len() <= TERMINAL_ID_MAX_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        valid.then(|| Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TerminalId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or(value)
    }
}

impl From<TerminalId> for String {
    fn from(id: TerminalId) -> Self {
        id.0
    }
}

impl std::fmt::Display for TerminalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The agent CLI whose hook fired. Selects how the payload's process is
/// found and which harness-specific transitions apply.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
}

impl Harness {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];

    /// The `--harness` argument value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|h| h.as_str() == value)
    }
}

/// A terminal's route, as the hook handler reads it from its environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookRoute {
    pub terminal: TerminalId,
    pub status_dir: PathBuf,
}

impl HookRoute {
    /// The route of the terminal this process runs in; `None` outside a
    /// daruda terminal or when either value is unusable.
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            process_env::TERMINAL_ID.read_utf8().ok().as_deref(),
            process_env::HOOK_STATUS_DIR.read_os().map(PathBuf::from),
        )
    }

    fn from_values(terminal: Option<&str>, status_dir: Option<PathBuf>) -> Option<Self> {
        let terminal = TerminalId::parse(terminal?)?;
        let status_dir = status_dir.filter(|dir| dir.is_absolute())?;
        Some(Self {
            terminal,
            status_dir,
        })
    }
}

/// Environment a terminal PTY carries. `None` when a path is not UTF-8,
/// which a PTY environment cannot hold; that terminal reports no status.
pub fn terminal_env(
    terminal: &TerminalId,
    hook_bin: &Path,
    status_dir: &Path,
) -> Option<Vec<(String, String)>> {
    Some(vec![
        (
            process_env::TERMINAL_ID.name().to_owned(),
            terminal.as_str().to_owned(),
        ),
        (
            process_env::HOOK_BIN.name().to_owned(),
            bash_path(hook_bin.to_str()?),
        ),
        (
            process_env::HOOK_STATUS_DIR.name().to_owned(),
            status_dir.to_str()?.to_owned(),
        ),
    ])
}

/// Hook commands run in Bash (Git Bash on Windows), which takes
/// forward-slash paths on every host.
fn bash_path(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_ids_reject_anything_beyond_a_minted_shape() {
        assert!(TerminalId::parse("0123abcd-EF").is_some());
        assert!(TerminalId::parse("").is_none());
        assert!(TerminalId::parse("../status").is_none());
        assert!(TerminalId::parse("a b").is_none());
        assert!(TerminalId::parse(&"a".repeat(TERMINAL_ID_MAX_LEN + 1)).is_none());
    }

    #[test]
    fn a_route_needs_a_valid_id_and_an_absolute_directory() {
        let dir = std::env::temp_dir();
        let route = HookRoute::from_values(Some("abc"), Some(dir.clone())).unwrap();
        assert_eq!(route.terminal.as_str(), "abc");
        assert_eq!(route.status_dir, dir);
        assert!(HookRoute::from_values(None, Some(dir.clone())).is_none());
        assert!(HookRoute::from_values(Some("abc"), None).is_none());
        assert!(HookRoute::from_values(Some("a/b"), Some(dir)).is_none());
        assert!(HookRoute::from_values(Some("abc"), Some(PathBuf::from("rel"))).is_none());
    }

    #[test]
    fn harness_names_round_trip() {
        for harness in Harness::ALL {
            assert_eq!(Harness::parse(harness.as_str()), Some(harness));
        }
        assert_eq!(Harness::parse("gemini"), None);
    }

    #[test]
    fn terminal_env_names_every_route_key() {
        let id = TerminalId::parse("t1").unwrap();
        let env = terminal_env(&id, Path::new("/opt/daruda"), Path::new("/data/status")).unwrap();
        let names: Vec<_> = env.iter().map(|(k, _)| k.as_str()).collect();
        let expected: Vec<_> = process_env::TERMINAL_HOOK_ROUTE
            .iter()
            .map(|key| key.name())
            .collect();
        assert_eq!(names, expected);
    }
}
