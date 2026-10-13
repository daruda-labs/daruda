//! In-memory mirror of `~/.daruda/status/*.json`.
//!
//! GPUI-free. The watcher (`app/src/hooks/watcher.rs`) feeds events
//! in; renderers (left dock) and consumers query by cwd or session id.
//!
//! Race policy: hook wins over jsonl when both report on the same
//! session. The jsonl path is a lag-prone fallback, so:
//!
//! - Incoming `hook` always replaces the existing entry.
//! - Incoming `jsonl` replaces only if the existing entry is also
//!   `jsonl`, or if the existing `hook` entry is older than the
//!   incoming jsonl timestamp.

use std::collections::HashMap;
use std::path::Path;

use crate::SessionStatus;
use crate::hooks::status_file::{Source, StatusFile};

#[derive(Default)]
pub struct ClaudeStatusStore {
    by_session: HashMap<String, StatusFile>,
}

impl ClaudeStatusStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bulk-load entries (cold restore).
    pub fn load_initial(&mut self, files: Vec<StatusFile>) {
        for f in files {
            self.by_session.insert(f.session_id.clone(), f);
        }
    }

    /// Insert / update one entry, applying the hook-wins race policy.
    /// Returns `true` if the store was modified.
    pub fn update(&mut self, incoming: StatusFile) -> bool {
        if let Some(existing) = self.by_session.get(&incoming.session_id)
            && !should_replace(existing, &incoming)
        {
            return false;
        }
        self.by_session
            .insert(incoming.session_id.clone(), incoming);
        true
    }

    /// Remove a session — typically on `SessionEnd`. Returns the
    /// previous entry if it was present.
    pub fn remove(&mut self, session_id: &str) -> Option<StatusFile> {
        self.by_session.remove(session_id)
    }

    pub fn get(&self, session_id: &str) -> Option<&StatusFile> {
        self.by_session.get(session_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &StatusFile)> {
        self.by_session.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.by_session.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_session.is_empty()
    }

    /// Iterator over sessions whose `cwd` exactly equals `target`.
    pub fn sessions_for_cwd<'a>(
        &'a self,
        target: &'a Path,
    ) -> impl Iterator<Item = &'a StatusFile> + 'a {
        self.by_session.values().filter(move |s| s.cwd == target)
    }

    /// Aggregate session status for `cwd` — the highest-priority
    /// state among all sessions there. `None` if no session present.
    pub fn aggregate_for_cwd(&self, target: &Path) -> Option<SessionStatus> {
        SessionStatus::aggregate(self.sessions_for_cwd(target).map(|s| s.status))
    }
}

/// Race policy core — hook wins over jsonl. Pure function, exposed so the
/// jsonl watcher can preflight without holding the store lock.
fn should_replace(existing: &StatusFile, incoming: &StatusFile) -> bool {
    match (existing.source, incoming.source) {
        // Incoming is hook: always wins.
        (_, Source::Hook) => true,
        // Both jsonl: take the newer one.
        (Source::Jsonl, Source::Jsonl) => incoming.timestamp >= existing.timestamp,
        // Existing is a hook, incoming is jsonl: jsonl only wins if
        // it's strictly newer (otherwise hook keeps the slot).
        (Source::Hook, Source::Jsonl) => incoming.timestamp > existing.timestamp,
    }
}
