//! Public API tests for agent vocabulary.

use daruda_config::agent::vocabulary::*;
use daruda_store::agent_vocabulary::VocabEntry;

#[test]
fn a_claude_command_seeds_both_axes() {
    let seed = seed_for_command("npx -y @agentclientprotocol/claude-agent-acp@latest")
        .expect("the Claude adapter marker is recognized");
    assert_eq!(
        ids(&seed.modes),
        vec![
            "auto",
            "default",
            "acceptEdits",
            "plan",
            "dontAsk",
            "bypassPermissions"
        ]
    );
    assert_eq!(
        ids(&seed.models),
        vec!["default", "opus", "sonnet", "opusplan"]
    );
    assert_eq!(seed.default_mode.as_deref(), Some("default"));
    assert_eq!(seed.default_model.as_deref(), Some("default"));
}

#[test]
fn a_codex_command_seeds_modes_but_no_models() {
    let seed = seed_for_command("npx -y @agentclientprotocol/codex-acp@1.1.0")
        .expect("the Codex adapter marker is recognized");
    assert_eq!(
        ids(&seed.modes),
        vec!["read-only", "agent", "agent-full-access"]
    );
    assert!(
        seed.models.is_empty(),
        "Codex resolves its model list from its backend — daruda has no id to state"
    );
    assert_eq!(seed.default_mode.as_deref(), Some("agent"));
    assert_eq!(seed.default_model, None);
}

#[test]
fn an_unknown_adapter_has_no_seed() {
    assert!(seed_for_command("npx -y @google/gemini-cli@latest --acp").is_none());
    assert!(seed_for_command(r#"{"command":"my-agent"}"#).is_none());
    assert!(seed_for_command("").is_none());
}

/// The default the picker labels must be one of the ids it also offers,
/// or the "agent default" entry would name a mode nothing can select.
#[test]
fn every_seeded_default_is_listed_in_its_own_axis() {
    for command in [
        "npx -y @agentclientprotocol/claude-agent-acp@latest",
        "npx -y @agentclientprotocol/codex-acp@latest",
    ] {
        let seed = seed_for_command(command).expect("a known adapter");
        if let Some(mode) = seed.default_mode.as_deref() {
            assert!(ids(&seed.modes).contains(&mode), "{command}: {mode}");
        }
        if let Some(model) = seed.default_model.as_deref() {
            assert!(ids(&seed.models).contains(&model), "{command}: {model}");
        }
    }
}

fn ids(entries: &[VocabEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.id.as_str()).collect()
}
