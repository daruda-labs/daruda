//! Cache of the option vocabularies an ACP agent actually advertised.
//!
//! Modes and models differ per agent and — for models — per account and
//! plan, so neither list can be known at build time. Every connect
//! re-records what the adapter advertised, so the option lists a picker
//! offers are always what a live agent accepted. Entries are keyed by both
//! stable agent id and adapter command so old and new connections can coexist
//! while a catalog row's command is being replaced.
//!
//! Storage layout:
//! ```text
//! ~/.config/daruda/
//! └── agent_vocabulary.json   # { version, agents: { <agent_id>: { sources: { <command>: ... } } } }
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::observability::system_info::redact_home;
use crate::persistence::{LoadOutcome, load_json_file, save_json_atomic};

/// On-disk schema version of `agent_vocabulary.json`.
pub const SCHEMA_VERSION: u32 = 2;

/// One advertised choice on either axis — the id submitted back to the
/// agent (`set_mode` / `set_config_option`) plus its display label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabEntry {
    pub id: String,
    pub name: String,
    /// The adapter's secondary text; for Claude's `default` model it names the
    /// model that default resolves to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl VocabEntry {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description;
        self
    }
}

/// What one source advertised, per axis. `None` means that axis has not been
/// seen yet; `Some([])` means the agent advertised the axis and offered no
/// choices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SourceVocabulary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modes: Option<Vec<VocabEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    models: Option<Vec<VocabEntry>>,
}

impl SourceVocabulary {
    fn axis(&self, axis: Axis) -> Option<&[VocabEntry]> {
        match axis {
            Axis::Modes => self.modes.as_deref(),
            Axis::Models => self.models.as_deref(),
        }
    }

    fn axis_mut(&mut self, axis: Axis) -> &mut Option<Vec<VocabEntry>> {
        match axis {
            Axis::Modes => &mut self.modes,
            Axis::Models => &mut self.models,
        }
    }
}

/// All adapter commands observed for one stable agent id.
///
/// The legacy fields deserialize schema v1 and are removed when the cache is
/// migrated. They are never emitted in schema v2.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AgentVocabulary {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    sources: BTreeMap<String, SourceVocabulary>,
    #[serde(default, skip_serializing)]
    source: Option<String>,
    #[serde(default, skip_serializing)]
    modes: Option<Vec<VocabEntry>>,
    #[serde(default, skip_serializing)]
    models: Option<Vec<VocabEntry>>,
}

impl AgentVocabulary {
    fn migrate_v1(&mut self) {
        let Some(source) = self.source.take() else {
            self.modes = None;
            self.models = None;
            return;
        };
        self.sources.insert(
            source.trim().to_string(),
            SourceVocabulary {
                modes: self.modes.take(),
                models: self.models.take(),
            },
        );
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
        let mut cache: Self = match load_json_file::<Self>("agent_vocabulary", &path) {
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
        if cache.version < SCHEMA_VERSION {
            for vocabulary in cache.agents.values_mut() {
                vocabulary.migrate_v1();
            }
            cache.version = SCHEMA_VERSION;
            if let Err(error) = cache.save_in(data_dir) {
                LogWriter::log(
                    ErrorReport::new("Failed to save migrated agent_vocabulary.json")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&error)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&path))
                        .dedup("agent_vocabulary.migrate.save")
                        .build(),
                );
                return Self::default();
            }
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
    pub fn record_modes(&mut self, agent_id: &str, source: &str, entries: Vec<VocabEntry>) -> bool {
        self.record_axis(agent_id, source, Axis::Modes, entries)
    }

    /// Replace `agent_id`'s model vocabulary with what was advertised.
    /// `true` when the stored list actually changed.
    pub fn record_models(
        &mut self,
        agent_id: &str,
        source: &str,
        entries: Vec<VocabEntry>,
    ) -> bool {
        self.record_axis(agent_id, source, Axis::Models, entries)
    }

    fn record_axis(
        &mut self,
        agent_id: &str,
        source: &str,
        axis: Axis,
        entries: Vec<VocabEntry>,
    ) -> bool {
        let source = source.trim();
        let vocabulary = self
            .agents
            .entry(agent_id.to_string())
            .or_default()
            .sources
            .entry(source.to_string())
            .or_default();
        if vocabulary.axis(axis) == Some(entries.as_slice()) {
            return false;
        }
        *vocabulary.axis_mut(axis) = Some(entries);
        true
    }

    /// Modes advertised by this stable agent id and adapter command.
    pub fn known_modes_for(&self, agent_id: &str, source: &str) -> Option<&[VocabEntry]> {
        self.source_vocabulary(agent_id, source)
            .and_then(|vocabulary| vocabulary.modes.as_deref())
    }

    /// Models advertised by this stable agent id and adapter command.
    pub fn known_models_for(&self, agent_id: &str, source: &str) -> Option<&[VocabEntry]> {
        self.source_vocabulary(agent_id, source)
            .and_then(|vocabulary| vocabulary.models.as_deref())
    }

    fn source_vocabulary(&self, agent_id: &str, source: &str) -> Option<&SourceVocabulary> {
        self.agents
            .get(agent_id)
            .and_then(|vocabulary| vocabulary.sources.get(source.trim()))
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
