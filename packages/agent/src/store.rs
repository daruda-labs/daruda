//! In-memory mirror of `~/.daruda/status/*.json`.
//!
//! GPUI-free. The watcher (`app/src/hooks/watcher.rs`) feeds events
//! in; renderers (left dock) and consumers query by cwd or session id.
//!
use std::collections::HashMap;
use std::path::Path;

use crate::SessionStatus;
use crate::hooks::status_file::StatusFile;

#[derive(Default)]
pub struct ClaudeStatusStore {
    by_key: HashMap<String, StatusFile>,
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
        let key = incoming.storage_key().to_owned();
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

    /// Legacy session lookup. Routed consumers use `remove_key` so another
    /// terminal resuming the same session remains untouched.
    pub fn remove(&mut self, session_id: &str) -> Option<StatusFile> {
        if let Some(file) = self.by_key.remove(session_id) {
            return Some(file);
        }
        let key = self
            .by_key
            .iter()
            .find(|(_, file)| file.session_id == session_id)
            .map(|(key, _)| key.clone())?;
        self.by_key.remove(&key)
    }

    /// Legacy session lookup; routed panes use `for_binding` or `get_key`.
    pub fn get(&self, session_id: &str) -> Option<&StatusFile> {
        self.by_key.get(session_id).or_else(|| {
            self.by_key
                .values()
                .find(|file| file.session_id == session_id)
        })
    }

    /// Resolve a pane using its physical terminal, falling back for legacy bindings.
    pub fn for_binding(&self, binding: &crate::pty_tracker::PtyBinding) -> Option<&StatusFile> {
        match &binding.terminal_id {
            Some(terminal) => self.by_key.get(terminal.as_str()),
            None => self.get(&binding.session_id),
        }
    }

    /// Read an exact on-disk identity without session-id fallback.
    pub fn get_key(&self, key: &str) -> Option<&StatusFile> {
        self.by_key.get(key)
    }

    pub fn remove_key(&mut self, key: &str) -> Option<StatusFile> {
        self.by_key.remove(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &StatusFile)> {
        self.by_key.iter().map(|(k, v)| (k.as_str(), v))
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
