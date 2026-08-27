//! Cache of the option vocabularies an ACP agent actually advertised.
//!
//! Modes and models differ per agent and — for models — per account and
//! plan, so neither list can be known at build time. Every connect
//! re-records what the adapter advertised, so the option lists a picker
//! offers are always what the live agent last accepted.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! └── agent_vocabulary.json   # { version, agents: { <agent_id>: { modes, models } } }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;
use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

/// On-disk schema version of `agent_vocabulary.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// One advertised choice on either axis — the id submitted back to the
/// agent (`set_mode` / `set_config_option`) plus its display label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabEntry {
    pub id: String,
    pub name: String,
}

impl VocabEntry {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// What one agent advertised, per axis. An axis is empty when the agent
/// advertised nothing on it (or has not been connected to yet).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentVocabulary {
    #[serde(default)]
    pub modes: Vec<VocabEntry>,
    #[serde(default)]
    pub models: Vec<VocabEntry>,
}

impl AgentVocabulary {
    fn axis(&self, axis: Axis) -> &[VocabEntry] {
        match axis {
            Axis::Modes => &self.modes,
            Axis::Models => &self.models,
        }
    }

    fn axis_mut(&mut self, axis: Axis) -> &mut Vec<VocabEntry> {
        match axis {
            Axis::Modes => &mut self.modes,
            Axis::Models => &mut self.models,
        }
    }
}

/// Which vocabulary a `record_*` call replaces. Private: callers pick the
/// axis by choosing the method, so no invalid axis value can be passed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Modes,
    Models,
}

/// Every agent's advertised vocabulary, keyed by config `agent_id`.
/// `BTreeMap` so the persisted JSON has a stable key order and a
/// re-record that changed nothing produces a byte-identical file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentVocabularyCache {
    version: u32,
    #[serde(default)]
    agents: BTreeMap<String, AgentVocabulary>,
}

impl Default for AgentVocabularyCache {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            agents: BTreeMap::new(),
        }
    }
}

/// `agent_vocabulary.json` path under `data_dir`.
pub fn agent_vocabulary_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("agent_vocabulary.json")
}

impl AgentVocabularyCache {
    /// Load the cache from `data_dir`. A missing, unreadable, corrupt, or
    /// too-new file yields an empty cache — the next connect re-records
    /// the real vocabulary, so there is nothing here worth failing over.
    pub fn load_in(data_dir: &Path) -> Self {
        let path = agent_vocabulary_path_in(data_dir);
        let cache: Self = match load_json_file::<Self>("agent_vocabulary", &path) {
            LoadOutcome::Parsed(c) => c,
            LoadOutcome::Missing => return Self::default(),
            LoadOutcome::Corrupt => {
                Self::log_reset(&path, "file unreadable or invalid JSON");
                return Self::default();
            }
        };
        if cache.version > SCHEMA_VERSION {
            Self::log_reset(
                &path,
                format!("version {} > supported {SCHEMA_VERSION}", cache.version),
            );
            return Self::default();
        }
        cache
    }

    /// A discarded on-disk cache is routine recovery, not a failure the
    /// user can act on — Info, and the next connect refills it.
    fn log_reset(path: &Path, reason: impl Into<String>) {
        LogWriter::log(
            ErrorReport::new("agent_vocabulary.json discarded — starting from an empty cache")
                .severity(ErrorSeverity::Info)
                .message(reason)
                .at(file!(), line!())
                .with_context("path", redact_home(path))
                .dedup("agent_vocabulary.load.reset")
                .build(),
        );
    }

    /// Replace `agent_id`'s mode vocabulary with what was advertised.
    /// `true` when the stored list actually changed.
    pub fn record_modes(&mut self, agent_id: &str, entries: Vec<VocabEntry>) -> bool {
        self.record_axis(agent_id, Axis::Modes, entries)
    }

    /// Replace `agent_id`'s model vocabulary with what was advertised.
    /// `true` when the stored list actually changed.
    pub fn record_models(&mut self, agent_id: &str, entries: Vec<VocabEntry>) -> bool {
        self.record_axis(agent_id, Axis::Models, entries)
    }

    fn record_axis(&mut self, agent_id: &str, axis: Axis, entries: Vec<VocabEntry>) -> bool {
        let current = self
            .agents
            .get(agent_id)
            .map(|v| v.axis(axis))
            .unwrap_or_default();
        if current == entries.as_slice() {
            return false;
        }
        *self
            .agents
            .entry(agent_id.to_string())
            .or_default()
            .axis_mut(axis) = entries;
        true
    }

    /// What `agent_id` last advertised, or `None` for an agent that has
    /// never connected.
    pub fn for_agent(&self, agent_id: &str) -> Option<&AgentVocabulary> {
        self.agents.get(agent_id)
    }

