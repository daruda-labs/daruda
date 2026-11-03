//! In-memory mirror of `~/.daruda/status/*.json`.
//!
//! GPUI-free. The watcher (`app/src/hooks/watcher.rs`) feeds events
//! in; renderers (left dock) and consumers query by cwd or session id.
//!
use std::collections::HashMap;
use std::path::Path;

use crate::SessionStatus;
use crate::hooks::status_file::StatusFile;

/// Identity of a status snapshot. Legacy session filenames and terminal IDs
/// occupy separate namespaces even when their text happens to match.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StatusKey {
    Terminal(crate::hooks::route::TerminalId),
    LegacySession(String),
}

impl StatusKey {
    pub fn for_session(terminal: Option<&crate::hooks::route::TerminalId>, session: &str) -> Self {
        match terminal {
            Some(terminal) => Self::Terminal(terminal.clone()),
            None => Self::LegacySession(session.to_owned()),
        }
    }

    pub fn storage_key(&self) -> &str {
        match self {
            Self::Terminal(terminal) => terminal.as_str(),
            Self::LegacySession(session) => session,
        }
    }
}

#[derive(Default)]
pub struct ClaudeStatusStore {
    by_key: HashMap<StatusKey, StatusFile>,
}

impl ClaudeStatusStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bulk-load entries (cold restore).
    pub fn load_initial(&mut self, files: Vec<StatusFile>) {
        for f in files {
            self.update(f);
        }
    }

    /// Insert or update an entry, rejecting older event snapshots.
    /// Returns `true` if the store was modified.
    pub fn update(&mut self, incoming: StatusFile) -> bool {
        let key = incoming.identity();
        if self
            .by_key
            .get(&key)
            .is_some_and(|old| old.timestamp > incoming.timestamp)
        {
            return false;
        }
        self.by_key.insert(key, incoming);
        true
    }

    /// Exact identity lookup; a terminal key never falls back to a session ID.
    pub fn get(&self, key: &StatusKey) -> Option<&StatusFile> {
        self.by_key.get(key)
    }

    pub fn remove(&mut self, key: &StatusKey) -> Option<StatusFile> {
        self.by_key.remove(key)
    }

    pub fn get_terminal(&self, terminal: &crate::hooks::route::TerminalId) -> Option<&StatusFile> {
        self.get(&StatusKey::Terminal(terminal.clone()))
    }

    /// All physical observations of one logical conversation.
    pub fn sessions_for_session<'a>(
        &'a self,
        session: &'a str,
    ) -> impl Iterator<Item = &'a StatusFile> + 'a {
        self.by_key
            .values()
            .filter(move |file| file.session_id == session)
    }

    /// A legacy consumer may require one snapshot, but cannot choose among
    /// multiple terminals resuming the same conversation.
    pub fn unique_session(&self, session: &str) -> Option<&StatusFile> {
        let mut matches = self
            .by_key
            .values()
            .filter(|file| file.session_id == session);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub fn remove_unique_session(&mut self, session: &str) -> Option<StatusFile> {
        let key = self.unique_session(session)?.identity();
        self.remove(&key)
    }

    /// Resolve a filename only at the filesystem event boundary.
    pub fn file_for_storage_key(&self, key: &str) -> Option<&StatusFile> {
        let mut matches = self
            .by_key
            .values()
            .filter(|file| file.storage_key() == key);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub fn for_binding(&self, binding: &crate::pty_tracker::PtyBinding) -> Option<&StatusFile> {
        match binding.terminal_id.as_ref() {
            Some(terminal) => self.get_terminal(terminal),
            None => self.unique_session(&binding.session_id),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&StatusKey, &StatusFile)> {
        self.by_key.iter()
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Iterator over sessions whose `cwd` exactly equals `target`.
    pub fn sessions_for_cwd<'a>(
        &'a self,
        target: &'a Path,
    ) -> impl Iterator<Item = &'a StatusFile> + 'a {
        self.by_key.values().filter(move |s| s.cwd == target)
    }

    /// Aggregate session status for `cwd` — the highest-priority
    /// state among all sessions there. `None` if no session present.
    pub fn aggregate_for_cwd(&self, target: &Path) -> Option<SessionStatus> {
        SessionStatus::aggregate(self.sessions_for_cwd(target).map(|s| s.status))
    }
}
