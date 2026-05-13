//! In-memory mirror of `~/.daruda/status/*.json`.
//!
//! GPUI-free. The watcher (`app/src/hooks/watcher.rs`) feeds events
//! in; renderers (sidebar) and consumers query by cwd or session id.
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
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::SessionStatus;
use crate::hooks::status_file::{Source, StatusFile};

/// Default age past which a `NeedsAttention` indicator decays to
/// `Idle` in aggregate queries. Catches the case where Claude Code
/// fired a `Notification` for a permission/idle prompt that the user
/// dismissed in the TUI without daruda seeing the follow-up event.
pub const DEFAULT_NEEDS_ATTENTION_STALE: Duration = Duration::from_secs(60);

pub struct ClaudeStatusStore {
    by_session: HashMap<String, StatusFile>,
    /// Age past which `NeedsAttention` decays to `Idle` in aggregate
    /// queries. Configurable via
    /// `daruda_config::ClaudeStatusConfig::needs_attention_stale_secs`.
    needs_attention_stale: Duration,
}

impl Default for ClaudeStatusStore {
    fn default() -> Self {
        Self {
            by_session: HashMap::new(),
            needs_attention_stale: DEFAULT_NEEDS_ATTENTION_STALE,
        }
    }
}

impl ClaudeStatusStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the stale threshold (default
    /// [`DEFAULT_NEEDS_ATTENTION_STALE`] = 60s).
    pub fn with_needs_attention_stale(mut self, threshold: Duration) -> Self {
        self.needs_attention_stale = threshold;
        self
    }

    /// Effective state for one entry given a reference time. Demotes
    /// stale `NeedsAttention` (older than [`needs_attention_stale`])
    /// to `Idle`. Pure — for testability.
    fn effective_state(&self, file: &StatusFile, now: DateTime<Utc>) -> SessionStatus {
        if file.status == SessionStatus::NeedsAttention {
            let age = (now - file.timestamp).to_std().unwrap_or(Duration::ZERO);
            if age >= self.needs_attention_stale {
                return SessionStatus::Idle;
            }
        }
        file.status
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
    /// state among all sessions there, with stale `NeedsAttention`
    /// demoted to `Idle`. `None` if no session present.
    ///
    /// Uses `Utc::now()` as the reference time. Tests should call
    /// [`Self::aggregate_for_cwd_at`] for determinism.
    pub fn aggregate_for_cwd(&self, target: &Path) -> Option<SessionStatus> {
        self.aggregate_for_cwd_at(target, Utc::now())
    }

    /// Like [`Self::aggregate_for_cwd`] with an explicit reference time.
    pub fn aggregate_for_cwd_at(&self, target: &Path, now: DateTime<Utc>) -> Option<SessionStatus> {
        self.sessions_for_cwd(target)
            .map(|s| self.effective_state(s, now))
            .max_by_key(|s| s.priority())
    }

    /// All session statuses for `cwd`, sorted oldest-first (the
    /// natural reading order for the Phase D sub-row).
    pub fn per_session_for_cwd<'a>(&'a self, target: &'a Path) -> Vec<&'a StatusFile> {
        let mut v: Vec<&'a StatusFile> = self.sessions_for_cwd(target).collect();
        v.sort_by_key(|s| s.timestamp);
        v
    }

    /// `(session_id, effective_state)` pairs for `cwd`, sorted oldest
    /// first. `effective_state` applies the same stale-NeedsAttention
    /// demotion as [`Self::aggregate_for_cwd`] so the Phase D sub-row
    /// stays consistent with the leading aggregate indicator.
    pub fn per_session_states_for_cwd(&self, target: &Path) -> Vec<(String, SessionStatus)> {
        self.per_session_states_for_cwd_at(target, Utc::now())
    }

    /// Like [`Self::per_session_states_for_cwd`] with an explicit reference time.
    pub fn per_session_states_for_cwd_at(
        &self,
        target: &Path,
        now: DateTime<Utc>,
    ) -> Vec<(String, SessionStatus)> {
        self.per_session_for_cwd(target)
            .into_iter()
            .map(|s| (s.session_id.clone(), self.effective_state(s, now)))
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration as CDuration, TimeZone, Utc};
    use std::path::PathBuf;

    /// Fixed reference time for tests so offsets are exact; `Utc::now()`
    /// otherwise drifts microseconds between calls and breaks "same
    /// timestamp" comparisons.
    fn ref_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()
    }

    fn entry(
        session: &str,
        cwd: &str,
        status: SessionStatus,
        source: Source,
        offset_secs: i64,
    ) -> StatusFile {
        StatusFile {
            schema_version: 1,
            session_id: session.into(),
            cwd: PathBuf::from(cwd),
            transcript_path: None,
            status,
            last_event: "test".into(),
            tool_name: None,
            tool_input: None,
            permission_mode: None,
            timestamp: ref_time() + CDuration::seconds(offset_secs),
            source,
        }
    }

    #[test]
    fn upsert_inserts_and_replaces() {
        let mut s = ClaudeStatusStore::new();
        assert!(s.update(entry("a", "/x", SessionStatus::Working, Source::Hook, 0)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Working);
        assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 1)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    }

    #[test]
    fn remove_returns_old() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 0));
        let old = s.remove("a").unwrap();
        assert_eq!(old.session_id, "a");
        assert!(s.get("a").is_none());
    }

    #[test]
    fn hook_always_wins_over_existing_jsonl() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 0));
        // Hook arriving even with same timestamp wins.
        assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 0)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
        assert_eq!(s.get("a").unwrap().source, Source::Hook);
    }

    #[test]
    fn jsonl_loses_to_existing_hook_at_same_or_older_time() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/x", SessionStatus::Idle, Source::Hook, 10));
        // Older jsonl: rejected.
        assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 5)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
        // Same-timestamp jsonl: rejected (hook keeps slot).
        assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 10)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    }

    #[test]
    fn jsonl_strictly_newer_overrides_hook() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/x", SessionStatus::Working, Source::Hook, 0));
        // jsonl 30s newer → wins (e.g. user uninstalled hooks; jsonl
        // catches up).
        assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Jsonl, 30)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
        assert_eq!(s.get("a").unwrap().source, Source::Jsonl);
    }

    #[test]
    fn jsonl_vs_jsonl_takes_newest() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, 0));
        // Newer wins.
        assert!(s.update(entry("a", "/x", SessionStatus::Idle, Source::Jsonl, 1)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
        // Older rejected.
        assert!(!s.update(entry("a", "/x", SessionStatus::Working, Source::Jsonl, -1)));
        assert_eq!(s.get("a").unwrap().status, SessionStatus::Idle);
    }

    #[test]
    fn aggregate_picks_highest_priority() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("a", "/wt", SessionStatus::Idle, Source::Hook, 0));
        s.update(entry("b", "/wt", SessionStatus::Working, Source::Hook, 0));
        s.update(entry(
            "c",
            "/wt",
            SessionStatus::NeedsAttention,
            Source::Hook,
            0,
        ));
        s.update(entry(
            "d",
            "/other",
            SessionStatus::Working,
            Source::Hook,
            0,
        ));

        // Use a fixed reference time close to the entry timestamps so
        // NeedsAttention is not demoted by the stale-age check.
        let now = ref_time() + CDuration::seconds(30);
        assert_eq!(
            s.aggregate_for_cwd_at(&PathBuf::from("/wt"), now),
            Some(SessionStatus::NeedsAttention)
        );
        assert_eq!(
            s.aggregate_for_cwd_at(&PathBuf::from("/other"), now),
            Some(SessionStatus::Working)
        );
        assert!(
            s.aggregate_for_cwd_at(&PathBuf::from("/empty"), now)
                .is_none()
        );
    }

    #[test]
    fn per_session_sorted_by_timestamp() {
        let mut s = ClaudeStatusStore::new();
        s.update(entry("c", "/wt", SessionStatus::Idle, Source::Hook, 30));
        s.update(entry("a", "/wt", SessionStatus::Working, Source::Hook, 0));
        s.update(entry(
            "b",
            "/wt",
            SessionStatus::NeedsAttention,
            Source::Hook,
            10,
        ));

        let path = PathBuf::from("/wt");
        let v = s.per_session_for_cwd(&path);
        let ids: Vec<&str> = v.iter().map(|f| f.session_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn stale_needs_attention_demotes_to_idle() {
        let s = ClaudeStatusStore::new().with_needs_attention_stale(Duration::from_secs(60));
        let mut store = s;
        store.update(entry(
            "a",
            "/wt",
            SessionStatus::NeedsAttention,
            Source::Hook,
            0,
        ));
        let path = PathBuf::from("/wt");

        // Within threshold: kept.
        let now = ref_time() + CDuration::seconds(30);
        assert_eq!(
            store.aggregate_for_cwd_at(&path, now),
            Some(SessionStatus::NeedsAttention)
        );

        // Past threshold: demoted to Idle.
        let now = ref_time() + CDuration::seconds(60);
        assert_eq!(
            store.aggregate_for_cwd_at(&path, now),
            Some(SessionStatus::Idle)
        );
    }

    #[test]
    fn stale_demotion_changes_aggregate_winner() {
        let mut store =
            ClaudeStatusStore::new().with_needs_attention_stale(Duration::from_secs(60));
        // Stale NeedsAttention loses to a fresh Working.
        store.update(entry(
            "old",
            "/wt",
            SessionStatus::NeedsAttention,
            Source::Hook,
            0,
        ));
        store.update(entry(
            "new",
            "/wt",
            SessionStatus::Working,
            Source::Hook,
            80,
        ));
        let now = ref_time() + CDuration::seconds(80);
        let path = PathBuf::from("/wt");
        // Without demotion, NeedsAttention would win priority. With
        // demotion, the stale `old` entry becomes Idle and `new`
        // (Working) wins.
        assert_eq!(
            store.aggregate_for_cwd_at(&path, now),
            Some(SessionStatus::Working)
        );
    }

    #[test]
    fn working_does_not_decay() {
        let mut store =
            ClaudeStatusStore::new().with_needs_attention_stale(Duration::from_secs(60));
        store.update(entry("a", "/wt", SessionStatus::Working, Source::Hook, 0));
        let now = ref_time() + CDuration::seconds(120);
        let path = PathBuf::from("/wt");
        assert_eq!(
            store.aggregate_for_cwd_at(&path, now),
            Some(SessionStatus::Working)
        );
    }

    #[test]
    fn load_initial_seeds_store() {
        let mut s = ClaudeStatusStore::new();
        s.load_initial(vec![
            entry("a", "/x", SessionStatus::Idle, Source::Hook, 0),
            entry("b", "/x", SessionStatus::Working, Source::Hook, 0),
        ]);
        assert_eq!(s.len(), 2);
    }
}
