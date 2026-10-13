//! Per-agent mode / model vocabulary persistence.

use daruda_store::agent_vocabulary::*;

const CLAUDE: &str = "npx -y @agentclientprotocol/claude-agent-acp@latest";
const CODEX: &str = "npx -y @agentclientprotocol/codex-acp@latest";

fn entries(pairs: &[(&str, &str)]) -> Vec<VocabEntry> {
    pairs
        .iter()
        .map(|(id, name)| VocabEntry::new(*id, *name))
        .collect()
}

/// A cache written before entries carried a description still loads, and
/// an entry without one writes no `description` key.
#[test]
fn description_is_optional_on_disk() {
    let old: VocabEntry = serde_json::from_str(r#"{"id":"opus","name":"Opus"}"#).unwrap();
    assert_eq!(old, VocabEntry::new("opus", "Opus"));
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"id":"opus","name":"Opus"}"#
    );

    let described =
        VocabEntry::new("default", "Default").with_description(Some("Opus 5.5".to_string()));
    let json = serde_json::to_string(&described).unwrap();
    assert_eq!(
        serde_json::from_str::<VocabEntry>(&json).unwrap(),
        described
    );
}

#[test]
fn save_then_load_roundtrips_both_axes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = AgentVocabularyCache::default();
    assert!(cache.record_modes("claude", CLAUDE, entries(&[("default", "Default")])));
    assert!(cache.record_models("claude", CLAUDE, entries(&[("opus", "Opus")])));
    cache.save_in(dir.path()).unwrap();

    let loaded = AgentVocabularyCache::load_in(dir.path());
    assert_eq!(loaded, cache);
    assert_eq!(
        loaded.known_modes_for("claude", CLAUDE),
        Some(entries(&[("default", "Default")]).as_slice())
    );
    assert_eq!(
        loaded.known_models_for("claude", CLAUDE),
        Some(entries(&[("opus", "Opus")]).as_slice())
    );
}

#[test]
fn missing_file_loads_empty_cache() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = AgentVocabularyCache::load_in(dir.path());
    assert_eq!(loaded, AgentVocabularyCache::default());
    assert!(loaded.known_models_for("claude", CLAUDE).is_none());
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
    cache.record_modes("claude", CLAUDE, entries(&[("plan", "Plan")]));
    cache.record_models("claude", CLAUDE, entries(&[("opus", "Opus")]));

    assert!(cache.record_models("claude", CLAUDE, entries(&[("sonnet", "Sonnet")])));
    assert_eq!(
        cache.known_modes_for("claude", CLAUDE),
        Some(entries(&[("plan", "Plan")]).as_slice())
    );
    assert_eq!(
        cache.known_models_for("claude", CLAUDE),
        Some(entries(&[("sonnet", "Sonnet")]).as_slice())
    );
}

#[test]
fn recording_one_agent_leaves_other_agents_intact() {
    let mut cache = AgentVocabularyCache::default();
    cache.record_models("claude", CLAUDE, entries(&[("opus", "Opus")]));
    cache.record_models("codex", CODEX, entries(&[("gpt", "GPT")]));

    assert!(cache.record_models("codex", CODEX, entries(&[("gpt-2", "GPT 2")])));
    assert_eq!(
        cache.known_models_for("claude", CLAUDE),
        Some(entries(&[("opus", "Opus")]).as_slice())
    );
    assert_eq!(
        cache.known_models_for("codex", CODEX),
        Some(entries(&[("gpt-2", "GPT 2")]).as_slice())
    );
}

#[test]
fn re_recording_the_same_list_reports_no_change() {
    let mut cache = AgentVocabularyCache::default();
    let advertised = entries(&[("opus", "Opus"), ("sonnet", "Sonnet")]);
    assert!(cache.record_models("claude", CLAUDE, advertised.clone()));
    assert!(
        !cache.record_models("claude", CLAUDE, advertised),
        "an identical re-advertisement must not request a rewrite"
    );
}

#[test]
fn empty_advertisement_for_unknown_agent_records_known_empty_axes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cache = AgentVocabularyCache::default();
    assert!(cache.record_modes("claude", CLAUDE, Vec::new()));
    assert!(cache.record_models("claude", CLAUDE, Vec::new()));
    assert_eq!(cache.known_modes_for("claude", CLAUDE), Some(&[][..]));
    assert_eq!(cache.known_models_for("claude", CLAUDE), Some(&[][..]));
    assert!(
        !cache.record_models("claude", CLAUDE, Vec::new()),
        "re-recording known empty must not request another rewrite"
    );
    cache.save_in(dir.path()).unwrap();
    let loaded = AgentVocabularyCache::load_in(dir.path());
    assert_eq!(loaded.known_modes_for("claude", CLAUDE), Some(&[][..]));
    assert_eq!(loaded.known_models_for("claude", CLAUDE), Some(&[][..]));
}

#[test]
fn losing_an_axis_is_a_change_and_clears_it() {
    let mut cache = AgentVocabularyCache::default();
    cache.record_models("claude", CLAUDE, entries(&[("opus", "Opus")]));
    assert!(cache.record_models("claude", CLAUDE, Vec::new()));
    assert_eq!(cache.known_models_for("claude", CLAUDE), Some(&[][..]));
}

#[test]
fn reordered_advertisement_is_a_change() {
    let mut cache = AgentVocabularyCache::default();
    cache.record_models("claude", CLAUDE, entries(&[("a", "A"), ("b", "B")]));
    assert!(
        cache.record_models("claude", CLAUDE, entries(&[("b", "B"), ("a", "A")])),
        "order is the agent's own presentation order — preserve it"
    );
}

#[test]
fn advertisements_from_two_sources_for_one_agent_are_kept_separately() {
    let mut cache = AgentVocabularyCache::default();
    cache.record_modes("assistant", CLAUDE, entries(&[("plan", "Plan")]));
    cache.record_models("assistant", CLAUDE, entries(&[("opus", "Opus")]));

    assert!(cache.record_modes("assistant", CODEX, entries(&[("agent", "Agent")]),));
    assert_eq!(
        cache.known_modes_for("assistant", CLAUDE),
        Some(entries(&[("plan", "Plan")]).as_slice())
    );
    assert_eq!(
        cache.known_models_for("assistant", CLAUDE),
        Some(entries(&[("opus", "Opus")]).as_slice())
    );
    assert_eq!(cache.known_models_for("assistant", CODEX), None);
    assert_eq!(
        cache.known_modes_for("assistant", CODEX),
        Some(entries(&[("agent", "Agent")]).as_slice())
    );
}

#[test]
fn schema_v1_is_migrated_without_losing_vocabulary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        agent_vocabulary_path_in(dir.path()),
        format!(
            r#"{{
  "version": 1,
  "agents": {{
"assistant": {{
  "source": {source:?},
  "modes": [{{"id":"plan","name":"Plan"}}],
  "models": [{{"id":"opus","name":"Opus"}}]
}}
  }}
}}"#,
            source = CLAUDE,
        ),
    )
    .unwrap();

    let loaded = AgentVocabularyCache::load_in(dir.path());

    assert_eq!(
        loaded.known_modes_for("assistant", CLAUDE),
        Some(entries(&[("plan", "Plan")]).as_slice())
    );
    assert_eq!(
        loaded.known_models_for("assistant", CLAUDE),
        Some(entries(&[("opus", "Opus")]).as_slice())
    );
    let persisted: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(agent_vocabulary_path_in(dir.path())).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["version"], SCHEMA_VERSION);
    assert_eq!(
        persisted["agents"]["assistant"]["sources"][CLAUDE]["models"][0]["id"],
        "opus"
    );
}
