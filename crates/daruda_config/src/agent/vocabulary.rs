//! Build-time mode / model vocabularies for the ACP adapters daruda ships
//! facts about — what a picker offers for an agent that has never connected,
//! before [`daruda_store::agent_vocabulary`] has recorded anything real.
//!
//! Keyed on adapter identity via [`account_recipe_for_local_command`] rather
//! than agent id, so a hand-written catalog entry launching the same adapter
//! gets the same seed.

use daruda_store::accounts::AccountRecipeId;
use daruda_store::agent_vocabulary::VocabEntry;

use super::account_recipe_for_local_command;

/// One adapter's statically-known vocabulary.
pub struct AgentVocabularySeed {
    pub modes: Vec<VocabEntry>,
    pub models: Vec<VocabEntry>,
    /// What the adapter uses when nothing is pinned. Display only —
    /// never sent at connect; both adapters already clamp to it themselves.
    pub default_mode: Option<String>,
    pub default_model: Option<String>,
}

/// The vocabulary of the adapter `command` launches. `None` when the
/// command names no adapter daruda has facts about.
pub fn seed_for_command(command: &str) -> Option<AgentVocabularySeed> {
    match account_recipe_for_local_command(command)? {
        AccountRecipeId::Claude => Some(claude_seed()),
        AccountRecipeId::Codex => Some(codex_seed()),
    }
}

/// `@agentclientprotocol/claude-agent-acp@0.70.0`, `dist/acp-agent.js:5393`
/// (`buildAvailableModes`). `auto` is advertised only for a model reporting
/// `supportsAutoMode` and `bypassPermissions` only off root; both are listed
/// anyway, because an unadvertised candidate is simply skipped at connect.
const CLAUDE_MODES: &[(&str, &str)] = &[
    ("auto", "Auto"),
    ("default", "Manual"),
    ("acceptEdits", "Accept Edits"),
    ("plan", "Plan Mode"),
    ("dontAsk", "Don't Ask"),
    ("bypassPermissions", "Bypass Permissions"),
];

/// The model aliases the same adapter resolves itself (`acp-agent.js:3781`,
/// `:5601`, `:5725`) — the real per-account list only arrives on connect.
/// Names are daruda's own: the adapter takes these as input rather than
/// advertising them, so it states no display name for them.
const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("default", "Default"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("opusplan", "Opus Plan"),
];

/// `resolvePermissionMode(undefined)`, `acp-agent.js:407-410`.
const CLAUDE_DEFAULT_MODE: &str = "default";

/// The `"default"` model id the adapter resolves against the account's plan.
const CLAUDE_DEFAULT_MODEL: &str = "default";

/// `@agentclientprotocol/codex-acp@1.1.0`, `dist/index.js:24849` (`AgentMode`).
const CODEX_MODES: &[(&str, &str)] = &[
    ("read-only", "Read-only"),
    ("agent", "Agent"),
    ("agent-full-access", "Agent (full access)"),
];

/// `DEFAULT_AGENT_MODE`, `index.js:24897`.
const CODEX_DEFAULT_MODE: &str = "agent";

fn claude_seed() -> AgentVocabularySeed {
    AgentVocabularySeed {
        modes: entries(CLAUDE_MODES),
        models: entries(CLAUDE_MODELS),
        default_mode: Some(CLAUDE_DEFAULT_MODE.to_string()),
        default_model: Some(CLAUDE_DEFAULT_MODEL.to_string()),
    }
}

/// Codex seeds no models: its list comes from a backend `listModels` call
/// (`index.js:25757`, `createModelConfigOption`), so there is no id daruda
/// can state at build time — and none it may invent.
fn codex_seed() -> AgentVocabularySeed {
    AgentVocabularySeed {
        modes: entries(CODEX_MODES),
        models: Vec::new(),
        default_mode: Some(CODEX_DEFAULT_MODE.to_string()),
        default_model: None,
    }
}

fn entries(table: &[(&str, &str)]) -> Vec<VocabEntry> {
    table
        .iter()
        .map(|(id, name)| VocabEntry::new(*id, *name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(entries: &[VocabEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.id.as_str()).collect()
    }

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

    /// The seed is resolved from adapter markers, not from an agent id, so a
    /// hand-written entry running the same adapter gets the same vocabulary.
    #[test]
    fn the_seed_follows_the_adapter_marker_not_the_agent_id() {
        let pinned = seed_for_command("env FOO=1 npx @agentclientprotocol/claude-agent-acp@0.70.0")
            .expect("a version pin and an env prefix still resolve");
        assert_eq!(ids(&pinned.modes), ids(&claude_seed().modes));
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
}
