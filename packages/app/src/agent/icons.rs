//! Agent → icon-asset mapping, shared by every surface that names an agent
//! or an auth domain.
//!
//! Returned strings are `DarudaAssets::load` keys; callers hand them to
//! [`crate::ui::agent_icon`]. A const table rather than a `match` so the test
//! below can walk every entry — a path missing from `assets.rs` renders as
//! nothing at all, with no compile or runtime error to catch it.

use daruda_store::accounts::AccountRecipeId;

const CLAUDE_ICON: &str = "icons/agents/claude-acp.svg";
const CODEX_ICON: &str = "icons/agents/codex-acp.svg";

/// `(agent id, icon asset path)`. An adapter shipping under more than one id
/// lists each id against the same path.
const AGENT_ICONS: &[(&str, &str)] = &[
    ("claude", CLAUDE_ICON),
    ("claude-acp", CLAUDE_ICON),
    ("codex", CODEX_ICON),
    ("codex-acp", CODEX_ICON),
    ("agoragentic-acp", "icons/agents/agoragentic-acp.svg"),
    ("amp-acp", "icons/agents/amp-acp.svg"),
    ("auggie", "icons/agents/auggie.svg"),
    ("autohand", "icons/agents/autohand.svg"),
    ("cline", "icons/agents/cline.svg"),
    ("codebuddy-code", "icons/agents/codebuddy-code.svg"),
    ("cortex-code", "icons/agents/cortex-code.svg"),
    ("corust-agent", "icons/agents/corust-agent.svg"),
    ("crow-cli", "icons/agents/crow-cli.svg"),
    ("cursor", "icons/agents/cursor.svg"),
    ("deepagents", "icons/agents/deepagents.svg"),
    ("devin", "icons/agents/devin.svg"),
    ("dimcode", "icons/agents/dimcode.svg"),
    ("dirac", "icons/agents/dirac.svg"),
    ("factory-droid", "icons/agents/factory-droid.svg"),
    ("fast-agent", "icons/agents/fast-agent.svg"),
    ("gemini", "icons/agents/gemini.svg"),
    ("github-copilot-cli", "icons/agents/github-copilot-cli.svg"),
    ("glm-acp-agent", "icons/agents/glm-acp-agent.svg"),
    ("goose", "icons/agents/goose.svg"),
    ("grok-build", "icons/agents/grok-build.svg"),
    ("harn", "icons/agents/harn.svg"),
    ("junie", "icons/agents/junie.svg"),
    ("kilo", "icons/agents/kilo.svg"),
    ("kimi", "icons/agents/kimi.svg"),
    ("minion-code", "icons/agents/minion-code.svg"),
    ("mistral-vibe", "icons/agents/mistral-vibe.svg"),
    ("nova", "icons/agents/nova.svg"),
    ("opencode", "icons/agents/opencode.svg"),
    ("pi-acp", "icons/agents/pi-acp.svg"),
    ("poolside", "icons/agents/poolside.svg"),
    ("qoder", "icons/agents/qoder.svg"),
    ("qwen-code", "icons/agents/qwen-code.svg"),
    ("sigit", "icons/agents/sigit.svg"),
    ("stakpak", "icons/agents/stakpak.svg"),
    ("vtcode", "icons/agents/vtcode.svg"),
];

/// Icon for a configured agent, or `None` when the catalog doesn't know it —
/// the caller falls back to a generic mark rather than dropping the slot.
pub fn icon_for_agent(agent_id: &str) -> Option<&'static str> {
    AGENT_ICONS
        .iter()
        .find(|(id, _)| *id == agent_id)
        .map(|(_, path)| *path)
}

/// Icon for an auth domain, named after the CLI whose credentials it holds.
/// Total (unlike [`icon_for_agent`]): every domain is one daruda ships with.
pub fn icon_for_recipe(recipe: AccountRecipeId) -> &'static str {
    match recipe {
        AccountRecipeId::Claude => CLAUDE_ICON,
        AccountRecipeId::Codex => CODEX_ICON,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource as _;

    /// A path the catalog names but `assets.rs` never registered would paint
    /// an empty box with no error anywhere — the one failure mode of this
    /// module that reading it can't reveal.
    #[test]
    fn every_catalog_path_is_embedded_in_the_binary() {
        for (id, path) in AGENT_ICONS {
            let bytes = crate::assets::DarudaAssets
                .load(path)
                .unwrap_or_else(|e| panic!("{id}: loading {path} errored: {e}"))
                .unwrap_or_else(|| panic!("{id}: {path} is not registered in assets.rs"));
            // Not `starts_with`: some assets open with an XML declaration.
            assert!(
                bytes.windows(4).any(|w| w == b"<svg"),
                "{id}: {path} resolved to something that is not an SVG"
            );
        }
    }

    /// Guards the test above from going toothless: `DarudaAssets` falls
    /// through to gpui_component's own icon set, so an unregistered path must
    /// actually come back empty for the walk to prove anything.
    #[test]
    fn an_unregistered_icon_path_does_not_load() {
        let loaded = crate::assets::DarudaAssets
            .load("icons/agents/not-a-real-agent.svg")
            .ok()
            .flatten();
        assert!(loaded.is_none(), "asset fallback resolves unknown paths");
    }

    #[test]
    fn every_auth_domain_has_an_icon_in_the_catalog() {
        for recipe in AccountRecipeId::all() {
            let path = icon_for_recipe(recipe);
            assert!(
                AGENT_ICONS.iter().any(|(_, p)| *p == path),
                "{recipe:?} points at {path}, which no agent id lists"
            );
        }
    }

    #[test]
    fn an_adapter_under_two_ids_resolves_to_one_icon() {
        assert_eq!(icon_for_agent("claude"), icon_for_agent("claude-acp"));
        assert_eq!(icon_for_agent("codex"), icon_for_agent("codex-acp"));
    }

    #[test]
    fn an_unknown_agent_has_no_icon() {
        assert_eq!(icon_for_agent("not-a-shipped-agent"), None);
    }
}