    /// `agent_id`'s last-advertised modes; empty when unknown.
    pub fn modes(&self, agent_id: &str) -> &[VocabEntry] {
        self.for_agent(agent_id)
            .map(|v| v.modes.as_slice())
            .unwrap_or_default()
    }

    /// `agent_id`'s last-advertised models; empty when unknown.
    pub fn models(&self, agent_id: &str) -> &[VocabEntry] {
        self.for_agent(agent_id)
            .map(|v| v.models.as_slice())
            .unwrap_or_default()
    }

    /// Save atomically — same-FS tempfile + rename.
    pub fn save_in(&self, data_dir: &Path) -> std::io::Result<()> {
        save_json_atomic(data_dir, &agent_vocabulary_path_in(data_dir), self)
    }

    /// Production convenience — load from the default data dir, for a caller
    /// (Settings) that holds no `data_dir` of its own.
    pub fn load() -> Self {
        Self::load_in(&crate::persistence::default_data_dir())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(pairs: &[(&str, &str)]) -> Vec<VocabEntry> {
        pairs
            .iter()
            .map(|(id, name)| VocabEntry::new(*id, *name))
            .collect()
    }

    #[test]
    fn save_then_load_roundtrips_both_axes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache = AgentVocabularyCache::default();
        assert!(cache.record_modes("claude", entries(&[("default", "Default")])));
        assert!(cache.record_models("claude", entries(&[("opus", "Opus")])));
        cache.save_in(dir.path()).unwrap();

        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(loaded, cache);
        assert_eq!(loaded.modes("claude"), entries(&[("default", "Default")]));
        assert_eq!(loaded.models("claude"), entries(&[("opus", "Opus")]));
    }

    #[test]
    fn missing_file_loads_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(loaded, AgentVocabularyCache::default());
        assert!(loaded.for_agent("claude").is_none());
    }

    #[test]
    fn corrupt_json_loads_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(agent_vocabulary_path_in(dir.path()), b"{ not json").unwrap();
        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(loaded, AgentVocabularyCache::default());
    }

    #[test]
    fn newer_schema_version_loads_empty_cache() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            agent_vocabulary_path_in(dir.path()),
            format!(
                r#"{{"version":{},"agents":{{"claude":{{"modes":[],"models":[]}}}}}}"#,
                SCHEMA_VERSION + 1
            ),
        )
        .unwrap();
        let loaded = AgentVocabularyCache::load_in(dir.path());
        assert_eq!(loaded, AgentVocabularyCache::default());
    }

    #[test]
    fn recording_one_axis_leaves_the_other_intact() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_modes("claude", entries(&[("plan", "Plan")]));
        cache.record_models("claude", entries(&[("opus", "Opus")]));

        assert!(cache.record_models("claude", entries(&[("sonnet", "Sonnet")])));
        assert_eq!(cache.modes("claude"), entries(&[("plan", "Plan")]));
        assert_eq!(cache.models("claude"), entries(&[("sonnet", "Sonnet")]));
    }

    #[test]
    fn recording_one_agent_leaves_other_agents_intact() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_models("claude", entries(&[("opus", "Opus")]));
        cache.record_models("codex", entries(&[("gpt", "GPT")]));

        assert!(cache.record_models("codex", entries(&[("gpt-2", "GPT 2")])));
        assert_eq!(cache.models("claude"), entries(&[("opus", "Opus")]));
        assert_eq!(cache.models("codex"), entries(&[("gpt-2", "GPT 2")]));
    }

    #[test]
    fn re_recording_the_same_list_reports_no_change() {
        let mut cache = AgentVocabularyCache::default();
        let advertised = entries(&[("opus", "Opus"), ("sonnet", "Sonnet")]);
        assert!(cache.record_models("claude", advertised.clone()));
        assert!(
            !cache.record_models("claude", advertised),
            "an identical re-advertisement must not request a rewrite"
        );
    }

    #[test]
    fn empty_advertisement_for_unknown_agent_reports_no_change() {
        let mut cache = AgentVocabularyCache::default();
        assert!(!cache.record_modes("claude", Vec::new()));
        assert!(!cache.record_models("claude", Vec::new()));
        assert!(
            cache.for_agent("claude").is_none(),
            "an agent that advertised nothing must not gain a record"
        );
    }

    #[test]
    fn losing_an_axis_is_a_change_and_clears_it() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_models("claude", entries(&[("opus", "Opus")]));
        assert!(cache.record_models("claude", Vec::new()));
        assert!(cache.models("claude").is_empty());
    }

    #[test]
    fn reordered_advertisement_is_a_change() {
        let mut cache = AgentVocabularyCache::default();
        cache.record_models("claude", entries(&[("a", "A"), ("b", "B")]));
        assert!(
            cache.record_models("claude", entries(&[("b", "B"), ("a", "A")])),
            "order is the agent's own presentation order — preserve it"
        );
    }
}
